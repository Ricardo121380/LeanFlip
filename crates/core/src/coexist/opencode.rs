//! OpenCode（`~/.config/opencode/opencode.json[c]`）。
//!
//! 两种声明格式并存：旧版（V1）在 `provider.<id>`（`npm`、`options`、`models`），原生
//! （V2）在 `providers.<id>`（`package`、`settings`……）。读取时合法的 V2 优先；写入按卡记
//! 下的格式（`meta.opencodeConfigFormat`，没记就按内容推断）。移植自 cc-switch
//! `opencode_config.rs`（格式推断、片段提取、V2 字段校验照搬），不含 OMO 插件。

use std::path::Path;

use indexmap::IndexMap;
use serde_json::{json, Map, Value};

use super::jsonc_entry::{parse_value, JsoncEdit};
use super::{key_conflict, Edit, EntryChange, WriteMode};
use crate::error::AppError;
use crate::provider::{Provider, Tool};

pub const META_CONFIG_FORMAT: &str = "opencodeConfigFormat";

const DEFAULT_SOURCE: &str = "{\n  \"$schema\": \"https://opencode.ai/config.json\"\n}\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    V1,
    V2,
}

impl Format {
    fn key(self) -> &'static str {
        match self {
            Format::V1 => "provider",
            Format::V2 => "providers",
        }
    }
}

fn stored_format(provider: &Provider) -> Option<Format> {
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

/// live 里的条目：合法的 V2 优先，不合法的 V2 不碰（OpenCode 也会跳过它）。
pub(crate) fn entries(
    path: &Path,
    text: &str,
) -> Result<IndexMap<String, (Value, Format)>, AppError> {
    let mut config = parse_value(path, text, DEFAULT_SOURCE)?;
    let mut take = |key| match config.get_mut(key).map(Value::take) {
        Some(Value::Object(providers)) => providers,
        _ => Map::new(),
    };
    let (legacy, native) = (take("provider"), take("providers"));
    let mut providers: IndexMap<_, _> = legacy
        .into_iter()
        .map(|(id, value)| (id, (value, Format::V1)))
        .collect();
    for (id, value) in native {
        if native_provider_problem(&value).is_none() {
            providers.insert(id, (value, Format::V2));
        } else {
            log::warn!("OpenCode 原生供应商 {id} 不合法，保持原样");
        }
    }
    Ok(providers)
}

pub(crate) fn imported_card(id: &str, value: Value, format: Format) -> Provider {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(id)
        .to_string();
    let mut card = Provider::new(id, name, value);
    if format == Format::V2 {
        card.meta
            .extra
            .insert(META_CONFIG_FORMAT.into(), json!("v2"));
    }
    card
}

const LEGACY_ONLY_KEYS: [&str; 3] = ["npm", "options", "api"];
const NATIVE_ONLY_KEYS: [&str; 5] = ["package", "settings", "headers", "body", "canonical"];

/// 来源不明时按内容推断格式：只有原生独有键、没有旧版独有键才是 V2。
pub fn provider_format(value: &Value, source: Option<Format>) -> Format {
    source.unwrap_or_else(|| {
        let has_any = |keys: &[&str]| keys.iter().any(|key| value.get(*key).is_some());
        if !has_any(&LEGACY_ONLY_KEYS) && has_any(&NATIVE_ONLY_KEYS) {
            Format::V2
        } else {
            Format::V1
        }
    })
}

/// 卡里存的通常就是条目本身，但老数据可能存了整份配置：按读取的规则从里面挑。
pub fn provider_fragment<'a>(
    id: &str,
    settings: &'a Value,
    source: Option<Format>,
) -> Result<(&'a Value, Format), AppError> {
    let Some(obj) = settings.as_object().filter(|obj| {
        ["$schema", "provider", "providers"]
            .iter()
            .any(|key| obj.contains_key(*key))
    }) else {
        return Ok((settings, provider_format(settings, source)));
    };
    let native = obj.get("providers").and_then(|providers| providers.get(id));
    let legacy = obj.get("provider").and_then(|providers| providers.get(id));
    match (native, legacy) {
        (Some(native), legacy) if legacy.is_none() || native_provider_problem(native).is_none() => {
            Ok((native, Format::V2))
        }
        (_, Some(legacy)) => Ok((legacy, Format::V1)),
        _ if obj.contains_key("provider") || obj.contains_key("providers") => {
            Err(AppError::localized(
                "provider.opencode.fragment_missing",
                format!("OpenCode 配置中没有供应商「{id}」"),
                format!("OpenCode config does not contain provider '{id}'"),
            ))
        }
        _ => Ok((settings, provider_format(settings, source))),
    }
}

