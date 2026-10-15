//! Tool-wide preferences, edited separately from provider credentials and routing.
//! JSON is the editor interchange format; native TOML/YAML/JSONC and untouched comments survive.
use crate::{
    live::{
        engine::{digest, read_current, LiveFile},
        floor,
        patch::{
            json::JsonPatch, jsonc::JsoncDocument, toml::TomlPatch, Guarded, KeyPath, LivePatch,
            WholeFile,
        },
    },
    operation::{AppWrite, FileChange, OperationReport},
    state::PendingTarget,
    AppError, Env, Tool,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommonConfig {
    pub config: Value,
    pub paths: Vec<PathBuf>,
    pub revision: String,
    pub format: String,
    /// Detected file schema, not a claim about the installed executable version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}
fn paths(env: &Env, tool: Tool) -> Vec<PathBuf> {
    match tool {
        Tool::Claude => vec![env.claude_settings_path()],
        Tool::Codex => vec![env.codex_config_path()],
        Tool::Gemini => vec![env.gemini_settings_path()],
        Tool::GrokBuild => vec![env.grok_config_path()],
        Tool::ClaudeDesktop => vec![
            env.claude_desktop_config_path(),
            env.claude_desktop_3p_config_path(),
        ],
        Tool::OpenCode => vec![env.opencode_config_path()],
        Tool::OpenClaw => vec![env.openclaw_config_path()],
        Tool::Hermes => vec![env.hermes_config_path()],
        Tool::Pi => vec![env.pi_agent_dir.join("settings.json")],
        Tool::Mcode => vec![env.mcode_config_path()],
    }
}
fn format(tool: Tool) -> &'static str {
    match tool {
        Tool::Codex | Tool::GrokBuild => "TOML",
        Tool::Hermes | Tool::Mcode => "YAML",
        Tool::OpenCode | Tool::OpenClaw => "JSONC",
        _ => "JSON",
    }
}
fn invalid() -> AppError {
    AppError::Config("通用配置必须是有效对象 / Common configuration must be a valid object".into())
}
fn parse(tool: Tool, bytes: &[u8]) -> Result<Value, AppError> {
    let source = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    if source.trim().is_empty() {
        return Ok(json!({}));
    }
    let result = match format(tool) {
        "TOML" => serde_json::to_value(source.parse::<toml::Value>().map_err(|_| invalid())?)
            .map_err(|_| invalid())?,
        "YAML" => crate::coexist::yaml_sections::Document::parse(source)?.value,
        "JSONC" => JsoncDocument::parse(source)?.value().clone(),
        _ => serde_json::from_str(source).map_err(|_| invalid())?,
    };
    if !result.is_object() {
        return Err(invalid());
    }
    Ok(result)
}
fn protected(tool: Tool, keys: &[String]) -> bool {
    let key = keys.last().map(String::as_str).unwrap_or("");
    let at = |parts: &[&str]| keys.iter().map(String::as_str).eq(parts.iter().copied());
    match tool {
        Tool::Claude => {
            (keys.len() == 1 && floor::claude_floor_top(key))
                || (keys.len() == 2
                    && keys[0] == "env"
                    && (floor::claude_floor_env(key) || floor::claude_exclusive_env(key)))
        }
        Tool::Codex => {
            (keys.len() == 1
                && (floor::CODEX_FLOOR_TOP.contains(&key)
                    || floor::CODEX_EXCLUSIVE_TOP.contains(&key)
                    || ["model_providers", "profile", "profiles"].contains(&key)))
                || floor::CODEX_FLOOR_NESTED.iter().any(|p| at(p))
        }
        Tool::Gemini => floor::GEMINI_FLOOR_SETTINGS.iter().any(|p| at(p)),
        Tool::GrokBuild => keys.len() == 1 && ["model", "models", "api_key"].contains(&key),
        Tool::ClaudeDesktop => {
            keys.len() == 1 && ["deploymentMode", "enterpriseConfig"].contains(&key)
        }
        Tool::OpenCode => keys.len() == 1 && ["provider", "providers"].contains(&key),
        Tool::OpenClaw => at(&["models", "providers"]),
        Tool::Hermes => {
            keys.len() == 1 && ["model", "custom_providers", "providers"].contains(&key)
        }
        Tool::Pi => keys.len() == 1 && ["defaultProvider", "defaultModel"].contains(&key),
        Tool::Mcode => {
            (keys.len() == 1
                && [
                    "custom_provider",
                    "model",
                    "provider",
                    "defaultModel",
                    "defaultLightModel",
                ]
                .contains(&key))
                || at(&["memory", "captureModel"])
                || at(&["memory", "consolidationModel"])
        }
    }
}
fn preferences(tool: Tool, value: &Value) -> Value {
    fn visit(tool: Tool, value: &Value, keys: &mut Vec<String>) -> Value {
        if let Some(map) = value.as_object() {
            Value::Object(
                map.iter()
                    .filter_map(|(key, value)| {
                        keys.push(key.clone());
                        let keep = !protected(tool, keys);
                        let next = keep.then(|| visit(tool, value, keys));
                        keys.pop();
                        next.filter(|v| {
                            !v.as_object().is_some_and(|m| m.is_empty())
                                || value.as_object().is_some_and(|m| m.is_empty())
                        })
                        .map(|v| (key.clone(), v))
                    })
                    .collect(),
            )
        } else {
            value.clone()
        }
    }
    visit(tool, value, &mut Vec::new())
}
fn revision(paths: &[PathBuf], contents: &[Option<Vec<u8>>]) -> String {
    let entries: Vec<_> = paths
        .iter()
        .zip(contents)
        .map(|(p, b)| (p, digest(b.as_deref())))
        .collect();
    digest(Some(&serde_json::to_vec(&entries).expect("paths"))).unwrap()
}
pub fn read(env: &Env, tool: Tool) -> Result<CommonConfig, AppError> {
    let paths = paths(env, tool);
    let contents: Vec<_> = paths
        .iter()
        .map(|p| read_current(p))
        .collect::<Result<_, _>>()?;
    let mut parsed = contents
        .iter()
        .map(|b| parse(tool, b.as_deref().unwrap_or_default()))
        .collect::<Result<Vec<_>, _>>()?;
    // Desktop normal and enterprise configurations share changes, but unique existing fields remain intact.
    let root = parsed.remove(0);
    let variant = (tool == Tool::OpenCode
        && (root.get("providers").is_some()
            || root.get("permissions").is_some()
            || root.get("update").is_some()
            || root.pointer("/compaction/buffer").is_some()
            || root.pointer("/compaction/keep").is_some()
            || root
                .get("$schema")
                .and_then(Value::as_str)
                .is_some_and(|s| s.contains("/v2/"))))
    .then(|| "opencode-v2".to_string());
    let config = preferences(tool, &root);
    let revision = revision(&paths, &contents);
    Ok(CommonConfig {
        config,
        paths,
        revision,
        format: format(tool).into(),
        variant,
    })
}
fn diff(
    before: Option<&Value>,
    after: Option<&Value>,
    key: KeyPath,
    out: &mut Vec<(KeyPath, Option<Value>)>,
) {
    if before == after {
        return;
    }
    if before.is_some_and(Value::is_object) && after.is_some_and(Value::is_object) {
        let keys: BTreeSet<_> = before
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|o| o.keys())
            .chain(
                after
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|o| o.keys()),
            )
            .collect();
        for child in keys {
            diff(
                before.and_then(|v| v.get(child)),
                after.and_then(|v| v.get(child)),
                key.child(child),
                out,
            );
        }
    } else {
        out.push((key, after.cloned()));
    }
}
// Apply only edits visible in the common editor, retaining hidden connection fields.
fn merge_common(
    live: Option<&Value>,
    before: Option<&Value>,
    after: Option<&Value>,
) -> Option<Value> {
    if before == after {
        return live.cloned();
    }
    if before.is_none_or(Value::is_object)
        && after.is_none_or(Value::is_object)
        && live.is_none_or(Value::is_object)
    {
        let mut merged = live.and_then(Value::as_object).cloned().unwrap_or_default();
        let keys: BTreeSet<_> = before
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|m| m.keys())
            .chain(
                after
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|m| m.keys()),
            )
            .collect();
        for key in keys {
            match merge_common(
                live.and_then(|v| v.get(key)),
                before.and_then(|v| v.get(key)),
                after.and_then(|v| v.get(key)),
            ) {
                Some(value) => {
                    merged.insert(key.clone(), value);
                }
                None => {
                    merged.remove(key);
                }
            }
        }
        if after.is_none() && merged.is_empty() {
            None
        } else {
            Some(Value::Object(merged))
        }
    } else {
        after.cloned()
    }
}
fn validate_options(tool: Tool, config: &Value, before: &Value) -> Result<(), AppError> {
    if tool == Tool::Codex
        && config.get("tool_output_token_limit") != before.get("tool_output_token_limit")
        && config
            .get("tool_output_token_limit")
            .is_some_and(|v| !v.is_u64())
    {
        return Err(AppError::Config(
            "工具输出预算必须是非负整数 / Tool output budget must be a non-negative integer".into(),
        ));
    }
    let booleans: &[&str] = match tool {
        Tool::Codex => &["/features/multi_agent", "/features/shell_snapshot"],
        Tool::Gemini => &[
            "/general/enableAutoUpdate",
            "/general/vimMode",
            "/general/checkpointing/enabled",
        ],
        _ => &[],
    };
    for path in booleans {
        if config.pointer(path) == before.pointer(path) {
            continue;
        }
        let mut prefix = String::new();
        for key in path.trim_start_matches('/').split('/') {
            if config.pointer(&prefix).is_some_and(|v| !v.is_object()) {
                return Err(invalid());
            }
            prefix.push('/');
            prefix.push_str(key);
        }
        if config.pointer(path).is_some_and(|v| !v.is_boolean()) {
            return Err(invalid());
        }
    }
    if tool == Tool::Claude {
        for key in ["/attribution/commit", "/attribution/pr"] {
            if config.pointer(key).is_some_and(|v| !v.is_string()) {
                return Err(invalid());
            }
        }
        if config.get("attribution").is_some_and(|v| !v.is_object()) {
            return Err(invalid());
        }
    }
    if tool == Tool::Codex
        && config.pointer("/tui/notifications").is_some_and(|v| {
            !v.is_boolean() && !v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
        })
    {
        return Err(invalid());
    }
    if tool == Tool::OpenCode {
        if config.get("autoupdate") != before.get("autoupdate")
            && config
                .get("autoupdate")
                .is_some_and(|v| !v.is_boolean() && v.as_str() != Some("notify"))
        {
            return Err(invalid());
        }
        if config.get("share") != before.get("share")
            && config
                .get("share")
                .is_some_and(|v| !matches!(v.as_str(), Some("auto" | "manual" | "disabled")))
        {
            return Err(invalid());
        }
    }
    Ok(())
}
pub fn save(
    env: &Env,
    tool: Tool,
    expected: &str,
    config: Value,
) -> Result<OperationReport, AppError> {
    if !config.is_object() {
        return Err(invalid());
    }
    // Protected keys may not be smuggled back through JSON or a malformed parent container.
    fn validate(tool: Tool, value: &Value, path: &mut Vec<String>) -> Result<(), AppError> {
        if protected(tool, path) {
            return Err(AppError::Config(
                "连接字段请在供应商配置中编辑 / Edit connection fields in the provider editor"
                    .into(),
            ));
        }
        if let Some(map) = value.as_object() {
            for (key, value) in map {
                path.push(key.clone());
                validate(tool, value, path)?;
                path.pop();
            }
        }
        Ok(())
    }
    validate(tool, &config, &mut Vec::new())?;
    if tool == Tool::Claude {
        crate::switch::claude::validate(&crate::Provider::new("common", "Common", config.clone()))?;
    }
    let _client_lock = if tool == Tool::Mcode {
        Some(crate::coexist::mcode::DirLock::acquire(
            &env.mcode_config_path(),
        )?)
    } else {
        None
    };
    let commit = |target: &PendingTarget| crate::switch::commit_target(env, tool, target);
    let transaction = AppWrite::begin(env.device_store(), tool.as_str(), &commit)?;
    let current = read(env, tool)?;
    if current.revision != expected {
        return Err(AppError::Conflict(
            "客户端配置已变化，请重新载入 / Client configuration changed; reload before saving"
                .into(),
        ));
    }
    validate_options(tool, &config, &current.config)?;
    crate::config_shortcuts::common(tool, &current.config, &config)?;
    let mut patches = Vec::new();
    for path in &current.paths {
        let before = read_current(path)?;
        let value = parse(tool, before.as_deref().unwrap_or_default())?;
        let merged =
            merge_common(Some(&value), Some(&current.config), Some(&config)).ok_or_else(invalid)?;
        let mut changes = Vec::new();
        diff(Some(&value), Some(&merged), KeyPath::root(), &mut changes);
        let mut patch = JsonPatch::default();
        for (key, value) in &changes {
            if let Some(value) = value {
                patch.set.push((key.clone(), value.clone()));
            } else {
                patch.remove.push(key.clone());
            }
        }
        // Replacing a parent must never erase hidden connection fields.
        if preserve_protected(tool, &value) != preserve_protected(tool, &merged) {
            return Err(AppError::Config(
                "通用设置不能覆盖连接字段 / Common settings cannot overwrite connection fields"
                    .into(),
            ));
        }
        let source =
            std::str::from_utf8(before.as_deref().unwrap_or_default()).map_err(|_| invalid())?;
        let output = match format(tool) {
            "TOML" => {
                let mut patch = TomlPatch::default();
                for (key, value) in &changes {
                    if let Some(value) = value {
                        let fragment =
                            toml::to_string(&json!({"v":value})).map_err(|_| invalid())?;
                        let mut doc = fragment
                            .parse::<toml_edit::DocumentMut>()
                            .map_err(|_| invalid())?;
                        patch
                            .set
                            .push((key.clone(), doc.remove("v").ok_or_else(invalid)?));
                    } else {
                        patch.remove.push(key.clone());
                    }
                }
                patch.apply(path, before.as_deref())?
            }
            "YAML" => crate::coexist::yaml_sections::Document::parse(source)?
                .replace_all(&merged)?
                .into_bytes(),
            "JSONC" => {
                let mut doc = JsoncDocument::parse(if source.trim().is_empty() {
                    "{}"
                } else {
                    source
                })?;
                doc.apply(&merged)?;
                doc.validated_source()?.into_bytes()
            }
            _ => patch.apply(path, before.as_deref())?,
        };
        if parse(tool, &output)? != merged {
            return Err(invalid());
        }
        patches.push((
            LiveFile::private(path.clone()),
            Guarded {
                expected_pre: digest(before.as_deref()),
                then: WholeFile::Write(output),
            },
        ));
    }
    let edits: Vec<_> = patches
        .iter()
        .map(|(file, patch)| FileChange {
            file: file.clone(),
            patch,
        })
        .collect();
    transaction.run("common-config", &edits, PendingTarget::pointer(None))
}
fn preserve_protected(tool: Tool, value: &Value) -> Value {
    fn visit(
        tool: Tool,
        value: &Value,
        keys: &mut Vec<String>,
        result: &mut serde_json::Map<String, Value>,
    ) {
        if protected(tool, keys) {
            result.insert(keys.join("/"), value.clone());
            return;
        }
        if let Some(map) = value.as_object() {
            for (key, value) in map {
                keys.push(key.clone());
                visit(tool, value, keys, result);
                keys.pop();
            }
        }
    }
    let mut result = serde_json::Map::new();
    visit(tool, value, &mut Vec::new(), &mut result);
    Value::Object(result)
}
