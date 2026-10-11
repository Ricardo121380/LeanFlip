//! Recoverable multi-file updates for LeanFlip.
//!
//! A journal entry is durable before the first client file is replaced. Once
//! publication starts, recovery completes the remaining prepared values while
//! preserving files edited by another application. The provider pointer is an
//! idempotent finalization step, never an optimistic UI-side update.
use crate::{
    fsutil,
    live::{
        engine::{self, AppWriteGuard, DeviceStore, LiveFile, Planned},
        patch::{LivePatch, LiveWriteError},
    },
    state::{self, Pending, PendingFile, PendingTarget},
    AppError,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct FileChange<'a> {
    pub file: LiveFile,
    pub patch: &'a dyn LivePatch,
}
pub type CommitTarget<'a> = &'a dyn Fn(&PendingTarget) -> Result<(), AppError>;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    Discarded,
    Abandoned { paths: Vec<PathBuf> },
    RolledForward,
    RolledForwardExcept { paths: Vec<PathBuf> },
}
#[derive(Debug, Default)]
pub struct OperationReport {
    pub changed: Vec<PathBuf>,
    pub recovered: Option<RecoveryOutcome>,
    pub warnings: Vec<String>,
}

fn fingerprint(path: &Path) -> Result<Option<String>, AppError> {
    Ok(engine::digest(engine::read_current(path)?.as_deref()))
}
fn prepare(plan: &Planned) -> Result<PendingFile, AppError> {
    Ok(PendingFile {
        path: plan.file.path.clone(),
        pre: plan.pre.clone(),
        planned: plan.planned.clone(),
        staged: if plan.is_noop() {
            None
        } else {
            engine::stage(plan)?.map(|file| file.tmp_path().into())
        },
    })
}
fn dispose(files: &[PendingFile]) {
    for path in files.iter().filter_map(|entry| entry.staged.as_ref()) {
        let _ = fs::remove_file(path);
    }
}
fn publish(file: &PendingFile) -> Result<(), AppError> {
    match (&file.planned, &file.staged) {
        (None, _) => fsutil::delete_file(&file.path),
        (Some(_), Some(source)) => fsutil::commit_staged(source, &file.path),
        (Some(_), None) => Err(AppError::Config(
            "A prepared file is missing from the write journal".into(),
        )),
    }
}

struct Journal<'a> {
    store: &'a DeviceStore,
    app: &'a str,
    intent: Pending,
    changed: Vec<PathBuf>,
}
impl Journal<'_> {
    fn save(&self) -> Result<(), AppError> {
        state::set_pending(self.store, self.app, Some(self.intent.clone()))
    }
    fn cancel(&self) -> Result<(), AppError> {
        // If clearing the journal fails, retain its staged files for recovery.
        state::set_pending(self.store, self.app, None)?;
        dispose(&self.intent.files);
        Ok(())
    }
    fn fail(&self, error: AppError) -> AppError {
        if self.changed.is_empty() {
            if let Err(cleanup) = self.cancel() {
                return cleanup;
            }
        }
        error
    }
    fn replace_plan(&mut self, index: usize, next: &Planned) -> Result<(), AppError> {
        let prepared = prepare(next)?;
        let previous = std::mem::replace(&mut self.intent.files[index], prepared);
        if let Err(error) = self.save() {
            let unrecorded = std::mem::replace(&mut self.intent.files[index], previous);
            dispose(&[unrecorded]);
            return Err(error);
        }
        dispose(&[previous]);
        Ok(())
    }
    fn step(
        &mut self,
        index: usize,
        mut plan: Planned,
        patch: &dyn LivePatch,
    ) -> Result<(), AppError> {
        for attempt in 0..4 {
            failpoint::before_publish(index, &plan.file.path);
            let current = engine::read_current(&plan.file.path)?;
            if engine::digest(current.as_deref()) != plan.pre {
                if attempt == 3 {
                    return Err(self.fail(
                        LiveWriteError::Conflict {
                            path: plan.file.path,
                        }
                        .into(),
                    ));
                }
                let replacement = engine::plan_from(&plan.file, patch, current)
                    .map_err(|error| self.fail(error.into()))?;
                self.replace_plan(index, &replacement)?;
                if replacement.is_noop() {
                    return Ok(());
                }
                plan = replacement;
                continue;
            }
            engine::ensure_first_write_backup(self.store, &plan.file.path, current.as_deref())?;
            if !self.intent.published {
                failpoint::hit("mark").map_err(|e| self.fail(e))?;
                self.intent.published = true;
                self.save().map_err(|e| self.fail(e))?;
                failpoint::hit("marked")?;
            }
            publish(&self.intent.files[index]).map_err(|e| self.fail(e))?;
            self.changed.push(plan.file.path);
            return Ok(());
        }
        unreachable!("every attempt either publishes, replans, or returns a conflict")
    }
}

