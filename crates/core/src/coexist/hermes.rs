//! Hermes（`~/.hermes/config.yaml`）：供应商在 `custom_providers:` 列表里，按 `name` 认。
//!
//! 只替换 `custom_providers:`（激活时还有 `model:`）这两个顶层段的文本，其余段和注释
//! 原样保留。Hermes v12+ 的 `providers:` 字典是 Hermes 自己 Web UI 管的覆盖层：只出现
//! 在字典里的名字不能改也不能删，导入时也跳过。移植自 cc-switch `hermes_config.rs`
//! （段替换、键名清洗、models 数组↔字典、合并磁盘上的未知字段照搬）。

use std::path::Path;

use serde_json::{Map, Value};

use super::{key_conflict, rejected, Edit, EntryChange, WriteMode};
use crate::error::AppError;
use crate::live::patch::{decode_utf8, LivePatch, LiveWriteError};
use crate::provider::{Provider, Tool};

const PROVIDER_SOURCE_FIELD: &str = "_cc_source";

/// 解析（先按「保留最后一个」去掉重复的顶层段，和 PyYAML 的语义一致）。
fn parse(raw: &str) -> Result<serde_yaml::Value, AppError> {
    if raw.trim().is_empty() {
        return Ok(serde_yaml::Value::Mapping(Default::default()));
    }
    let value: serde_yaml::Value = serde_yaml::from_str(&deduplicate_top_level_keys(raw))
        .map_err(|e| AppError::Config(format!("无法解析 Hermes 配置（YAML）: {e}")))?;
    match value {
        serde_yaml::Value::Null => Ok(serde_yaml::Value::Mapping(Default::default())),
        serde_yaml::Value::Mapping(_) => Ok(value),
        _ => Err(AppError::Config("Hermes 配置的根必须是映射".into())),
    }
}

pub(crate) fn entries(text: &str) -> Result<Map<String, Value>, AppError> {
    let config = parse(text)?;
    let mut map = Map::new();
    if let Some(seq) = config.get("custom_providers").and_then(|v| v.as_sequence()) {
        for item in seq {
            let Some(name) = item.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            let mut json_val = yaml_to_json(item)?;
            sanitize_hermes_provider_keys(&mut json_val);
            denormalize_provider_models_for_read(&mut json_val);
            map.insert(name.to_string(), json_val);
        }
    }
    Ok(map)
}

pub(crate) fn validate(provider: &Provider) -> Result<(), AppError> {
    if provider.id.trim().is_empty() || !provider.settings_config.is_object() {
        return Err(AppError::localized(
            "provider.hermes.invalid",
            "Hermes 供应商需要非空的名字，配置必须是对象",
            "A Hermes provider needs a non-empty name and an object config",
        ));
    }
    Ok(())
}

fn is_dict_only_provider(config: &serde_yaml::Value, name: &str) -> bool {
    let list_has = config
        .get("custom_providers")
        .and_then(|v| v.as_sequence())
        .is_some_and(|seq| {
            seq.iter()
                .any(|item| item.get("name").and_then(|n| n.as_str()) == Some(name))
        });
    if list_has {
        return false;
    }
    config
        .get("providers")
        .and_then(|v| v.as_mapping())
        .is_some_and(|m| {
            m.iter().any(|(k, v)| {
                let key_matches = k.as_str() == Some(name);
                let name_matches = v.get("name").and_then(|n| n.as_str()) == Some(name);
                (key_matches || name_matches) && v.is_mapping()
            })
        })
}

pub(crate) struct HermesPatch<'a> {
    pub change: &'a EntryChange<'a>,
}

