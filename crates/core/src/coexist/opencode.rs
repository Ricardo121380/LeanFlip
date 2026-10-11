//! OpenCode provider formats and schema validation.
use super::{
    jsonc_entry::{edit_map, entries_at, parse_value, JsoncEdit},
    Edit, EntryChange,
};
use crate::{
    error::AppError,
    provider::{Provider, Tool},
};
use indexmap::IndexMap;
use serde_json::{json, Value};
use std::path::Path;
pub const META_CONFIG_FORMAT: &str = "opencodeConfigFormat";
const EMPTY: &str = "{\n  \"$schema\": \"https://opencode.ai/config.json\"\n}\n";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    V1,
    V2,
}
impl Format {
    fn field(self) -> &'static str {
        match self {
            Self::V1 => "provider",
            Self::V2 => "providers",
        }
    }
}
fn stored(provider: &Provider) -> Option<Format> {
    match provider
        .meta
        .extra
        .get(META_CONFIG_FORMAT)
        .and_then(Value::as_str)
    {
        Some("v1") => Some(Format::V1),
        Some("v2") => Some(Format::V2),
        _ => None,
    }
}
pub fn provider_format(value: &Value, source: Option<Format>) -> Format {
    source.unwrap_or_else(|| {
        if !["npm", "options", "api"]
            .iter()
            .any(|k| value.get(*k).is_some())
            && ["package", "settings", "headers", "body", "canonical"]
                .iter()
                .any(|k| value.get(*k).is_some())
        {
            Format::V2
        } else {
            Format::V1
        }
    })
}
pub fn provider_fragment<'a>(
    id: &str,
    value: &'a Value,
    source: Option<Format>,
) -> Result<(&'a Value, Format), AppError> {
    let native = value.get("providers").and_then(|p| p.get(id));
    let legacy = value.get("provider").and_then(|p| p.get(id));
    if let Some(native) =
        native.filter(|v| native_provider_problem(v).is_none() || legacy.is_none())
    {
        return Ok((native, Format::V2));
    }
    if let Some(legacy) = legacy {
        return Ok((legacy, Format::V1));
    }
    if value.get("provider").is_some() || value.get("providers").is_some() {
        return Err(AppError::NotFound(format!("OpenCode provider {id}")));
    }
    Ok((value, provider_format(value, source)))
}
pub(crate) fn entries(
    path: &Path,
    text: &str,
) -> Result<IndexMap<String, (Value, Format)>, AppError> {
    let value = parse_value(path, text, EMPTY)?;
    let mut result = IndexMap::new();
    for format in [Format::V1, Format::V2] {
        for (id, entry) in entries_at(&value, &[format.field()])? {
            if format == Format::V1 || native_provider_problem(&entry).is_none() {
                result.insert(id, (entry, format));
            }
        }
    }
    Ok(result)
}
pub(crate) fn imported_card(id: &str, value: Value, format: Format) -> Provider {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(id)
        .to_string();
    let mut provider = Provider::new(id, name, value);
    if format == Format::V2 {
        provider
            .meta
            .extra
            .insert(META_CONFIG_FORMAT.into(), json!("v2"));
    }
    provider
}
pub fn validate_native_provider(id: &str, value: &Value) -> Result<(), AppError> {
    if let Some(path) = native_provider_problem(value) {
        Err(AppError::InvalidInput(format!(
            "OpenCode {id} 字段无效 / Invalid field: {path}"
        )))
    } else {
        Ok(())
    }
}
pub(crate) fn validate(provider: &Provider) -> Result<(), AppError> {
    let (value, format) =
        provider_fragment(&provider.id, &provider.settings_config, stored(provider))?;
    if format == Format::V2 {
        return validate_native_provider(&provider.id, value);
    }
    if !value.is_object()
        || (!value.get("npm").is_some_and(Value::is_string)
            && !value.get("options").is_some_and(Value::is_object))
    {
        return Err(AppError::InvalidInput(
            "OpenCode 连接需要 npm 或 options / OpenCode connection needs npm or options".into(),
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
        edit: move |root: &mut Value| {
            if matches!(change.edit, Edit::Remove) {
                let legacy = root
                    .get("provider")
                    .and_then(|v| v.get(change.id))
                    .is_some();
                let valid_native = root
                    .get("providers")
                    .and_then(|v| v.get(change.id))
                    .is_some_and(|v| native_provider_problem(v).is_none());
                edit_map(root, &["provider"], change, Tool::OpenCode)?;
                if !legacy || valid_native {
                    edit_map(root, &["providers"], change, Tool::OpenCode)?;
                }
                return Ok(());
            }
            let Edit::Upsert(settings) = &change.edit else {
                unreachable!()
            };
            let (value, format) =
                provider_fragment(change.id, settings, change.provider.and_then(stored))?;
            if format == Format::V2 {
                validate_native_provider(change.id, value)?;
            } else {
                if root
                    .get("providers")
                    .and_then(|v| v.get(change.id))
                    .is_some_and(|v| native_provider_problem(v).is_none())
                {
                    return Err(AppError::Config("此 OpenCode 连接已有 V2 定义，请重新导入 / Import the V2 definition before editing".into()));
                }
                let complete = value
                    .get("npm")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.trim().is_empty())
                    && value
                        .get("models")
                        .and_then(Value::as_object)
                        .is_some_and(|m| !m.is_empty());
                if !complete
                    && root
                        .get("provider")
                        .and_then(|p| p.get(change.id))
                        .is_none()
                {
                    return Err(AppError::InvalidInput("新 OpenCode 连接需要 npm 包与模型 / New OpenCode connections need a package and model".into()));
                }
            }
            let edit = EntryChange {
                provider: change.provider,
                id: change.id,
                edit: Edit::Upsert(value.clone()),
                mode: change.mode,
                activate: false,
            };
            edit_map(root, &[format.field()], &edit, Tool::OpenCode)?;
            if format == Format::V2 {
                if let Some(legacy) = root.get_mut("provider").and_then(Value::as_object_mut) {
                    legacy.shift_remove(change.id);
                }
            }
            Ok(())
        },
    }
}
// Declarative field checks follow the client schema. Unknown extension fields
// are retained, but malformed known fields are rejected before any disk write.
#[derive(Clone, Copy)]
enum Shape {
    Provider,
    Model,
    Variant,
    Settings(bool),
    Compatibility,
    Limit,
    Cost,
    Cache,
    Tier,
    Capabilities,
}
fn finite(v: &Value) -> bool {
    v.as_f64().is_some_and(f64::is_finite)
}
fn integer(v: &Value) -> bool {
    v.as_f64()
        .is_some_and(|n| n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0)
}
fn strings(v: &Value) -> bool {
    v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
}
fn validate_object(value: &Value, shape: Shape, prefix: &str) -> Option<String> {
    let Some(map) = value.as_object() else {
        return Some(prefix.into());
    };
    let location = |key: &str| {
        if prefix.is_empty() {
            key.into()
        } else {
            format!("{prefix}.{key}")
        }
    };
    let required: &[&str] = match shape {
        Shape::Capabilities => &["tools", "input", "output"],
        Shape::Cost => &["input", "output"],
        Shape::Tier => &["type", "size"],
        Shape::Variant => &["id"],
        _ => &[],
    };
    for key in required {
        if !map.contains_key(*key) {
            return Some(location(key));
        }
    }
    for (key, v) in map {
        let path = location(key);
        let nested = match key.as_str() {
            "settings" if matches!(shape, Shape::Provider | Shape::Model | Shape::Variant) => {
                Some(Shape::Settings(matches!(shape, Shape::Provider)))
            }
            "compatibility" if matches!(shape, Shape::Model) => Some(Shape::Compatibility),
            "limit" if matches!(shape, Shape::Model) => Some(Shape::Limit),
            "capabilities" if matches!(shape, Shape::Model) => Some(Shape::Capabilities),
            "tier" if matches!(shape, Shape::Cost) => Some(Shape::Tier),
            "cache" if matches!(shape, Shape::Cost) => Some(Shape::Cache),
            _ => None,
        };
        if let Some(next) = nested {
            if let Some(error) = validate_object(v, next, &path) {
                return Some(error);
            }
            continue;
        }
        let valid = match (shape, key.as_str()) {
            (Shape::Provider, "name" | "package" | "canonical")
            | (Shape::Model, "name" | "modelID" | "family" | "package")
            | (Shape::Variant, "id")
            | (Shape::Compatibility, "reasoningField") => v.is_string(),
            (Shape::Provider, "env") | (Shape::Capabilities, "input" | "output") => strings(v),
            (Shape::Provider | Shape::Model | Shape::Variant, "headers") => v
                .as_object()
                .is_some_and(|m| m.values().all(Value::is_string)),
            (Shape::Provider | Shape::Model | Shape::Variant, "body") => v.is_object(),
            (Shape::Model, "disabled")
            | (Shape::Capabilities, "tools")
            | (
                Shape::Compatibility,
                "requireReasoning"
                | "requireFinishReason"
                | "requireAssistantAfterTool"
                | "supportsPromptCacheKey",
            ) => v.is_boolean(),
            (Shape::Settings(true), "timeout") => v == &Value::Bool(false) || finite(v),
            (Shape::Settings(true), "chunkTimeout") => finite(v),
            (Shape::Settings(true), "transport") => {
                matches!(v.as_str(), Some("http" | "websocket"))
            }
            (Shape::Settings(_), "compaction") => v
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|s| s == "summary" || s == "native"),
            (Shape::Compatibility, "maxTokensField") => {
                matches!(v.as_str(), Some("max_tokens" | "max_completion_tokens"))
            }
            (Shape::Limit, "context" | "input" | "output") | (Shape::Tier, "size") => integer(v),
            (Shape::Cost, "input" | "output") | (Shape::Cache, "read" | "write") => finite(v),
            (Shape::Tier, "type") => v.as_str() == Some("context"),
            (Shape::Provider, "models") => {
                let Some(models) = v.as_object() else {
                    return Some(path);
                };
                for (id, model) in models {
                    if let Some(error) =
                        validate_object(model, Shape::Model, &format!("{path}.{id}"))
                    {
                        return Some(error);
                    }
                }
                true
            }
            (Shape::Model, "variants") => {
                let Some(items) = v.as_array() else {
                    return Some(path);
                };
                for item in items {
                    if let Some(error) = validate_object(item, Shape::Variant, &path) {
                        return Some(error);
                    }
                }
                true
            }
            (Shape::Model, "cost") => {
                let items: Vec<&Value> = match v.as_array() {
                    Some(a) => a.iter().collect(),
                    None => vec![v],
                };
                for item in items {
                    if let Some(error) = validate_object(item, Shape::Cost, &path) {
                        return Some(error);
                    }
                }
                true
            }
            _ => true,
        };
        if !valid {
            return Some(path);
        }
    }
    None
}
pub fn native_provider_problem(value: &Value) -> Option<String> {
    validate_object(value, Shape::Provider, "")
}