pub fn run(
    store: &DeviceStore,
    guard: &AppWriteGuard,
    op: &str,
    changes: &[FileChange<'_>],
    target: PendingTarget,
    commit: CommitTarget<'_>,
) -> Result<OperationReport, AppError> {
    let recovered = recover(store, guard, commit)?;
    let plans = changes
        .iter()
        .map(|change| engine::plan(&change.file, change.patch).map(|plan| (plan, change.patch)))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|(plan, _)| !plan.is_noop())
        .collect::<Vec<_>>();
    if plans.is_empty() {
        commit(&target)?;
        return Ok(OperationReport {
            recovered,
            ..OperationReport::default()
        });
    }
    let mut journal = Journal {
        store,
        app: guard.app(),
        intent: Pending {
            op: op.into(),
            files: Vec::new(),
            target,
            published: false,
        },
        changed: Vec::new(),
    };
    for (plan, _) in &plans {
        match prepare(plan) {
            Ok(file) => journal.intent.files.push(file),
            Err(error) => {
                dispose(&journal.intent.files);
                return Err(error);
            }
        }
    }
    if let Err(error) = failpoint::hit("staged").and_then(|()| journal.save()) {
        dispose(&journal.intent.files);
        return Err(error);
    }
    failpoint::hit("pending")?;
    for (index, (plan, patch)) in plans.into_iter().enumerate() {
        journal.step(index, plan, patch)?;
        failpoint::hit(&format!("published:{index}"))?;
    }
    commit(&journal.intent.target)?;
    failpoint::hit("target")?;
    journal.cancel()?;
    Ok(OperationReport {
        recovered,
        changed: journal.changed,
        warnings: Vec::new(),
    })
}

pub fn recover(
    store: &DeviceStore,
    guard: &AppWriteGuard,
    commit: CommitTarget<'_>,
) -> Result<Option<RecoveryOutcome>, AppError> {
    let Some(intent) = state::pending(store, guard.app())? else {
        return Ok(None);
    };
    let current = intent
        .files
        .iter()
        .map(|file| fingerprint(&file.path))
        .collect::<Result<Vec<_>, _>>()?;
    let started = intent.published
        || intent
            .files
            .iter()
            .zip(&current)
            .any(|(file, hash)| file.pre != file.planned && hash == &file.planned);
    let divergent = intent
        .files
        .iter()
        .zip(&current)
        .filter(|(file, hash)| **hash != file.pre && **hash != file.planned)
        .map(|(file, _)| file.path.clone())
        .collect::<Vec<_>>();
    if !started {
        state::set_pending(store, guard.app(), None)?;
        dispose(&intent.files);
        return Ok(Some(if divergent.is_empty() {
            RecoveryOutcome::Discarded
        } else {
            RecoveryOutcome::Abandoned { paths: divergent }
        }));
    }
    let mut skipped = Vec::new();
    for file in &intent.files {
        // Read again here: a client can have edited a later file while an earlier
        // one was being recovered. The earlier classification is not a write permit.
        let bytes = engine::read_current(&file.path)?;
        let hash = engine::digest(bytes.as_deref());
        if hash == file.planned {
            continue;
        }
        let prepared = match (&file.planned, &file.staged) {
            (None, _) => true,
            (Some(_), Some(path)) => fingerprint(path)? == file.planned,
            (Some(_), None) => false,
        };
        if hash != file.pre || !prepared {
            skipped.push(file.path.clone());
            continue;
        }
        engine::ensure_first_write_backup(store, &file.path, bytes.as_deref())?;
        // Backing up can involve IO; guard against a change during that work too.
        if fingerprint(&file.path)? != hash {
            skipped.push(file.path.clone());
            continue;
        }
        publish(file)?;
    }
    failpoint::hit("recover:target")?;
    commit(&intent.target)?;
    state::set_pending(store, guard.app(), None)?;
    dispose(&intent.files);
    Ok(Some(if skipped.is_empty() {
        RecoveryOutcome::RolledForward
    } else {
        RecoveryOutcome::RolledForwardExcept { paths: skipped }
    }))
}

