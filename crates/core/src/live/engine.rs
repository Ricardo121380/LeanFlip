//! LeanFlip's file snapshots and cooperative writer locks.
//! Plans contain immutable before/after bytes. Publication is owned by the
//! transaction journal, which can compare these snapshots with a fresh read.
use super::patch::{LivePatch, LiveWriteError};
use crate::{
    fsutil::{atomic_write_private, stage_write, StagedWrite},
    AppError,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStore {
    directory: PathBuf,
}
impl DeviceStore {
    pub fn at(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
    pub fn file(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
    pub fn state_path(&self) -> PathBuf {
        self.file("live-state.json")
    }
    pub fn first_write_backup_dir(&self) -> PathBuf {
        self.file("backups/live-first-write")
    }
    pub fn lock_app(&self, app: &str) -> Result<AppWriteGuard, AppError> {
        if app.is_empty() || app == "." || app == ".." || app.contains(['/', '\\']) {
            return Err(AppError::InvalidInput("Invalid writer lock name".into()));
        }
        let path = self.file("locks").join(format!("{app}.lock"));
        let mut guard = acquire(path.to_string_lossy().into_owned(), app);
        guard.file = Some(lock_file(&path, true)?);
        Ok(guard)
    }
}

#[derive(Debug, Default)]
struct Gate {
    occupied: Mutex<bool>,
    ready: Condvar,
}
#[derive(Debug)]
pub struct AppWriteGuard {
    label: String,
    gate: Arc<Gate>,
    file: Option<File>,
}
impl AppWriteGuard {
    pub fn app(&self) -> &str {
        &self.label
    }
}
impl Drop for AppWriteGuard {
    fn drop(&mut self) {
        // Release the OS handle before allowing another local writer through.
        self.file.take();
        *self.gate.occupied.lock().unwrap_or_else(|e| e.into_inner()) = false;
        self.gate.ready.notify_one();
    }
}
fn acquire(key: String, label: &str) -> AppWriteGuard {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Weak<Gate>>>> = OnceLock::new();
    let gate = {
        let mut registry = REGISTRY
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        registry.retain(|_, value| value.strong_count() != 0);
        let gate = registry
            .get(&key)
            .and_then(Weak::upgrade)
            .unwrap_or_default();
        registry.insert(key, Arc::downgrade(&gate));
        gate
    };
    let mut occupied = gate.occupied.lock().unwrap_or_else(|e| e.into_inner());
    while *occupied {
        occupied = gate.ready.wait(occupied).unwrap_or_else(|e| e.into_inner());
    }
    *occupied = true;
    drop(occupied);
    AppWriteGuard {
        label: label.into(),
        gate,
        file: None,
    }
}
pub fn lock_app(app: &str) -> AppWriteGuard {
    acquire(format!("local:{app}"), app)
}

pub(crate) fn lock_file(path: &Path, exclusive: bool) -> Result<File, AppError> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(|e| AppError::io(directory, e))?;
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|e| AppError::io(path, e))?;
    if exclusive {
        file.lock()
    } else {
        file.lock_shared()
    }
    .map_err(|e| AppError::io(path, e))?;
    Ok(file)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveFile {
    pub path: PathBuf,
    pub private: bool,
}
impl LiveFile {
    pub fn private(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            private: true,
        }
    }
    pub fn shared(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            private: false,
        }
    }
}
#[derive(Debug, Clone)]
pub struct Planned {
    pub file: LiveFile,
    pub pre: Option<String>,
    pub planned: Option<String>,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
}
impl Planned {
    pub fn is_noop(&self) -> bool {
        self.before == self.after
    }
    pub fn pre_bytes(&self) -> Option<&[u8]> {
        self.before.as_deref()
    }
}
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn digest(bytes: Option<&[u8]>) -> Option<String> {
    bytes.map(sha256_hex)
}
pub fn read_current(path: &Path) -> Result<Option<Vec<u8>>, LiveWriteError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(LiveWriteError::Io {
            path: path.into(),
            source,
        }),
    }
}
pub fn plan(file: &LiveFile, patch: &dyn LivePatch) -> Result<Planned, LiveWriteError> {
    plan_from(file, patch, read_current(&file.path)?)
}
pub(crate) fn plan_from(
    file: &LiveFile,
    patch: &dyn LivePatch,
    before: Option<Vec<u8>>,
) -> Result<Planned, LiveWriteError> {
    let after = patch.apply_file(&file.path, before.as_deref())?;
    Ok(Planned {
        file: file.clone(),
        pre: digest(before.as_deref()),
        planned: digest(after.as_deref()),
        before,
        after,
    })
}
pub fn stage(plan: &Planned) -> Result<Option<StagedWrite>, AppError> {
    plan.after
        .as_deref()
        .map(|bytes| {
            stage_write(
                &plan.file.path,
                bytes,
                plan.file.private.then_some(0o600),
                true,
            )
        })
        .transpose()
}
pub fn ensure_first_write_backup(
    store: &DeviceStore,
    path: &Path,
    current: Option<&[u8]>,
) -> Result<(), AppError> {
    let _lock = lock_file(&store.file("locks/first-write-backups.lock"), true)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let fingerprint = sha256_hex(path.to_string_lossy().as_bytes());
    let stem = format!("{}-{name}", &fingerprint[..12]);
    let directory = store.first_write_backup_dir();
    let marker = directory.join(format!("{stem}.source"));
    if !marker.try_exists().map_err(|e| AppError::io(&marker, e))? {
        if let Some(bytes) = current {
            atomic_write_private(&directory.join(stem), bytes)?;
        }
        atomic_write_private(&marker, path.to_string_lossy().as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::patch::WholeFile;
    #[test]
    fn snapshot_and_missing_file_backup_do_not_invent_previous_contents() {
        let root = tempfile::tempdir().unwrap();
        let store = DeviceStore::at(root.path().join("state"));
        let target = root.path().join("client");
        let plan = plan(
            &LiveFile::private(&target),
            &WholeFile::Write(b"new".to_vec()),
        )
        .unwrap();
        assert_eq!(plan.pre, None);
        assert!(!target.exists());
        ensure_first_write_backup(&store, &target, None).unwrap();
        ensure_first_write_backup(&store, &target, Some(b"not-original")).unwrap();
        assert_eq!(
            fs::read_dir(store.first_write_backup_dir())
                .unwrap()
                .count(),
            1
        );
        stage(&plan).unwrap().unwrap().commit().unwrap();
        assert_eq!(read_current(&target).unwrap().unwrap(), b"new");
    }
    #[test]
    fn gates_serialize_threads_but_do_not_block_other_homes() {
        use std::sync::mpsc;
        let root = tempfile::tempdir().unwrap();
        let a = DeviceStore::at(root.path().join("a"));
        let b = DeviceStore::at(root.path().join("b"));
        let first = a.lock_app("claude").unwrap();
        let other_home = b.lock_app("claude").unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _lock = a.lock_app("claude").unwrap();
            tx.send(()).unwrap();
        });
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(30))
            .is_err());
        drop(first);
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        drop(other_home);
    }
}
