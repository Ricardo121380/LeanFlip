//! In-memory edits. Implementations never read or publish client files themselves.
pub mod dotenv;
pub mod json;
pub mod jsonc;
pub mod toml;
use crate::AppError;
use std::{
    fmt,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyPath(pub Vec<String>);
impl KeyPath {
    pub fn new(keys: &[&str]) -> Self {
        Self(keys.iter().map(|key| key.to_string()).collect())
    }
    pub fn root() -> Self {
        Self(Vec::new())
    }
    pub fn child(&self, key: &str) -> Self {
        Self(
            self.0
                .iter()
                .cloned()
                .chain(std::iter::once(key.into()))
                .collect(),
        )
    }
    fn split_last(&self) -> Option<(&[String], &String)> {
        self.0.split_last().map(|(leaf, parents)| (parents, leaf))
    }
}
impl fmt::Display for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("."))
    }
}
pub trait LivePatch {
    fn apply(&self, path: &Path, before: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError>;
    fn apply_file(
        &self,
        path: &Path,
        before: Option<&[u8]>,
    ) -> Result<Option<Vec<u8>>, LiveWriteError> {
        self.apply(path, before).map(Some)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WholeFile {
    Write(Vec<u8>),
    Delete,
}
impl LivePatch for WholeFile {
    fn apply(&self, path: &Path, before: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        Ok(self.apply_file(path, before)?.unwrap_or_default())
    }
    fn apply_file(&self, _: &Path, _: Option<&[u8]>) -> Result<Option<Vec<u8>>, LiveWriteError> {
        Ok(match self {
            Self::Write(bytes) => Some(bytes.clone()),
            Self::Delete => None,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guarded {
    pub expected_pre: Option<String>,
    pub then: WholeFile,
}
impl LivePatch for Guarded {
    fn apply(&self, path: &Path, before: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        Ok(self.apply_file(path, before)?.unwrap_or_default())
    }
    fn apply_file(
        &self,
        path: &Path,
        before: Option<&[u8]>,
    ) -> Result<Option<Vec<u8>>, LiveWriteError> {
        if crate::live::engine::digest(before) == self.expected_pre {
            self.then.apply_file(path, before)
        } else {
            Err(LiveWriteError::Conflict { path: path.into() })
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum LiveWriteError {
    #[error("Cannot parse {path} at {line}:{column}")]
    Parse {
        path: PathBuf,
        line: usize,
        column: usize,
        message: String,
    },
    #[error("Expected {expected} at {key_path} in {path}")]
    Shape {
        path: PathBuf,
        key_path: KeyPath,
        expected: &'static str,
    },
    #[error("Cannot access {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("File changed while preparing an update: {path}")]
    Conflict { path: PathBuf },
    #[error("Edited fields changed concurrently in {path}")]
    EditConflict { path: PathBuf, keys: Vec<String> },
    #[error("Active profile {profile} overrides {key}")]
    Route { profile: String, key: String },
    #[error("{0}")]
    Rejected(Box<AppError>),
}
pub const EDIT_CONFLICT_CODE: &str = "LIVE_EDIT_CONFLICT";
impl From<LiveWriteError> for AppError {
    fn from(error: LiveWriteError) -> Self {
        match error {
            LiveWriteError::Rejected(error) => *error,
            LiveWriteError::Io { path, source } => AppError::io(path, source),
            LiveWriteError::Conflict { path } => AppError::Conflict(path.display().to_string()),
            LiveWriteError::EditConflict { path, keys } => AppError::Message(
                serde_json::json!({"code": EDIT_CONFLICT_CODE, "path": path, "keys": keys})
                    .to_string(),
            ),
            LiveWriteError::Parse {
                path, line, column, ..
            } => AppError::localized(
                "live.parse_error",
                format!(
                    "配置文件无法解析：{}，第 {line} 行、第 {column} 列；尚未写入。",
                    path.display()
                ),
                format!(
                    "Cannot parse {} at {line}:{column}; no files were written.",
                    path.display()
                ),
            ),
            LiveWriteError::Shape {
                path,
                key_path,
                expected,
            } => AppError::localized(
                "live.shape_error",
                format!(
                    "{} 中的 {key_path} 类型不符合预期（{expected}）；尚未写入。",
                    path.display()
                ),
                format!(
                    "Unexpected configuration type at {key_path} in {}; no files were written.",
                    path.display()
                ),
            ),
            LiveWriteError::Route { profile, key } => AppError::localized(
                "provider.codex.config.profile_overrides_route",
                format!("当前 profile {profile} 覆盖了 {key}；请调整后重试。",),
                format!("Active profile {profile} overrides {key}. Review it before switching."),
            ),
        }
    }
}
pub(crate) fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let end = text
        .char_indices()
        .map(|(position, _)| position)
        .chain(std::iter::once(text.len()))
        .take_while(|position| *position <= offset)
        .last()
        .unwrap_or(0);
    let prefix = &text[..end];
    (
        prefix.bytes().filter(|ch| *ch == b'\n').count() + 1,
        prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
    )
}
pub(crate) fn decode_utf8<'a>(path: &Path, bytes: &'a [u8]) -> Result<&'a str, LiveWriteError> {
    std::str::from_utf8(bytes).map_err(|error| {
        let prefix = std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or("");
        let (line, column) = line_column(prefix, prefix.len());
        LiveWriteError::Parse {
            path: path.into(),
            line,
            column,
            message: "Invalid UTF-8".into(),
        }
    })
}
