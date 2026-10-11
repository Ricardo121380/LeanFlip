//! OpenClaw stores named connections under models.providers.
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
const EMPTY: &str = "{\n  \"models\": {\"mode\": \"merge\", \"providers\": {}}\n}\n";
pub(crate) fn entries(path: &Path, text: &str) -> Result<Map<String, Value>, AppError> {
    entries_at(&parse_value(path, text, EMPTY)?, &["models", "providers"])
}
pub(crate) fn validate(provider: &Provider) -> Result<(), AppError> {
    let value = &provider.settings_config;
    if provider.id.trim().is_empty()
        || !value.is_object()
        || !["baseUrl", "api", "models"]
            .iter()
            .any(|k| value.get(*k).is_some())
    {
        return Err(AppError::localized(
            "provider.openclaw.invalid",
            "OpenClaw 连接需要名称及 baseUrl、api 或 models",
            "An OpenClaw connection needs a name and baseUrl, api or models",
        ));
    }
    Ok(())
}
pub(crate) fn patch<'a>(
    change: &'a EntryChange<'a>,
) -> JsoncEdit<impl Fn(&mut Value) -> Result<(), AppError> + 'a> {
    JsoncEdit {
        default_source: EMPTY,
        max_bytes: None,
        edit: move |value: &mut Value| {
            edit_map(value, &["models", "providers"], change, Tool::OpenClaw)
        },
    }
}