impl HermesPatch<'_> {
    fn edit(&self, raw: &str) -> Result<String, AppError> {
        let change = self.change;
        let name = change.id;
        let config = parse(raw)?;
        if is_dict_only_provider(&config, name) {
            return Err(AppError::Config(format!(
                "供应商「{name}」由 Hermes 的 providers: 字典管理，请在 Hermes Web UI 里修改"
            )));
        }
        let mut providers: Vec<serde_yaml::Value> = match config.get("custom_providers") {
            None | Some(serde_yaml::Value::Null) => Vec::new(),
            Some(serde_yaml::Value::Sequence(seq)) => seq.clone(),
            Some(_) => {
                return Err(AppError::Config(
                    "Hermes 配置里的 custom_providers 不是列表，没有写入".into(),
                ))
            }
        };
        let is_ours = |p: &serde_yaml::Value| p.get("name").and_then(|n| n.as_str()) == Some(name);
        let mut out = raw.to_string();
        let mut first_model_id = None;
        match &change.edit {
            Edit::Remove => {
                let before = providers.len();
                providers.retain(|p| !is_ours(p));
                if providers.len() != before {
                    out = replace_yaml_section(
                        &out,
                        "custom_providers",
                        &serde_yaml::Value::Sequence(providers),
                    )?;
                }
            }
            Edit::Upsert(settings) => {
                let mut normalized = settings.clone();
                sanitize_hermes_provider_keys(&mut normalized);
                normalize_provider_models_for_write(&mut normalized);
                first_model_id = normalized
                    .get("models")
                    .and_then(Value::as_object)
                    .and_then(|obj| obj.keys().next())
                    .cloned();
                let mut entry = json_to_yaml(&normalized)?;
                if let serde_yaml::Value::Mapping(m) = &mut entry {
                    m.insert("name".into(), name.into());
                    match &first_model_id {
                        Some(model) => {
                            m.insert("model".into(), model.as_str().into());
                        }
                        None => {
                            m.remove("model");
                        }
                    }
                }
                match providers.iter_mut().find(|p| is_ours(p)) {
                    Some(existing) => {
                        // 磁盘上有、卡里没有的字段（Hermes 新加的、用户在 Hermes 里设的）留着。
                        if let (Some(existing_map), serde_yaml::Value::Mapping(new_map)) =
                            (existing.as_mapping(), &mut entry)
                        {
                            for (k, v) in existing_map {
                                new_map.entry(k.clone()).or_insert_with(|| v.clone());
                            }
                        }
                        if change.mode == WriteMode::CreateOnly && *existing != entry {
                            return Err(key_conflict(Tool::Hermes, name));
                        }
                        if *existing != entry {
                            *existing = entry;
                            out = replace_yaml_section(
                                &out,
                                "custom_providers",
                                &serde_yaml::Value::Sequence(providers),
                            )?;
                        }
                    }
                    None => {
                        providers.push(entry);
                        out = replace_yaml_section(
                            &out,
                            "custom_providers",
                            &serde_yaml::Value::Sequence(providers),
                        )?;
                    }
                }
            }
        }
        if change.activate {
            // 顶层 model:：provider 总是指向这家；声明了模型才改 default，否则保留原来的。
            let mut model = match config.get("model") {
                None | Some(serde_yaml::Value::Null) => serde_yaml::Mapping::new(),
                Some(serde_yaml::Value::Mapping(m)) => m.clone(),
                Some(_) => {
                    return Err(AppError::Config(
                        "Hermes 配置里的 model 不是映射，没有写入".into(),
                    ))
                }
            };
            if let Some(first) = &first_model_id {
                model.insert("default".into(), first.as_str().into());
            }
            model.insert("provider".into(), name.into());
            if config.get("model").and_then(|m| m.as_mapping()) != Some(&model) {
                out = replace_yaml_section(&out, "model", &serde_yaml::Value::Mapping(model))?;
            }
        }
        Ok(out)
    }
}

impl LivePatch for HermesPatch<'_> {
    fn apply(&self, path: &Path, pre: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let raw = match pre {
            Some(bytes) => decode_utf8(path, bytes)?,
            None => "",
        };
        self.edit(raw).map(String::into_bytes).map_err(rejected)
    }
}

fn yaml_to_json(yaml: &serde_yaml::Value) -> Result<Value, AppError> {
    let text = serde_yaml::to_string(yaml)
        .map_err(|e| AppError::Config(format!("YAML 序列化失败: {e}")))?;
    serde_yaml::from_str::<Value>(&text)
        .map_err(|e| AppError::Config(format!("YAML 转 JSON 失败: {e}")))
}

fn json_to_yaml(json: &Value) -> Result<serde_yaml::Value, AppError> {
    let text = serde_json::to_string(json).map_err(|source| AppError::JsonSerialize { source })?;
    serde_yaml::from_str(&text).map_err(|e| AppError::Config(format!("JSON 转 YAML 失败: {e}")))
}

// ---- 以下照搬 cc-switch hermes_config.rs ----