/// 原生声明 OpenCode 会跳过的，拒收并指出字段。
pub fn validate_native_provider(id: &str, value: &Value) -> Result<(), AppError> {
    match native_provider_problem(value) {
        None => Ok(()),
        Some(path) if path.is_empty() => Err(AppError::localized(
            "provider.opencode.native_not_object",
            format!("OpenCode 原生供应商「{id}」必须是 JSON 对象"),
            format!("Native OpenCode provider '{id}' must be a JSON object"),
        )),
        Some(path) => Err(AppError::localized(
            "provider.opencode.native_invalid_field",
            format!("OpenCode 原生供应商「{id}」的字段 {path} 不符合 OpenCode V2 配置格式"),
            format!("Native OpenCode provider '{id}' has an invalid field: {path}"),
        )),
    }
}

pub(crate) fn validate(provider: &Provider) -> Result<(), AppError> {
    let (fragment, format) = provider_fragment(
        &provider.id,
        &provider.settings_config,
        stored_format(provider),
    )?;
    match format {
        Format::V2 => validate_native_provider(&provider.id, fragment),
        Format::V1 => {
            if !fragment.is_object()
                || (fragment.get("npm").is_none() && fragment.get("options").is_none())
            {
                return Err(AppError::localized(
                    "provider.opencode.invalid",
                    format!(
                        "OpenCode 供应商 {} 的配置必须是含 npm 或 options 的对象",
                        provider.id
                    ),
                    format!(
                        "OpenCode provider {} must be an object with npm or options",
                        provider.id
                    ),
                ));
            }
            Ok(())
        }
    }
}

pub(crate) fn patch<'a>(
    change: &'a EntryChange<'a>,
) -> JsoncEdit<impl Fn(&mut Value) -> Result<(), AppError> + 'a> {
    JsoncEdit {
        default_source: DEFAULT_SOURCE,
        max_bytes: None,
        edit: move |config: &mut Value| match &change.edit {
            Edit::Remove => {
                remove(config, change.id);
                Ok(())
            }
            Edit::Upsert(settings) => {
                let source = change.provider.and_then(stored_format);
                let (fragment, format) = provider_fragment(change.id, settings, source)?;
                upsert(config, change.id, fragment.clone(), format, change.mode)
            }
        },
    }
}

