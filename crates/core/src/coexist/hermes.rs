//! Hermes list-based custom connections with lossless edits outside selected sections.
use super::{key_conflict, rejected, yaml_sections::Document, Edit, EntryChange, WriteMode};
use crate::{
    error::AppError,
    live::patch::{decode_utf8, LivePatch, LiveWriteError},
    provider::{Provider, Tool},
};
use serde_json::{json, Map, Value};
use std::path::Path;
fn normalize(value: &Value, write: bool) -> Result<Value, AppError> {
    let mut object = value.as_object().cloned().ok_or_else(|| {
        AppError::Config("Hermes 连接必须是对象 / Hermes connection must be an object".into())
    })?;
    for (camel, snake) in [
        ("apiKey", "api_key"),
        ("baseUrl", "base_url"),
        ("contextLength", "context_length"),
        ("maxTokens", "max_tokens"),
        ("apiMode", "api_mode"),
    ] {
        if let Some(value) = object.shift_remove(camel) {
            object.entry(snake).or_insert(value);
        }
    }
    for key in ["api", "_cc_source", "provider_key"] {
        object.shift_remove(key);
    }
    if let Some(models) = object.get("models") {
        let converted = match (write, models) {
            (true, Value::Array(models)) => {
                let mut map = Map::new();
                for model in models {
                    if let Some(id) = model
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        let mut entry = model.as_object().cloned().unwrap_or_default();
                        entry.shift_remove("id");
                        map.insert(id.into(), Value::Object(entry));
                    }
                }
                Some(Value::Object(map))
            }
            (false, Value::Object(models)) => {
                let mut list = Vec::new();
                for (id, model) in models {
                    if model.is_object() || model.is_null() {
                        let mut model = model.as_object().cloned().unwrap_or_default();
                        model.insert("id".into(), json!(id));
                        list.push(Value::Object(model));
                    }
                }
                Some(Value::Array(list))
            }
            _ => None,
        };
        if let Some(value) = converted {
            object.insert("models".into(), value);
        }
    }
    Ok(Value::Object(object))
}
pub(crate) fn entries(text: &str) -> Result<Map<String, Value>, AppError> {
    let doc = Document::parse(text)?;
    let mut result = Map::new();
    if let Some(list) = doc.value.get("custom_providers") {
        if !list.is_null() && !list.is_array() {
            return Err(AppError::Config(
                "Hermes custom_providers 必须是列表 / Expected a list".into(),
            ));
        }
        for entry in list.as_array().into_iter().flatten() {
            if let Some(name) = entry.get("name").and_then(Value::as_str) {
                result.insert(name.into(), normalize(entry, false)?);
            }
        }
    }
    Ok(result)
}
/// 顶层 `model.provider` 指向的那一家（没有设置时是 `None`）。
pub(crate) fn active(text: &str) -> Result<Option<String>, AppError> {
    let doc = Document::parse(text)?;
    Ok(doc
        .value
        .get("model")
        .and_then(|model| model.get("provider"))
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_string))
}
pub(crate) fn validate(provider: &Provider) -> Result<(), AppError> {
    if provider.id.trim().is_empty() || !provider.settings_config.is_object() {
        Err(AppError::localized(
            "provider.hermes.invalid",
            "Hermes 连接需要非空名称和对象配置",
            "A Hermes connection needs a non-empty name and an object",
        ))
    } else {
        Ok(())
    }
}
pub(crate) struct HermesPatch<'a> {
    pub change: &'a EntryChange<'a>,
}
impl HermesPatch<'_> {
    fn edit(&self, source: &str) -> Result<String, AppError> {
        let doc = Document::parse(source)?;
        let name = self.change.id;
        let mut providers = match doc.value.get("custom_providers") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(list)) => list.clone(),
            _ => {
                return Err(AppError::Config(
                    "Hermes custom_providers 必须是列表 / Expected a list".into(),
                ))
            }
        };
        let position = providers
            .iter()
            .position(|p| p.get("name").and_then(Value::as_str) == Some(name));
        if position.is_none()
            && doc
                .value
                .get("providers")
                .and_then(Value::as_object)
                .is_some_and(|map| {
                    map.iter().any(|(id, p)| {
                        p.is_object()
                            && (id == name || p.get("name").and_then(Value::as_str) == Some(name))
                    })
                })
        {
            return Err(AppError::InvalidInput(format!(
                "{name} 由 Hermes 账号字典管理，请在 Hermes 中编辑 / Managed by Hermes"
            )));
        }
        let mut first = None;
        match &self.change.edit {
            Edit::Remove => {
                providers.retain(|p| p.get("name").and_then(Value::as_str) != Some(name))
            }
            Edit::Upsert(value) => {
                let mut entry = normalize(value, true)?;
                entry["name"] = json!(name);
                first = entry
                    .get("models")
                    .and_then(Value::as_object)
                    .and_then(|m| m.keys().next())
                    .cloned();
                if let Some(model) = &first {
                    entry["model"] = json!(model);
                } else {
                    entry.as_object_mut().unwrap().shift_remove("model");
                }
                if let Some(index) = position {
                    for (key, value) in providers[index].as_object().into_iter().flatten() {
                        entry
                            .as_object_mut()
                            .unwrap()
                            .entry(key)
                            .or_insert_with(|| value.clone());
                    }
                    if self.change.mode == WriteMode::CreateOnly && providers[index] != entry {
                        return Err(key_conflict(Tool::Hermes, name));
                    }
                    providers[index] = entry;
                } else {
                    providers.push(entry);
                }
            }
        }
        let mut changes = vec![("custom_providers", Value::Array(providers))];
        if self.change.activate {
            let mut model = match doc.value.get("model") {
                None | Some(Value::Null) => json!({}),
                Some(Value::String(s)) if s.is_empty() => json!({}),
                Some(value) if value.is_object() => value.clone(),
                _ => {
                    return Err(AppError::Config(
                        "Hermes model 必须是对象 / Expected model mapping".into(),
                    ))
                }
            };
            model["provider"] = json!(name);
            if let Some(model_id) = first {
                model["default"] = json!(model_id);
            }
            changes.push(("model", model));
        }
        doc.replace(&changes)
    }
}
impl LivePatch for HermesPatch<'_> {
    fn apply(&self, path: &Path, before: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        self.edit(decode_utf8(path, before.unwrap_or_default())?)
            .map(String::into_bytes)
            .map_err(rejected)
    }
}
