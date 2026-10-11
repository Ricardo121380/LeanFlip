//! Claude Desktop gateway profiles and their activation metadata.
use crate::{
    env::Env,
    error::AppError,
    live::{
        engine::{read_current, LiveFile},
        floor,
        patch::{
            json::{self as json_patch, ClearScope, JsonPatch},
            KeyPath, LivePatch, LiveWriteError,
        },
    },
    operation::{AppWrite, FileChange, OperationReport},
    provider::{Provider, Tool},
    state::PendingTarget,
};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
pub const PROFILE_ID: &str = "4ea69f50-5b19-4bd0-9f3e-24193c845a97";
pub const PROFILE_NAME: &str = "LeanFlip";
pub const CLAUDE_ROUTE_PREFIX: &str = "claude-";
pub const ANTHROPIC_CLAUDE_ROUTE_PREFIX: &str = "anthropic/claude-";
pub const ONE_M_CONTEXT_MARKER: &str = "[1m]";
pub fn profile_path(env: &Env) -> PathBuf {
    env.claude_desktop_library_dir()
        .join(format!("{PROFILE_ID}.json"))
}
pub fn meta_path(env: &Env) -> PathBuf {
    env.claude_desktop_library_dir().join("_meta.json")
}
pub fn is_official(provider: &Provider) -> bool {
    provider.is_official_category()
}
pub fn is_claude_safe_model_id(id: &str) -> bool {
    let name = id.trim().to_ascii_lowercase();
    let name = name.strip_prefix("anthropic/").unwrap_or(&name);
    let Some(name) = name.strip_prefix("claude-") else {
        return false;
    };
    ["sonnet-", "haiku-", "opus-", "fable-"]
        .iter()
        .any(|family| {
            name.strip_prefix(family).is_some_and(|tail| {
                !tail.is_empty()
                    && tail
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
            })
        })
}
fn invalid(field: &str) -> AppError {
    AppError::Config(format!(
        "Claude Desktop 直连配置无效 / Invalid desktop gateway configuration: {field}"
    ))
}
fn profile(provider: &Provider) -> Result<Option<Map<String, Value>>, AppError> {
    if is_official(provider) {
        return Ok(None);
    }
    let meta = &provider.meta.extra;
    if !provider.settings_config.is_object() {
        return Err(invalid("settings"));
    }
    if meta.get("claudeDesktopMode").and_then(Value::as_str) == Some("proxy")
        || meta.get("isFullUrl").and_then(Value::as_bool) == Some(true)
    {
        return Err(invalid("direct mode required"));
    }
    if meta
        .get("apiFormat")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty() && s != "anthropic")
        || meta
            .get("providerType")
            .and_then(Value::as_str)
            .is_some_and(|s| ["github_copilot", "codex_oauth", "xai_oauth"].contains(&s))
    {
        return Err(invalid("Anthropic Messages API required"));
    }
    let field = |key: &str| {
        provider
            .settings_config
            .get("env")
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid(key))
    };
    let mut output=json!({"inferenceProvider":"gateway","inferenceGatewayBaseUrl":field("ANTHROPIC_BASE_URL")?,"inferenceGatewayApiKey":field("ANTHROPIC_AUTH_TOKEN")?,"inferenceGatewayAuthScheme":"bearer","disableDeploymentModeChooser":true,"coworkEgressAllowedHosts":["*"]}).as_object().unwrap().clone();
    let mut models = BTreeModels::new();
    if let Some(routes) = meta.get("claudeDesktopModelRoutes") {
        let routes = routes.as_object().ok_or_else(|| invalid("model routes"))?;
        for (name, options) in routes {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            if !is_claude_safe_model_id(name) {
                return Err(invalid("model name"));
            }
            if options
                .get("model")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty() && s.trim() != name)
            {
                return Err(invalid("model mapping requires a proxy"));
            }
            let label = options
                .get("labelOverride")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let large = options.get("supports1m").and_then(Value::as_bool) == Some(true);
            let mut item = Value::String(name.into());
            if label.is_some() || large {
                item = json!({"name":name});
                if let Some(label) = label {
                    item["labelOverride"] = json!(label);
                }
                if large {
                    item["supports1m"] = json!(true);
                }
            }
            models.insert(name.to_string(), item);
        }
    }
    if !models.is_empty() {
        output.insert(
            "inferenceModels".into(),
            Value::Array(models.into_values().collect()),
        );
    }
    Ok(Some(output))
}
type BTreeModels = std::collections::BTreeMap<String, Value>;
pub fn validate(provider: &Provider) -> Result<(), AppError> {
    profile(provider).map(|_| ())
}
struct DesktopPatch {
    kind: DesktopEdit,
}
enum DesktopEdit {
    Mode(bool, bool),
    Profile(Option<Map<String, Value>>),
    Index(bool),
}
impl LivePatch for DesktopPatch {
    fn apply(&self, path: &Path, before: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let (mut value, style) = json_patch::parse(path, before)?;
        let mut edit = JsonPatch::default();
        match &self.kind {
            DesktopEdit::Mode(active, legacy) => {
                edit.set.push((
                    KeyPath::new(&["deploymentMode"]),
                    json!(if *active { "3p" } else { "1p" }),
                ));
                if *legacy && !active {
                    for key in [
                        "inferenceProvider",
                        "inferenceGatewayBaseUrl",
                        "inferenceGatewayApiKey",
                        "inferenceGatewayAuthScheme",
                        "disableDeploymentModeChooser",
                    ] {
                        edit.remove.push(KeyPath::new(&["enterpriseConfig", key]));
                    }
                }
            }
            DesktopEdit::Profile(fields) => {
                edit.clear.push(ClearScope {
                    parent: KeyPath::root(),
                    is_floor: floor::desktop_profile_floor,
                });
                if let Some(fields) = fields {
                    for (key, value) in fields {
                        let destination = if floor::DESKTOP_PROFILE_SEED.contains(&key.as_str()) {
                            &mut edit.seed
                        } else {
                            &mut edit.set
                        };
                        destination.push((KeyPath::root().child(key), value.clone()));
                    }
                }
            }
            DesktopEdit::Index(active) => {
                let mut entries = match value.get("entries") {
                    Some(Value::Array(entries)) => entries.clone(),
                    None => Vec::new(),
                    _ => {
                        return Err(LiveWriteError::Shape {
                            path: path.into(),
                            key_path: KeyPath::new(&["entries"]),
                            expected: "数组",
                        })
                    }
                };
                let position = entries
                    .iter()
                    .position(|v| v.get("id").and_then(Value::as_str) == Some(PROFILE_ID));
                if *active {
                    if let Some(index) = position {
                        entries[index]["name"] = json!(PROFILE_NAME);
                    } else {
                        entries.push(json!({"id":PROFILE_ID,"name":PROFILE_NAME}));
                    }
                    edit.set
                        .push((KeyPath::new(&["appliedId"]), json!(PROFILE_ID)));
                } else {
                    entries.retain(|v| v.get("id").and_then(Value::as_str) != Some(PROFILE_ID));
                    if value.get("appliedId").and_then(Value::as_str) == Some(PROFILE_ID) {
                        if let Some(id) = entries
                            .iter()
                            .find_map(|v| v.get("id").and_then(Value::as_str))
                        {
                            edit.set.push((KeyPath::new(&["appliedId"]), json!(id)));
                        } else {
                            edit.remove.push(KeyPath::new(&["appliedId"]));
                        }
                    }
                }
                if *active || value.get("entries").is_some() {
                    edit.set
                        .push((KeyPath::new(&["entries"]), Value::Array(entries)));
                }
            }
        }
        edit.apply_to(path, &mut value)?;
        if matches!(self.kind, DesktopEdit::Mode(false, true))
            && value
                .get("enterpriseConfig")
                .and_then(Value::as_object)
                .is_some_and(|m| m.is_empty())
        {
            value
                .as_object_mut()
                .unwrap()
                .shift_remove("enterpriseConfig");
        }
        json_patch::serialize(path, &value, &style)
    }
}
pub fn write(
    env: &Env,
    target: &Provider,
    op: &str,
    pending: PendingTarget,
) -> Result<OperationReport, AppError> {
    let profile = profile(target)?;
    let active = profile.is_some();
    let plans = [
        (
            env.claude_desktop_config_path(),
            DesktopEdit::Mode(active, false),
            false,
        ),
        (
            env.claude_desktop_3p_config_path(),
            DesktopEdit::Mode(active, true),
            false,
        ),
        (profile_path(env), DesktopEdit::Profile(profile), true),
        (meta_path(env), DesktopEdit::Index(active), false),
    ];
    let edits: Vec<_> = plans
        .into_iter()
        .filter(|(path, kind, _)| {
            active || !matches!(kind, DesktopEdit::Profile(_)) || path.exists()
        })
        .map(|(path, kind, private)| {
            (
                if private {
                    LiveFile::private(path)
                } else {
                    LiveFile::shared(path)
                },
                DesktopPatch { kind },
            )
        })
        .collect();
    let changes: Vec<_> = edits
        .iter()
        .map(|(file, patch)| FileChange {
            file: file.clone(),
            patch: patch as &dyn LivePatch,
        })
        .collect();
    let commit = |target: &PendingTarget| super::commit_target(env, Tool::ClaudeDesktop, target);
    AppWrite::begin(env.device_store(), Tool::ClaudeDesktop.as_str(), &commit)?
        .run(op, &changes, pending)
}
pub fn imported_card(env: &Env) -> Result<Option<Provider>, AppError> {
    let read = |path: &Path| -> Result<Value, AppError> {
        match read_current(path)? {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|e| AppError::json(path, e)),
            None => Ok(json!({})),
        }
    };
    let meta = read(&meta_path(env))?;
    let Some(id) = meta.get("appliedId").and_then(Value::as_str) else {
        return Ok(None);
    };
    // An imported profile identifier is a filename, never a path.
    if id.is_empty() || id.contains(['/', '\\']) || id == "." || id == ".." {
        return Err(invalid("profile identifier"));
    }
    let current = read(&env.claude_desktop_library_dir().join(format!("{id}.json")))?;
    let text = |field: &str| {
        current
            .get(field)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
    };
    let (Some(url), Some(token)) = (
        text("inferenceGatewayBaseUrl"),
        text("inferenceGatewayApiKey"),
    ) else {
        return Ok(None);
    };
    let mut provider = Provider::new(
        crate::import::IMPORTED_ID,
        "当前桌面连接 (Imported)",
        json!({"env":{"ANTHROPIC_BASE_URL":url,"ANTHROPIC_AUTH_TOKEN":token}}),
    );
    let mut routes = Map::new();
    for model in current
        .get("inferenceModels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(name) = model
            .as_str()
            .or_else(|| model.get("name").and_then(Value::as_str))
            .filter(|s| is_claude_safe_model_id(s))
        {
            let mut route = json!({"model":name});
            for key in ["labelOverride", "supports1m"] {
                if let Some(value) = model.get(key) {
                    route[key] = value.clone();
                }
            }
            routes.insert(name.into(), route);
        }
    }
    if !routes.is_empty() {
        provider
            .meta
            .extra
            .insert("claudeDesktopModelRoutes".into(), Value::Object(routes));
    }
    Ok(Some(provider))
}
