//! LeanFlip's file snapshots and cooperative writer locks.
//! Plans contain immutable before/after bytes. Publication is owned by the
//! transaction journal, which can compare these snapshots with a fresh read.
use super::patch::{LivePatch, LiveWriteError};
use crate::{
    fsutil::{atomic_write_private, resolve_write_path, stage_write, StagedWrite},
    AppError,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

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
        self.lock_app_timeout(app, LOCK_TIMEOUT)
    }
    fn lock_app_timeout(&self, app: &str, timeout: Duration) -> Result<AppWriteGuard, AppError> {
        if app.is_empty() || app == "." || app == ".." || app.contains(['/', '\\']) {
            return Err(AppError::InvalidInput("Invalid writer lock name".into()));
        }
        let path = self.file("locks").join(format!("{app}.lock"));
        let deadline = Instant::now() + timeout;
        let mut guard = acquire(path.to_string_lossy().into_owned(), app, deadline)?;
        guard.file = Some(lock_file_until(&path, true, deadline)?);
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
fn acquire(key: String, label: &str, deadline: Instant) -> Result<AppWriteGuard, AppError> {
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
        registry.insert(key.clone(), Arc::downgrade(&gate));
        gate
    };
    let mut occupied = gate.occupied.lock().unwrap_or_else(|e| e.into_inner());
    while *occupied {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(lock_busy(Path::new(&key), Some(std::process::id())));
        }
        occupied = gate
            .ready
            .wait_timeout(occupied, remaining)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
    *occupied = true;
    drop(occupied);
    Ok(AppWriteGuard {
        label: label.into(),
        gate,
        file: None,
    })
}
pub fn lock_app(app: &str) -> Result<AppWriteGuard, AppError> {
    acquire(format!("local:{app}"), app, Instant::now() + LOCK_TIMEOUT)
}

pub(crate) fn lock_file(path: &Path, exclusive: bool) -> Result<File, AppError> {
    lock_file_until(path, exclusive, Instant::now() + LOCK_TIMEOUT)
}
fn lock_busy(path: &Path, pid: Option<u32>) -> AppError {
    let owner = pid
        .map(|pid| format!("; last recorded PID: {pid}"))
        .unwrap_or_default();
    AppError::localized(
        "lock.busy",
        format!("等待锁超时：{}{owner}。请等待其他操作完成，或检查持锁进程后重试。", path.display()),
        format!("Timed out waiting for lock: {}{owner}. Wait for the other operation or check the holder process and retry.", path.display()),
    )
}
fn lock_file_until(path: &Path, exclusive: bool, deadline: Instant) -> Result<File, AppError> {
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
    let mut file = options.open(path).map_err(|e| AppError::io(path, e))?;
    loop {
        match if exclusive {
            file.try_lock()
        } else {
            file.try_lock_shared()
        } {
            Ok(()) => break,
            Err(TryLockError::Error(error)) => return Err(AppError::io(path, error)),
            Err(TryLockError::WouldBlock) => {}
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let mut owner = String::new();
            let _ = File::open(path).and_then(|file| file.take(128).read_to_string(&mut owner));
            let pid = owner
                .split_whitespace()
                .next()
                .and_then(|pid| pid.parse().ok());
            return Err(lock_busy(path, pid));
        }
        std::thread::sleep(remaining.min(Duration::from_millis(25)));
    }
    if exclusive {
        let acquired_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        file.set_len(0)
            .and_then(|()| writeln!(file, "{} {acquired_at}", std::process::id()))
            .map_err(|e| AppError::io(path, e))?;
    }
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
            atomic_write_private(&directory.join(&stem), bytes)?;
        }
        let resolved = resolve_write_path(path)?;
        if resolved != path {
            atomic_write_private(
                &directory.join(format!("{stem}.resolved")),
                resolved.to_string_lossy().as_bytes(),
            )?;
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
    fn thread_gate_and_os_lock_waits_are_bounded_and_release_after_timeout() {
        let root = tempfile::tempdir().unwrap();
        let store = DeviceStore::at(root.path());
        let first = store.lock_app("claude").unwrap();
        let started = Instant::now();
        let error = store
            .lock_app_timeout("claude", Duration::from_millis(30))
            .unwrap_err();
        assert!(matches!(
            error,
            AppError::Localized {
                key: "lock.busy",
                ..
            }
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(first);
        store.lock_app("claude").unwrap();

        let path = store.file("external.lock");
        let first = lock_file(&path, true).unwrap();
        for exclusive in [true, false] {
            let error =
                lock_file_until(&path, exclusive, Instant::now() + Duration::from_millis(30))
                    .unwrap_err();
            // Windows locks prevent reading the owner's PID through a second
            // handle. The diagnostic includes it only when the OS allows it.
            #[cfg(unix)]
            assert!(error.to_string().contains(&std::process::id().to_string()));
            assert!(matches!(
                error,
                AppError::Localized {
                    key: "lock.busy",
                    ..
                }
            ));
        }
        drop(first);
        lock_file(&path, true).unwrap();
    }
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
            1 + usize::from(resolve_write_path(&target).unwrap() != target)
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
