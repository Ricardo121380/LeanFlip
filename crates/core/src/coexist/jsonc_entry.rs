//! Shared JSONC editing primitives for provider maps.
use super::{key_conflict, rejected, Edit, EntryChange, WriteMode};
use crate::{
    error::AppError,
    live::patch::{decode_utf8, jsonc::JsoncDocument, LivePatch, LiveWriteError},
    provider::Tool,
};
use serde_json::{json, Map, Value};
use std::path::Path;
pub(crate) struct JsoncEdit<F> {
    pub default_source: &'static str,
    pub max_bytes: Option<usize>,
    pub edit: F,
}
pub(crate) fn parse_value(path: &Path, text: &str, default: &str) -> Result<Value, AppError> {
    let source = if text.trim().is_empty() {
        default
    } else {
        text
    };
    let document = JsoncDocument::parse(source)
        .map_err(|_| AppError::Config(format!("{}：JSONC 格式无效", path.display())))?;
    if !document.value().is_object() {
        return Err(AppError::Config(
            "配置根节点必须是对象 / Configuration root must be an object".into(),
        ));
    }
    Ok(document.value().clone())
}
impl<F: Fn(&mut Value) -> Result<(), AppError>> LivePatch for JsoncEdit<F> {
    fn apply(&self, path: &Path, before: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let text = decode_utf8(path, before.unwrap_or_default())?;
        if self.max_bytes.is_some_and(|limit| text.len() > limit) {
            return Err(rejected(AppError::InvalidInput(
                "配置文件超过大小上限 / Configuration exceeds size limit".into(),
            )));
        }
        let source = if text.trim().is_empty() {
            self.default_source
        } else {
            text
        };
        let mut document = JsoncDocument::parse(source)
            .map_err(|_| rejected(AppError::Config("JSONC 格式无效 / Invalid JSONC".into())))?;
        let mut value = document.value().clone();
        (self.edit)(&mut value).map_err(rejected)?;
        document.apply(&value).map_err(rejected)?;
        Ok(document.validated_source().map_err(rejected)?.into_bytes())
    }
}
pub(crate) fn entries_at(value: &Value, path: &[&str]) -> Result<Map<String, Value>, AppError> {
    let mut current = value;
    for key in path {
        if !current.is_object() {
            return Err(AppError::Config(format!(
                "{key} 的父节点不是对象 / Parent is not an object"
            )));
        }
        match current.get(*key) {
            Some(child) => current = child,
            None => return Ok(Map::new()),
        }
    }
    current.as_object().cloned().ok_or_else(|| {
        AppError::Config(format!(
            "{} 必须是对象 / Expected an object",
            path.join(".")
        ))
    })
}
pub(crate) fn edit_map(
    value: &mut Value,
    path: &[&str],
    change: &EntryChange<'_>,
    tool: Tool,
) -> Result<(), AppError> {
    let mut current = value;
    for key in path {
        let map = current.as_object_mut().ok_or_else(|| {
            AppError::Config(format!(
                "{} 必须是对象 / Expected an object",
                path.join(".")
            ))
        })?;
        if !map.contains_key(*key) && matches!(change.edit, Edit::Remove) {
            return Ok(());
        }
        current = map.entry(*key).or_insert_with(|| json!({}));
    }
    let map = current.as_object_mut().ok_or_else(|| {
        AppError::Config("供应商容器必须是对象 / Provider container must be an object".into())
    })?;
    match &change.edit {
        Edit::Remove => {
            map.shift_remove(change.id);
        }
        Edit::Upsert(next) => {
            if change.mode == WriteMode::CreateOnly
                && map.get(change.id).is_some_and(|old| old != next)
            {
                return Err(key_conflict(tool, change.id));
            }
            map.insert(change.id.into(), next.clone());
        }
    }
    Ok(())
}
