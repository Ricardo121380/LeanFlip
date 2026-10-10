//! Pi（`~/.pi/agent/models.json`）：供应商在 `providers.<key>`。Pi 自己管登录
//! （`auth.json`）和当前供应商 / 模型（`settings.json`），这两个文件不碰。
//! 移植自 cc-switch `pi_config`；改为保留格式编辑（原版整份重写成 JSON），
//! 文件大小上限 1 MiB 照旧。

use std::path::Path;

use serde_json::{json, Map, Value};

use super::jsonc_entry::{parse_value, JsoncEdit};
use super::{key_conflict, Edit, EntryChange, WriteMode};
use crate::error::AppError;
use crate::provider::{Provider, Tool};

const DEFAULT_SOURCE: &str = "{\n  \"providers\": {}\n}\n";
pub const MAX_MODELS_BYTES: usize = 1024 * 1024;

pub(crate) fn entries(path: &Path, text: &str) -> Result<Map<String, Value>, AppError> {
    if text.len() > MAX_MODELS_BYTES {
        return Err(AppError::InvalidInput(format!(
            "{} 超过 1 MiB 上限",
            path.display()
        )));
    }
    let config = parse_value(path, text, DEFAULT_SOURCE)?;
    match config.get("providers") {
        None => Ok(Map::new()),
        Some(Value::Object(providers)) => Ok(providers.clone()),
        Some(_) => Err(AppError::Config(format!(
            "{} 里的 providers 必须是对象",
            path.display()
        ))),
    }
}

pub(crate) fn validate(provider: &Provider) -> Result<(), AppError> {
    if provider.id.trim().is_empty() {
        return Err(AppError::InvalidInput("Pi 供应商的键不能为空".into()));
    }
    if !provider.settings_config.is_object() {
        return Err(AppError::InvalidInput("Pi 供应商配置必须是对象".into()));
    }
    Ok(())
}

pub(crate) fn patch<'a>(
    change: &'a EntryChange<'a>,
) -> JsoncEdit<impl Fn(&mut Value) -> Result<(), AppError> + 'a> {
    JsoncEdit {
        default_source: DEFAULT_SOURCE,
        max_bytes: Some(MAX_MODELS_BYTES),
        edit: move |config: &mut Value| {
            let root = config
                .as_object_mut()
                .ok_or_else(|| AppError::Config("Pi models.json 的根必须是对象".into()))?;
            let providers = root
                .entry("providers")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| {
                    AppError::Config("Pi models.json 里的 providers 必须是对象".into())
                })?;
            match &change.edit {
                Edit::Remove => {
                    providers.shift_remove(change.id);
                }
                Edit::Upsert(settings) => {
                    if change.mode == WriteMode::CreateOnly
                        && providers
                            .get(change.id)
                            .is_some_and(|existing| existing != settings)
                    {
                        return Err(key_conflict(Tool::Pi, change.id));
                    }
                    providers.insert(change.id.to_string(), settings.clone());
                }
            }
            Ok(())
        },
    }
}