fn deduplicate_top_level_keys(raw: &str) -> String {
    use std::collections::HashMap;

    // Pass 1: locate every top-level key line as (key, byte offset).
    let mut sections: Vec<(&str, usize)> = Vec::new();
    let mut offset = 0;
    for line in raw.split('\n') {
        if is_top_level_key_line(line) {
            if let Some(colon_pos) = line.find(':') {
                sections.push((&line[..colon_pos], offset));
            }
        }
        offset += line.len() + 1;
    }

    let mut remaining: HashMap<&str, usize> = HashMap::new();
    for (key, _) in &sections {
        *remaining.entry(key).or_insert(0) += 1;
    }
    if remaining.values().all(|&count| count <= 1) {
        return raw.to_string();
    }

    // Pass 2: re-emit, dropping every section that has a later occurrence of
    // the same key. A section spans from its key line to the next top-level
    // key line (or EOF), matching find_yaml_section_range. Content before the
    // first section (comments, document markers) is always kept.
    let mut result = String::with_capacity(raw.len());
    let head_end = sections
        .first()
        .map(|&(_, start)| start)
        .unwrap_or(raw.len());
    result.push_str(&raw[..head_end]);

    for (i, &(key, start)) in sections.iter().enumerate() {
        let end = sections
            .get(i + 1)
            .map(|&(_, next_start)| next_start)
            .unwrap_or(raw.len());
        let count = remaining.get_mut(key).expect("key collected in pass 1");
        *count -= 1;
        if *count > 0 {
            log::warn!(
                "Hermes config: dropped duplicate top-level section '{key}' (keeping the last occurrence)"
            );
            continue;
        }
        result.push_str(&raw[start..end]);
    }

    result
}

// YAML Section-Level Replacement

/// Check if a line is a YAML top-level key (mapping key at column 0).
///
/// A top-level key line must:
/// - Start at column 0 (no leading whitespace)
/// - Not be empty or whitespace-only
/// - Not be a comment (starting with `#`)
/// - Not be a sequence item (starting with `-`)
/// - Contain `:` followed by space, tab, newline, or end-of-line
///
/// Lines may carry a trailing `\r` (CRLF files split on `\n`) or `\n`
/// (callers using `split_inclusive`); both count as end-of-line after the
/// colon. Rejecting `\r` here used to make every section lookup miss on
/// CRLF configs, turning section replacement into endless appends (#3633).
fn is_top_level_key_line(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    let first_char = line.as_bytes()[0];
    if first_char == b' ' || first_char == b'\t' || first_char == b'#' || first_char == b'-' {
        return false;
    }
    if let Some(colon_pos) = line.find(':') {
        let after_colon = &line[colon_pos + 1..];
        after_colon.is_empty() || after_colon.starts_with([' ', '\t', '\r', '\n'])
    } else {
        false
    }
}

/// Find the byte range of a top-level YAML section.
///
/// A YAML top-level key is a line that starts at column 0 (no leading
/// whitespace), is not a comment, and contains `:` after the key name.
///
/// Returns `(start_byte_inclusive, end_byte_exclusive)` or `None` if not found.
fn find_yaml_section_range(raw: &str, section_key: &str) -> Option<(usize, usize)> {
    let target = format!("{}:", section_key);
    let mut section_start = None;
    let mut offset = 0;

    for line in raw.split('\n') {
        if section_start.is_none() && is_top_level_key_line(line) && line.starts_with(&target) {
            // Verify exact match: after "key:" must be whitespace or EOL
            let after_target = &line[target.len()..];
            if after_target.is_empty()
                || after_target.starts_with(' ')
                || after_target.starts_with('\t')
                || after_target.starts_with('\r')
            {
                section_start = Some(offset);
            }
        } else if section_start.is_some() && is_top_level_key_line(line) {
            // Found the next top-level key — this is the end of our section
            return Some((section_start.unwrap(), offset));
        }
        offset += line.len() + 1; // +1 for the \n
    }

    // Section extends to end of file
    section_start.map(|start| (start, raw.len()))
}

/// Serialize a section key + value into a YAML fragment like:
///
/// ```yaml
/// model:
///   default: "anthropic/claude-opus-4-8"
///   provider: "openrouter"
/// ```
fn serialize_yaml_section(key: &str, value: &serde_yaml::Value) -> Result<String, AppError> {
    let mut section = serde_yaml::Mapping::new();
    section.insert(serde_yaml::Value::String(key.to_string()), value.clone());
    let yaml_str = serde_yaml::to_string(&serde_yaml::Value::Mapping(section))
        .map_err(|e| AppError::Config(format!("Failed to serialize YAML section '{key}': {e}")))?;
    Ok(yaml_str)
}

/// Remove every top-level section with the given key from raw YAML text.
/// Used to clean residual duplicates of a key after replacing its first
/// occurrence; safe values come from the keep-last healed read, so dropping
/// all on-disk copies here loses nothing.
fn remove_all_sections(raw: &str, section_key: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some((start, end)) = find_yaml_section_range(rest, section_key) {
        result.push_str(&rest[..start]);
        rest = &rest[end..];
    }
    result.push_str(rest);
    result
}