fn upsert(
    config: &mut Value,
    id: &str,
    fragment: Value,
    format: Format,
    mode: WriteMode,
) -> Result<(), AppError> {
    let key = format.key();
    match format {
        Format::V1 => {
            if config
                .get("providers")
                .and_then(|providers| providers.get(id))
                .is_some_and(|native| native_provider_problem(native).is_none())
            {
                return Err(AppError::localized(
                    "provider.opencode.native_shadows_legacy",
                    format!("OpenCode 配置中已有原生 V2 格式的供应商「{id}」，请重新导入供应商后再编辑"),
                    format!("OpenCode provider '{id}' has a native V2 declaration. Reload providers before editing it."),
                ));
            }
            // 新的 id 不能沿用别家的内置定义：没有 npm 和模型时，只有 live 里已有的同名
            // 供应商可以这样写。
            let has_npm = fragment
                .get("npm")
                .and_then(Value::as_str)
                .is_some_and(|npm| !npm.trim().is_empty());
            let has_models = fragment
                .get("models")
                .and_then(Value::as_object)
                .is_some_and(|models| !models.is_empty());
            let exists = config
                .get("provider")
                .and_then(|providers| providers.get(id))
                .is_some();
            if (!has_npm || !has_models) && !exists {
                return Err(AppError::localized(
                    "provider.opencode.custom_definition_required",
                    "新的 OpenCode 供应商标识需要填写 npm 包和至少一个模型；只有配置中已有的同名供应商可以沿用默认定义",
                    "A new OpenCode provider ID requires an npm package and at least one model; only an existing ID in the live config may inherit defaults",
                ));
            }
        }
        Format::V2 => validate_native_provider(id, &fragment)?,
    }
    match config.get(key) {
        Some(Value::Object(_)) => {}
        Some(_) => {
            // 不像 cc-switch 那样清空：段的形状不对就拒绝，免得丢掉用户配置。
            return Err(AppError::localized(
                "provider.opencode.providers_not_object",
                format!("OpenCode 配置中的 {key} 必须是 JSON 对象"),
                format!("OpenCode {key} must be a JSON object"),
            ));
        }
        None => config[key] = json!({}),
    }
    if mode == WriteMode::CreateOnly {
        if let Some(existing) = config[key].get(id) {
            if *existing != fragment {
                return Err(key_conflict(Tool::OpenCode, id));
            }
        }
    }
    config[key][id] = fragment;
    Ok(())
}

fn remove(config: &mut Value, id: &str) {
    let removed_legacy = config
        .get_mut("provider")
        .and_then(Value::as_object_mut)
        .is_some_and(|providers| providers.shift_remove(id).is_some());
    // 合法的原生声明盖住了旧版：两个都删，免得旧的冒出来；不合法的原生声明从来没生效，
    // 不碰。
    if let Some(providers) = config.get_mut("providers").and_then(Value::as_object_mut) {
        if !removed_legacy
            || providers
                .get(id)
                .is_some_and(|native| native_provider_problem(native).is_none())
        {
            providers.shift_remove(id);
        }
    }
}

// ---- V2 字段校验（照搬 cc-switch，约束来自 OpenCode v2.0.12） ----

pub fn native_provider_problem(value: &Value) -> Option<String> {
    let Some(obj) = value.as_object() else {
        return Some(String::new());
    };
    if let Some(key) = first_invalid_field(
        obj,
        &[
            ("name", Value::is_string),
            ("package", Value::is_string),
            ("canonical", Value::is_string),
            ("env", native_string_array),
            ("settings", native_provider_settings),
            ("headers", native_headers),
            ("body", Value::is_object),
        ],
    ) {
        return Some(key.to_string());
    }
    match obj.get("models") {
        None => None,
        Some(Value::Object(models)) => models.iter().find_map(|(id, model)| {
            native_model_problem(model).map(|key| match key {
                "" => format!("models.{id}"),
                key => format!("models.{id}.{key}"),
            })
        }),
        Some(_) => Some("models".to_string()),
    }
}

type NativeFieldCheck<'a> = (&'a str, fn(&Value) -> bool);

/// Optional means absent, not null. Unknown keys remain untouched at every level.
fn first_invalid_field<'a>(
    obj: &Map<String, Value>,
    fields: &[NativeFieldCheck<'a>],
) -> Option<&'a str> {
    fields
        .iter()
        .find(|(key, check)| obj.get(*key).is_some_and(|value| !check(value)))
        .map(|(key, _)| *key)
}

fn native_fields_valid(value: &Value, fields: &[NativeFieldCheck<'_>]) -> bool {
    value
        .as_object()
        .is_some_and(|obj| first_invalid_field(obj, fields).is_none())
}

fn native_string_array(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|values| values.iter().all(Value::is_string))
}

