//! OpenClaw（`~/.openclaw/openclaw.json`，JSON5）：供应商在 `models.providers.<id>`。
//! 移植自 cc-switch `openclaw_config.rs` 的供应商部分；改为节点级保留格式编辑
//! （原版整段重写 `models`）。

use std::path::Path;

use serde_json::{json, Map, Value};

use super::jsonc_entry::{parse_value, JsoncEdit};
use super::{key_conflict, Edit, EntryChange, WriteMode};
use crate::error::AppError;
use crate::provider::{Provider, Tool};

const DEFAULT_SOURCE: &str =
    "{\n  \"models\": {\n    \"mode\": \"merge\",\n    \"providers\": {}\n  }\n}\n";

pub(crate) fn entries(path: &Path, text: &str) -> Result<Map<String, Value>, AppError> {
    let config = parse_value(path, text, DEFAULT_SOURCE)?;
    Ok(config
        .get("models")
        .and_then(|m| m.get("providers"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default())
}

pub(crate) fn validate(provider: &Provider) -> Result<(), AppError> {
    let settings = &provider.settings_config;
    if !settings.is_object()
        || !["baseUrl", "api", "models"]
            .iter()
            .any(|key| settings.get(*key).is_some())
    {
        return Err(AppError::localized(
            "provider.openclaw.invalid",
            format!(
                "OpenClaw 供应商 {} 的配置必须是含 baseUrl、api 或 models 的对象",
                provider.id
            ),
            format!(
                "OpenClaw provider {} must be an object with baseUrl, api or models",
                provider.id
            ),
        ));
    }
    Ok(())
}

fn object<'a>(value: &'a mut Value, what: &str) -> Result<&'a mut Map<String, Value>, AppError> {
    value
        .as_object_mut()
        .ok_or_else(|| AppError::Config(format!("OpenClaw 配置里的 {what} 不是对象，没有写入")))
}

pub(crate) fn patch<'a>(
    change: &'a EntryChange<'a>,
) -> JsoncEdit<impl Fn(&mut Value) -> Result<(), AppError> + 'a> {
    JsoncEdit {
        default_source: DEFAULT_SOURCE,
        max_bytes: None,
        edit: move |config: &mut Value| {
            let root = object(config, "根")?;
            match &change.edit {
                Edit::Remove => {
                    if let Some(providers) = root
                        .get_mut("models")
                        .and_then(|models| models.get_mut("providers"))
                        .and_then(Value::as_object_mut)
                    {
                        providers.shift_remove(change.id);
                    }
                    Ok(())
                }
                Edit::Upsert(settings) => {
                    let models = root
                        .entry("models")
                        .or_insert_with(|| json!({ "mode": "merge", "providers": {} }));
                    let providers = object(models, "models")?
                        .entry("providers")
                        .or_insert_with(|| json!({}));
                    let providers = object(providers, "models.providers")?;
                    if change.mode == WriteMode::CreateOnly
                        && providers
                            .get(change.id)
                            .is_some_and(|existing| existing != settings)
                    {
                        return Err(key_conflict(Tool::OpenClaw, change.id));
                    }
                    providers.insert(change.id.to_string(), settings.clone());
                    Ok(())
                }
            }
        },
    }
}