/// Replace a YAML section in raw text, or append it if not found.
fn replace_yaml_section(
    raw: &str,
    section_key: &str,
    value: &serde_yaml::Value,
) -> Result<String, AppError> {
    let serialized = serialize_yaml_section(section_key, value)?;

    if let Some((start, end)) = find_yaml_section_range(raw, section_key) {
        let mut result = String::with_capacity(raw.len());
        result.push_str(&raw[..start]);
        result.push_str(&serialized);
        // Drop duplicate sections of this key from the remainder — configs
        // written before the CRLF fix may carry several appended copies.
        let remainder = remove_all_sections(&raw[end..], section_key);
        // Ensure proper separation between sections
        if !serialized.ends_with('\n') && !remainder.is_empty() && !remainder.starts_with('\n') {
            result.push('\n');
        }
        result.push_str(&remainder);
        Ok(result)
    } else {
        // Section not found — append at end
        let mut result = raw.to_string();
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(&serialized);
        if !result.ends_with('\n') {
            result.push('\n');
        }
        Ok(result)
    }
}

// Backup & Cleanup

fn models_array_to_dict(array: Vec<serde_json::Value>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for item in array {
        let serde_json::Value::Object(mut obj) = item else {
            continue;
        };
        let Some(id) = obj
            .remove("id")
            .and_then(|v| v.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        map.insert(id, serde_json::Value::Object(obj));
    }
    serde_json::Value::Object(map)
}

/// Inverse of [`models_array_to_dict`]. Converts the YAML dict shape back to
/// the UI-friendly ordered array, re-injecting `id` as an object field.
fn models_dict_to_array(dict: serde_json::Map<String, serde_json::Value>) -> serde_json::Value {
    let mut out = Vec::with_capacity(dict.len());
    for (id, value) in dict {
        let mut obj = match value {
            serde_json::Value::Object(obj) => obj,
            serde_json::Value::Null => serde_json::Map::new(),
            other => {
                log::warn!("Unexpected Hermes model entry for '{id}': {other:?}, skipping");
                continue;
            }
        };
        obj.insert("id".to_string(), serde_json::Value::String(id));
        out.push(serde_json::Value::Object(obj));
    }
    serde_json::Value::Array(out)
}

/// Rewrite historical camelCase keys to Hermes' snake_case schema.
///
/// Older DeepLink import paths emitted `baseUrl` / `apiKey` / `apiMode` /
/// `maxTokens` / `contextLength`, which do not belong to Hermes'
/// `_VALID_CUSTOM_PROVIDER_FIELDS` set. Writing those raw to YAML silently
/// poisons `custom_providers:` entries. This sanitiser runs defensively on
/// every `set_provider` call so stored data heals on the next activation;
/// unknown keys pass through untouched to keep forward-compat with new
/// Hermes fields (e.g. `request_timeout_seconds`).
fn sanitize_hermes_provider_keys(config: &mut serde_json::Value) {
    const KEY_ALIASES: &[(&str, &str)] = &[
        ("baseUrl", "base_url"),
        ("apiKey", "api_key"),
        ("apiMode", "api_mode"),
        ("maxTokens", "max_tokens"),
        ("contextLength", "context_length"),
    ];
    // Legacy DeepLink emitted `api: "openai-completions"` which is neither a
    // Hermes field nor mappable to `api_mode`. `_cc_source` / `provider_key`
    // are UI-only markers injected on read — they must never reach YAML.
    const LEGACY_FIELDS_TO_DROP: &[&str] = &["api", PROVIDER_SOURCE_FIELD, "provider_key"];

    let Some(obj) = config.as_object_mut() else {
        return;
    };

    for (from, to) in KEY_ALIASES {
        if let Some(val) = obj.remove(*from) {
            // snake_case wins when both are present; stale camelCase is dropped.
            obj.entry((*to).to_string()).or_insert(val);
        }
    }

    for field in LEGACY_FIELDS_TO_DROP {
        obj.remove(*field);
    }
}

/// If `config.models` is a JSON array, convert it in-place to the dict shape.
/// No-op when `models` is absent or already a dict.
fn normalize_provider_models_for_write(config: &mut serde_json::Value) {
    let Some(obj) = config.as_object_mut() else {
        return;
    };
    let Some(models_val) = obj.get_mut("models") else {
        return;
    };
    if models_val.is_array() {
        let taken = std::mem::take(models_val);
        if let serde_json::Value::Array(arr) = taken {
            *models_val = models_array_to_dict(arr);
        }
    }
}

/// If `config.models` is a JSON dict, convert it in-place to the ordered array
/// shape. No-op when `models` is absent or already an array.
fn denormalize_provider_models_for_read(config: &mut serde_json::Value) {
    let Some(obj) = config.as_object_mut() else {
        return;
    };
    let Some(models_val) = obj.get_mut("models") else {
        return;
    };
    if models_val.is_object() {
        let taken = std::mem::take(models_val);
        if let serde_json::Value::Object(map) = taken {
            *models_val = models_dict_to_array(map);
        }
    }
}