fn native_finite(value: &Value) -> bool {
    value.as_f64().is_some_and(f64::is_finite)
}

fn native_integer(value: &Value) -> bool {
    // Effect Schema.Int uses JavaScript's safe integer range, including 1.0.
    value
        .as_f64()
        .is_some_and(|n| n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0)
}

fn native_compaction(value: &Value) -> bool {
    value.as_object().is_some_and(|obj| {
        matches!(
            obj.get("type").and_then(Value::as_str),
            Some("summary" | "native")
        )
    })
}

fn native_provider_settings(value: &Value) -> bool {
    native_fields_valid(
        value,
        &[
            ("timeout", |v| v == &Value::Bool(false) || native_finite(v)),
            ("chunkTimeout", native_finite),
            ("compaction", native_compaction),
            ("transport", |v| {
                matches!(v.as_str(), Some("http" | "websocket"))
            }),
        ],
    )
}

fn native_headers(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|headers| headers.values().all(Value::is_string))
}

// Model/variant settings only constrain compaction; timeout and transport here
// are package-specific extensions, unlike provider settings.
fn native_model_settings(value: &Value) -> bool {
    native_fields_valid(value, &[("compaction", native_compaction)])
}

fn native_model_overlays(value: &Value) -> bool {
    native_fields_valid(
        value,
        &[
            ("settings", native_model_settings),
            ("headers", native_headers),
            ("body", Value::is_object),
        ],
    )
}

/// The first invalid field of a model; an empty name means it is not an object.
fn native_model_problem(value: &Value) -> Option<&'static str> {
    let Some(obj) = value.as_object() else {
        return Some("");
    };
    first_invalid_field(
        obj,
        &[
            ("modelID", Value::is_string),
            ("family", Value::is_string),
            ("name", Value::is_string),
            ("package", Value::is_string),
            ("disabled", Value::is_boolean),
            ("settings", native_model_settings),
            ("headers", native_headers),
            ("body", Value::is_object),
            ("compatibility", native_compatibility),
            ("capabilities", |v| {
                v.as_object().is_some_and(|obj| {
                    obj.get("tools").is_some_and(Value::is_boolean)
                        && obj.get("input").is_some_and(native_string_array)
                        && obj.get("output").is_some_and(native_string_array)
                })
            }),
            ("variants", |v| {
                v.as_array().is_some_and(|variants| {
                    variants.iter().all(|variant| {
                        variant.get("id").is_some_and(Value::is_string)
                            && native_model_overlays(variant)
                    })
                })
            }),
            ("cost", |v| match v.as_array() {
                Some(costs) => costs.iter().all(native_cost),
                None => native_cost(v),
            }),
            ("limit", |v| {
                native_fields_valid(
                    v,
                    &[
                        ("context", native_integer),
                        ("input", native_integer),
                        ("output", native_integer),
                    ],
                )
            }),
        ],
    )
}

fn native_compatibility(value: &Value) -> bool {
    native_fields_valid(
        value,
        &[
            ("reasoningField", Value::is_string),
            ("requireReasoning", Value::is_boolean),
            ("maxTokensField", |v| {
                matches!(v.as_str(), Some("max_completion_tokens" | "max_tokens"))
            }),
            ("requireFinishReason", Value::is_boolean),
            ("requireAssistantAfterTool", Value::is_boolean),
            ("supportsPromptCacheKey", Value::is_boolean),
        ],
    )
}

fn native_cost(value: &Value) -> bool {
    value.get("input").is_some_and(native_finite)
        && value.get("output").is_some_and(native_finite)
        && native_fields_valid(
            value,
            &[
                ("tier", |v| {
                    v.as_object().is_some_and(|obj| {
                        obj.get("type").and_then(Value::as_str) == Some("context")
                            && obj.get("size").is_some_and(native_integer)
                    })
                }),
                ("cache", |v| {
                    native_fields_valid(v, &[("read", native_finite), ("write", native_finite)])
                }),
            ],
        )
}
