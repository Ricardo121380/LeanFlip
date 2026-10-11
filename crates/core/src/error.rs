//! Structured errors shared by the desktop and terminal frontends.
use std::{fmt, path::Path};
#[derive(Debug)]
pub enum AppError {
    Config(String),
    InvalidInput(String),
    Conflict(String),
    Message(String),
    NotFound(String),
    Io {
        path: String,
        source: std::io::Error,
    },
    IoContext {
        context: String,
        source: std::io::Error,
    },
    Json {
        path: String,
        source: serde_json::Error,
    },
    JsonSerialize {
        source: serde_json::Error,
    },
    Localized {
        key: &'static str,
        zh: String,
        en: String,
    },
}
impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => write!(f, "配置无效：{message}"),
            Self::InvalidInput(message) => write!(f, "输入无效：{message}"),
            Self::Conflict(message) => write!(f, "文件已被其他程序修改：{message}"),
            Self::Message(message) => f.write_str(message),
            Self::NotFound(message) => write!(f, "未找到：{message}"),
            Self::Localized { zh, en, .. } => write!(f, "{zh} ({en})"),
            Self::Io { path, source } => write!(f, "无法访问 {path}：{source}"),
            Self::IoContext { context, source } => write!(f, "{context}：{source}"),
            // Parser messages can include credentials from the malformed source.
            Self::Json { path, source } => write!(
                f,
                "{path}：JSON 格式错误（{}:{}）",
                source.line(),
                source.column()
            ),
            Self::JsonSerialize { .. } => f.write_str("无法编码 JSON 数据"),
        }
    }
}
impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::IoContext { source, .. } => Some(source),
            Self::Json { source, .. } | Self::JsonSerialize { source } => Some(source),
            _ => None,
        }
    }
}
impl AppError {
    pub fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().display().to_string(),
            source,
        }
    }
    pub fn json(path: impl AsRef<Path>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.as_ref().display().to_string(),
            source,
        }
    }
    pub fn localized(key: &'static str, zh: impl Into<String>, en: impl Into<String>) -> Self {
        Self::Localized {
            key,
            zh: zh.into(),
            en: en.into(),
        }
    }
}
impl From<AppError> for String {
    fn from(error: AppError) -> String {
        error.to_string()
    }
}
