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
}
impl StagedWrite {
    pub fn tmp_path(&self) -> &Path {
        &self.prepared
    }
    pub fn commit(self) -> Result<(), AppError> {
        let result = commit_staged(&self.prepared, &self.destination);
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
    })
}

pub fn commit_staged(tmp: &Path, path: &Path) -> Result<(), AppError> {
    fs::rename(tmp, path).map_err(|e| AppError::io(path, e))
}

pub fn atomic_write_private(path: &Path, data: &[u8]) -> Result<(), AppError> {
    stage_write(path, data, Some(0o600), true)?.commit()
}
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), AppError> {
    stage_write(path, data, None, true)?.commit()
}
pub fn delete_file(path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
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