pub fn pending_published(store: &DeviceStore, app: &str) -> Result<Option<bool>, AppError> {
    Ok(state::pending(store, app)?.map(|intent| intent.published))
}
pub fn settle(
    store: &DeviceStore,
    app: &str,
    commit: CommitTarget<'_>,
) -> Result<Option<RecoveryOutcome>, AppError> {
    recover(store, &store.lock_app(app)?, commit)
}
pub fn recover_before_write(
    store: &DeviceStore,
    guard: &AppWriteGuard,
    commit: CommitTarget<'_>,
) -> Result<(), AppError> {
    match recover(store, guard, commit)? {
        None | Some(RecoveryOutcome::Discarded | RecoveryOutcome::Abandoned { .. }) => Ok(()),
        Some(RecoveryOutcome::RolledForwardExcept { paths }) => {
            let files = paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            Err(AppError::localized("live.recovered_before_write", format!("上次操作仅部分恢复；请检查这些文件，再重试：\n{files}"), format!("The previous operation was partially recovered. Inspect these files before retrying:\n{files}")))
        }
        Some(RecoveryOutcome::RolledForward) => Err(AppError::localized(
            "live.recovered_before_write",
            "已完成上次中断的操作；请基于最新配置重试。",
            "The interrupted operation is complete. Retry using the current configuration.",
        )),
    }
}
pub struct AppWrite<'a> {
    pub store: DeviceStore,
    pub guard: AppWriteGuard,
    commit: CommitTarget<'a>,
}
impl<'a> AppWrite<'a> {
    pub fn begin(
        store: DeviceStore,
        app: &str,
        commit: CommitTarget<'a>,
    ) -> Result<Self, AppError> {
        let guard = store.lock_app(app)?;
        recover_before_write(&store, &guard, commit)?;
        Ok(Self {
            store,
            guard,
            commit,
        })
    }
    pub fn run(
        &self,
        op: &str,
        changes: &[FileChange<'_>],
        target: PendingTarget,
    ) -> Result<OperationReport, AppError> {
        run(&self.store, &self.guard, op, changes, target, self.commit)
    }
}
pub fn recover_all(
    store: &DeviceStore,
    commit: &dyn Fn(&str, &PendingTarget) -> Result<(), AppError>,
) -> Vec<(String, Result<RecoveryOutcome, AppError>)> {
    let apps = match state::apps_with_pending(store) {
        Ok(apps) => apps,
        Err(error) => return vec![("*".into(), Err(error))],
    };
    apps.into_iter()
        .filter_map(
            |app| match settle(store, &app, &|target| commit(&app, target)) {
                Ok(None) => None,
                Ok(Some(outcome)) => Some((app, Ok(outcome))),
                Err(error) => Some((app, Err(error))),
            },
        )
        .collect()
}
pub fn commit_device_state(
    store: &DeviceStore,
    app: &str,
    target: &PendingTarget,
) -> Result<(), AppError> {
    if let Some(written) = &target.written {
        state::update(store, |data| {
            data.apps.entry(app.into()).or_default().written = Some(written.clone())
        })?;
    }
    Ok(())
}

// Deterministic crash/race injection is confined to test builds.
pub(crate) mod failpoint {
    use crate::AppError;
    use std::path::Path;
    #[cfg(test)]
    type Callback = Box<dyn FnMut(usize, &Path)>;
    #[cfg(test)]
    #[derive(Default)]
    struct Scenario {
        stop: Option<String>,
        callback: Option<Callback>,
    }
    #[cfg(test)]
    std::thread_local! { static SCENARIO: std::cell::RefCell<Scenario> = std::cell::RefCell::new(Scenario::default()); }
    #[cfg(test)]
    pub fn crash_at(point: Option<&str>) {
        SCENARIO.with_borrow_mut(|s| s.stop = point.map(str::to_owned));
    }
    #[cfg(test)]
    pub fn on_before_publish(callback: Option<Callback>) {
        SCENARIO.with_borrow_mut(|s| s.callback = callback);
    }
    pub fn hit(point: &str) -> Result<(), AppError> {
        #[cfg(test)]
        if SCENARIO.with_borrow(|s| s.stop.as_deref() == Some(point)) {
            return Err(AppError::Message(format!(
                "Simulated interruption: {point}"
            )));
        }
        let _ = point;
        Ok(())
    }
    pub fn before_publish(index: usize, path: &Path) {
        #[cfg(test)]
        SCENARIO.with_borrow_mut(|s| {
            if let Some(callback) = &mut s.callback {
                callback(index, path);
            }
        });
        let _ = (index, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::patch::WholeFile;
    use std::cell::Cell;
    #[test]
    fn interruptions_before_and_after_publication_have_distinct_recovery() {
        for stop in ["pending", "marked", "published:0", "published:1", "target"] {
            let dir = tempfile::tempdir().unwrap();
            let store = DeviceStore::at(dir.path().join("private"));
            let paths = [dir.path().join("first"), dir.path().join("second")];
            for path in &paths {
                fs::write(path, b"before").unwrap();
            }
            let replacement = WholeFile::Write(b"after".to_vec());
            let changes = paths
                .iter()
                .map(|path| FileChange {
                    file: LiveFile::private(path),
                    patch: &replacement as &dyn LivePatch,
                })
                .collect::<Vec<_>>();
            let guard = store.lock_app("fixture").unwrap();
            let finalized = Cell::new(false);
            let commit = |_: &PendingTarget| {
                finalized.set(true);
                Ok(())
            };
            failpoint::crash_at(Some(stop));
            let result = run(
                &store,
                &guard,
                "switch",
                &changes,
                PendingTarget::pointer(Some("destination".into())),
                &commit,
            );
            failpoint::crash_at(None);
            assert!(result.is_err(), "{stop}");
            let pending = state::pending(&store, "fixture").unwrap().unwrap();
            if stop == "pending" {
                assert!(!pending.published);
            }
            let outcome = recover(&store, &guard, &commit).unwrap().unwrap();
            let expected: &[u8] = if stop == "pending" {
                b"before"
            } else {
                b"after"
            };
            for path in &paths {
                assert_eq!(fs::read(path).unwrap(), expected);
            }
            assert_eq!(outcome == RecoveryOutcome::Discarded, stop == "pending");
            assert_eq!(finalized.get(), stop != "pending");
            assert!(state::pending(&store, "fixture").unwrap().is_none());
            for file in pending.files {
                if let Some(path) = file.staged {
                    assert!(!path.exists());
                }
            }
        }
    }
    #[test]
    fn recovery_keeps_external_edits_and_reports_missing_staging() {
        for external_edit in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = DeviceStore::at(dir.path().join("private"));
            let a = dir.path().join("a");
            let b = dir.path().join("b");
            fs::write(&a, b"one").unwrap();
            fs::write(&b, b"two").unwrap();
            let patch = WholeFile::Write(b"replacement".to_vec());
            let guard = store.lock_app("fixture").unwrap();
            failpoint::crash_at(Some("published:0"));
            let result = run(
                &store,
                &guard,
                "switch",
                &[
                    FileChange {
                        file: LiveFile::private(&a),
                        patch: &patch,
                    },
                    FileChange {
                        file: LiveFile::private(&b),
                        patch: &patch,
                    },
                ],
                PendingTarget::default(),
                &|_| Ok(()),
            );
            failpoint::crash_at(None);
            assert!(result.is_err());
            if external_edit {
                fs::write(&b, b"external").unwrap();
            } else {
                fs::remove_file(
                    state::pending(&store, "fixture").unwrap().unwrap().files[1]
                        .staged
                        .as_ref()
                        .unwrap(),
                )
                .unwrap();
            }
            assert_eq!(
                recover(&store, &guard, &|_| Ok(())).unwrap(),
                Some(RecoveryOutcome::RolledForwardExcept {
                    paths: vec![b.clone()]
                })
            );
            assert_eq!(
                fs::read(&b).unwrap(),
                if external_edit {
                    b"external".as_slice()
                } else {
                    b"two".as_slice()
                }
            );
        }
    }
    #[test]
    fn a_rejected_plan_cannot_publish_an_earlier_valid_file() {
        let root = tempfile::tempdir().unwrap();
        let store = DeviceStore::at(root.path().join("journal"));
        let a = root.path().join("a");
        let b = root.path().join("b");
        fs::write(&a, b"unchanged").unwrap();
        fs::write(&b, b"client-updated").unwrap();
        let replacement = WholeFile::Write(b"updated".to_vec());
        let stale = crate::live::patch::Guarded {
            expected_pre: None,
            then: replacement.clone(),
        };
        let guard = store.lock_app("fixture").unwrap();
        assert!(run(
            &store,
            &guard,
            "apply",
            &[
                FileChange {
                    file: LiveFile::private(&a),
                    patch: &replacement
                },
                FileChange {
                    file: LiveFile::private(&b),
                    patch: &stale
                },
            ],
            PendingTarget::default(),
            &|_| Ok(())
        )
        .is_err());
        assert_eq!(fs::read(&a).unwrap(), b"unchanged");
        assert!(state::pending(&store, "fixture").unwrap().is_none());
    }
}
