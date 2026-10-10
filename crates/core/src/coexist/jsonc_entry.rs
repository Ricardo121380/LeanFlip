//! JSONC / JSON5 文件里一个条目的修改：以写前内容为底，用保留格式的编辑器改，
//! 只有语义变了的节点会重写。

use std::path::Path;

use serde_json::Value;

use super::rejected;
use crate::error::AppError;
use crate::live::patch::jsonc::JsoncDocument;
use crate::live::patch::{decode_utf8, LivePatch, LiveWriteError};

pub(crate) struct JsoncEdit<F> {
    /// 文件不存在（或是空的）时的起始内容。
    pub default_source: &'static str,
    /// 文件大小上限（Pi 的 `models.json` 限 1 MiB）。
    pub max_bytes: Option<usize>,
    pub edit: F,
}

/// 把 JSONC / JSON5 文本解析成值；空文本按 `default_source`。
pub(crate) fn parse_value(
    path: &Path,
    text: &str,
    default_source: &str,
) -> Result<Value, AppError> {
    let source = if text.trim().is_empty() {
        default_source
    } else {
        text
    };
    let value: Value = json5::from_str(source).map_err(|e| {
        AppError::Config(format!("无法解析 {}（JSONC / JSON5）: {e}", path.display()))
    })?;
    if !value.is_object() {
        return Err(AppError::Config(format!(
            "{} 的根必须是对象",
            path.display()
        )));
    }
    Ok(value)
}

impl<F> LivePatch for JsoncEdit<F>
where
    F: Fn(&mut Value) -> Result<(), AppError>,
{
    fn apply(&self, path: &Path, pre: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        if let (Some(max), Some(pre)) = (self.max_bytes, pre) {
            if pre.len() > max {
                return Err(rejected(AppError::InvalidInput(format!(
                    "{} 超过 {max} 字节上限",
                    path.display()
                ))));
            }
        }
        let text = match pre {
            Some(bytes) => decode_utf8(path, bytes)?,
            None => "",
        };
        let source = if text.trim().is_empty() {
            self.default_source
        } else {
            text
        };
        let mut document = JsoncDocument::parse(source)
            .map_err(|e| rejected(AppError::Config(format!("{}: {e}", path.display()))))?;
        let mut desired = document.value().clone();
        (self.edit)(&mut desired).map_err(rejected)?;
        document.apply(&desired).map_err(rejected)?;
        Ok(document.validated_source().map_err(rejected)?.into_bytes())
    }
}
