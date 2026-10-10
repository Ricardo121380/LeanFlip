//! 原子写入的底层工具（移植自 cc-switch `config.rs` 的 `stage_write` / `commit_staged` 等）。
//!
//! 和原版的差异：Windows 上原版优先用 `ReplaceFileW`（经 `windows-sys`），这里统一用
//! `std::fs::rename`（Windows 上 Rust 用 `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`，
//! 同样是替换）。Phase 2 做 Windows 打包时再评估是否要恢复 `ReplaceFileW`。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::AppError;

/// 原子写入含凭据的文件。Unix 上新文件和替换文件始终使用 0600。
pub fn atomic_write_private(path: &Path, data: &[u8]) -> Result<(), AppError> {
    stage_write(path, data, Some(0o600), false)?.commit()
}

/// 原子写入：沿用目标文件现有的权限位。
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), AppError> {
    stage_write(path, data, None, false)?.commit()
}

/// 已写好、还没替换目标的临时文件。原子写的前半步：写入引擎先把一次操作涉及的
/// 所有文件都备好临时文件、记下写前意图，再逐个 [`StagedWrite::commit`]。
#[derive(Debug)]
pub struct StagedWrite {
    tmp: PathBuf,
    path: PathBuf,
}

impl StagedWrite {
    pub fn tmp_path(&self) -> &Path {
        &self.tmp
    }

    /// 用临时文件替换目标（失败时删掉临时文件）。
    pub fn commit(self) -> Result<(), AppError> {
        commit_staged(&self.tmp, &self.path).inspect_err(|_| {
            let _ = fs::remove_file(&self.tmp);
        })
    }
}

/// 写入临时文件：Unix 下 `unix_mode` 为 `None` 时沿用目标文件现有的权限位。
/// `durable` 为真时写完先 fsync，崩溃恢复要靠这份临时文件前滚。
pub fn stage_write(
    path: &Path,
    data: &[u8],
    unix_mode: Option<u32>,
    durable: bool,
) -> Result<StagedWrite, AppError> {
    #[cfg(not(unix))]
    let _ = unix_mode;

    let parent = path
        .parent()
        .ok_or_else(|| AppError::Config("无效的路径".to_string()))?;
    fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Config("无效的文件名".to_string()))?
        .to_string_lossy()
        .to_string();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let (tmp, mut file) = (|| -> Result<(PathBuf, fs::File), AppError> {
        let mut last_collision = None;
        for _ in 0..16 {
            let counter = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let candidate = parent.join(format!(
                "{file_name}.tmp.{}.{ts}.{counter}",
                std::process::id()
            ));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            if let Some(mode) = unix_mode {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(mode);
            }
            match options.open(&candidate) {
                Ok(file) => return Ok((candidate, file)),
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_collision = Some((candidate, source));
                }
                Err(source) => return Err(AppError::io(&candidate, source)),
            }
        }
        let (candidate, source) = last_collision.expect("temporary filename loop must run");
        Err(AppError::io(&candidate, source))
    })()?;

    let written = file
        .write_all(data)
        .and_then(|_| file.flush())
        .and_then(|_| if durable { file.sync_all() } else { Ok(()) });
    if let Err(source) = written {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(AppError::io(&tmp, source));
    }
    drop(file);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(mode) = unix_mode {
            if let Err(source) = fs::set_permissions(&tmp, fs::Permissions::from_mode(mode)) {
                let _ = fs::remove_file(&tmp);
                return Err(AppError::io(&tmp, source));
            }
        } else if let Ok(meta) = fs::metadata(path) {
            let perm = meta.permissions().mode();
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(perm));
        }
    }

    Ok(StagedWrite {
        tmp,
        path: path.to_path_buf(),
    })
}

/// 原子写的后半步：用 `tmp` 替换 `path`。崩溃恢复也用它提交上次留下的临时文件。
///
/// 失败时临时文件留在原处：写入引擎的 pending 指着它，下次恢复要靠它前滚。
pub fn commit_staged(tmp: &Path, path: &Path) -> Result<(), AppError> {
    fs::rename(tmp, path).map_err(|source| AppError::IoContext {
        context: format!("原子替换失败: {} -> {}", tmp.display(), path.display()),
        source,
    })
}

/// 删除文件（不存在不算错）。
pub fn delete_file(path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AppError::io(path, err)),
    }
}

/// 键按字母排序的 pretty JSON（确定性输出）。Codex `auth.json` 整份写入时用。
pub fn sorted_json_bytes<T: Serialize>(data: &T) -> Result<Vec<u8>, AppError> {
    let value = serde_json::to_value(data).map_err(|e| AppError::JsonSerialize { source: e })?;
    let json = serde_json::to_string_pretty(&sort_json_keys(&value))
        .map_err(|e| AppError::JsonSerialize { source: e })?;
    Ok(json.into_bytes())
}

fn sort_json_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            let mut sorted = Map::new();
            for key in keys {
                sorted.insert(key.clone(), sort_json_keys(&map[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(sort_json_keys).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorted_json_is_deterministic_and_keeps_array_order() {
        let a = sorted_json_bytes(&json!({"z": 1, "a": {"y": [3, 1], "b": 2}})).unwrap();
        assert_eq!(
            String::from_utf8(a).unwrap(),
            "{\n  \"a\": {\n    \"b\": 2,\n    \"y\": [\n      3,\n      1\n    ]\n  },\n  \"z\": 1\n}"
        );
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        fs::write(&path, "old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn private_writes_are_owner_only_even_over_a_world_readable_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        fs::write(&path, "{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        atomic_write_private(&path, b"{\"k\":1}").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn deleting_a_missing_file_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        delete_file(&dir.path().join("nope")).unwrap();
    }
}
