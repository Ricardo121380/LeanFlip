//! Pi connection definitions. Its login and active-model files are separate.
use super::{
    jsonc_entry::{edit_map, entries_at, parse_value, JsoncEdit},
    EntryChange,
};
use crate::{
    error::AppError,
    provider::{Provider, Tool},
};
use serde_json::{Map, Value};
use std::path::Path;
pub const MAX_MODELS_BYTES: usize = 1024 * 1024;
const EMPTY: &str = "{\n  \"providers\": {}\n}\n";
pub(crate) fn entries(path: &Path, text: &str) -> Result<Map<String, Value>, AppError> {
    if text.len() > MAX_MODELS_BYTES {
        return Err(AppError::InvalidInput(
            "Pi 配置超过 1 MiB / Pi configuration exceeds 1 MiB".into(),
        ));
    }
    entries_at(&parse_value(path, text, EMPTY)?, &["providers"])
}
pub(crate) fn validate(provider: &Provider) -> Result<(), AppError> {
    if provider.id.trim().is_empty() || !provider.settings_config.is_object() {
        Err(AppError::localized(
            "provider.pi.invalid",
            "Pi 连接需要非空名称和对象配置",
            "A Pi connection needs a non-empty name and an object",
        ))
    } else {
        Ok(())
    }
}
pub(crate) fn patch<'a>(
    change: &'a EntryChange<'a>,
) -> JsoncEdit<impl Fn(&mut Value) -> Result<(), AppError> + 'a> {
    JsoncEdit {
        default_source: EMPTY,
        max_bytes: Some(MAX_MODELS_BYTES),
        edit: move |value: &mut Value| edit_map(value, &["providers"], change, Tool::Pi),
    }
}
