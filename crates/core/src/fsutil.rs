//! File replacement primitives used by LeanFlip's stores and write journal.
//! Temporary files are allocated by `tempfile` beside their destination, so
//! publishing a prepared value is a same-filesystem rename.
use crate::AppError;
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct StagedWrite {
    destination: PathBuf,
    prepared: PathBuf,
    durable: bool,
}
impl StagedWrite {
    pub fn tmp_path(&self) -> &Path {
        &self.prepared
    }
    pub fn destination(&self) -> &Path {
        &self.destination
    }
    pub fn commit(self) -> Result<(), AppError> {
        let result = publish(&self.prepared, &self.destination, self.durable);
        if result.is_err() {
            let _ = fs::remove_file(&self.prepared);
        }
        result
    }
}

pub fn stage_write(
    path: &Path,
    data: &[u8],
    unix_mode: Option<u32>,
    durable: bool,
) -> Result<StagedWrite, AppError> {
    let destination = resolve_write_path(path)?;
    let path = destination.as_path();
    let directory = path
        .parent()
        .filter(|_| path.file_name().is_some())
        .ok_or_else(|| AppError::InvalidInput("A destination file is required".into()))?;
    fs::create_dir_all(directory).map_err(|e| AppError::io(directory, e))?;
    let mut candidate = tempfile::Builder::new()
        .prefix(".leanflip-write-")
        .tempfile_in(directory)
        .map_err(|e| AppError::io(directory, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = unix_mode
            .map(fs::Permissions::from_mode)
            .or_else(|| fs::metadata(path).ok().map(|m| m.permissions()));
        if let Some(permissions) = permissions {
            candidate
                .as_file()
                .set_permissions(permissions)
                .map_err(|e| AppError::io(path, e))?;
        }
    }
    #[cfg(not(unix))]
    let _ = unix_mode;
    candidate
        .write_all(data)
        .map_err(|e| AppError::io(path, e))?;
    if durable {
        candidate
            .as_file()
            .sync_all()
            .map_err(|e| AppError::io(path, e))?;
    }
    // The journal, rather than an in-memory destructor, owns persisted staging
    // files. They must survive process termination for recovery to complete.
    let (_, prepared) = candidate.keep().map_err(|e| AppError::io(path, e.error))?;
    Ok(StagedWrite {
        destination: path.into(),
        prepared,
        durable,
    })
}

/// Resolve file and directory links, including a link to a not-yet-created file.
/// Keep the resolved destination stable for the lifetime of a staged write.
pub fn resolve_write_path(path: &Path) -> Result<PathBuf, AppError> {
    fn resolve(path: &Path, links: usize) -> Result<PathBuf, AppError> {
        if path.as_os_str().is_empty() {
            return resolve(Path::new("."), links);
        }
        match fs::canonicalize(path) {
            Ok(path) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AppError::io(path, error)),
        }
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            if links >= 40 {
                return Err(AppError::localized(
                    "live.symlink_target",
                    "配置文件的符号链接无法解析，请检查链接目标。",
                    "Cannot resolve the config symlink; check its target.",
                ));
            }
            let target = fs::read_link(path).map_err(|e| AppError::io(path, e))?;
            return resolve(
                &if target.is_absolute() {
                    target
                } else {
                    path.parent().unwrap_or(Path::new(".")).join(target)
                },
                links + 1,
            );
        }
        let name = path
            .file_name()
            .ok_or_else(|| AppError::InvalidInput("A destination file is required".into()))?;
        Ok(resolve(path.parent().unwrap_or(Path::new(".")), links)?.join(name))
    }
    resolve(path, 0)
}

pub fn sync_parent(path: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|e| AppError::io(parent, e))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
fn publish(tmp: &Path, path: &Path, durable: bool) -> Result<(), AppError> {
    fs::rename(tmp, path).map_err(|e| AppError::io(path, e))?;
    if durable {
        sync_parent(path)?;
    }
    Ok(())
}
pub fn commit_staged(tmp: &Path, path: &Path) -> Result<(), AppError> {
    publish(tmp, &resolve_write_path(path)?, true)
}

pub fn atomic_write_private(path: &Path, data: &[u8]) -> Result<(), AppError> {
    stage_write(path, data, Some(0o600), true)?.commit()
}
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), AppError> {
    stage_write(path, data, None, true)?.commit()
}
pub fn delete_file(path: &Path) -> Result<(), AppError> {
    let destination = resolve_write_path(path)?;
    match fs::remove_file(&destination) {
        Ok(()) => sync_parent(&destination),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(AppError::io(path, e)),
        _ => Ok(()),
    }
}
pub fn sorted_json_bytes<T: Serialize>(data: &T) -> Result<Vec<u8>, AppError> {
    let mut value =
        serde_json::to_value(data).map_err(|source| AppError::JsonSerialize { source })?;
    value.sort_all_objects();
    serde_json::to_vec_pretty(&value).map_err(|source| AppError::JsonSerialize { source })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn writes_follow_file_links_and_can_recreate_a_deleted_target() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("dotfiles/settings.json");
        atomic_write(&target, b"before").unwrap();
        let link = root.path().join("settings.json");
        symlink("dotfiles/settings.json", &link).unwrap();
        let staged = stage_write(&link, b"after", Some(0o600), true).unwrap();
        assert_eq!(staged.destination(), fs::canonicalize(&target).unwrap());
        assert_eq!(staged.tmp_path().parent(), staged.destination().parent());
        staged.commit().unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&target).unwrap(), b"after");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        delete_file(&link).unwrap();
        assert!(!target.exists());
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        atomic_write_private(&link, b"restored").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"restored");
    }

    #[cfg(unix)]
    #[test]
    fn staging_pins_the_destination_when_a_link_is_retargeted() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("old");
        let new = root.path().join("new");
        fs::write(&old, b"old").unwrap();
        fs::write(&new, b"new").unwrap();
        let link = root.path().join("link");
        symlink(&old, &link).unwrap();
        let staged = stage_write(&link, b"prepared", None, false).unwrap();
        fs::remove_file(&link).unwrap();
        symlink(&new, &link).unwrap();
        staged.commit().unwrap();
        assert_eq!(fs::read(&old).unwrap(), b"prepared");
        assert_eq!(fs::read(&new).unwrap(), b"new");
        assert_eq!(fs::read_link(&link).unwrap(), new);
    }
    #[test]
    fn staging_does_not_publish_and_can_be_recovered_without_its_owner() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested/settings");
        atomic_write_private(&path, b"initial").unwrap();
        let prepared = stage_write(&path, b"replacement", Some(0o600), true).unwrap();
        let source = prepared.tmp_path().to_owned();
        drop(prepared);
        assert_eq!(fs::read(&path).unwrap(), b"initial");
        commit_staged(&source, &path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(!source.exists());
    }
    #[test]
    fn nested_json_is_deterministic_and_missing_deletion_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        delete_file(&root.path().join("absent")).unwrap();
        let first = serde_json::json!({"z":[{"b":2,"a":1}],"a":false});
        let second = serde_json::json!({"a":false,"z":[{"a":1,"b":2}]});
        assert_eq!(
            sorted_json_bytes(&first).unwrap(),
            sorted_json_bytes(&second).unwrap()
        );
    }
}
