//! Claude Desktop 直连模式：一次切换改四个文件，作为**一个操作**提交（任一文件解析失败
//! 都不写；写到一半崩溃，下次操作或启动时按 pending 前滚或丢弃），指针在同一个操作里落定：
//!
//! 1. `Claude/claude_desktop_config.json`：`deploymentMode`（`3p` / `1p`）；
//! 2. `Claude-3p/claude_desktop_config.json`：同上；切回官方时顺带清掉旧版写进
//!    `enterpriseConfig` 的网关设置，空了就删掉整个 `enterpriseConfig`；
//! 3. `Claude-3p/configLibrary/<PROFILE_ID>.json`：网关地址、Key、模型列表（关键字段替换，
//!    策略键缺失时才写）；切回官方时文件保留、只清关键字段（用户在 Desktop 里对这个
//!    profile 改的设置下次还在，Key 从磁盘上清掉）；
//! 4. `Claude-3p/configLibrary/_meta.json`：登记 / 摘掉 CC Switch 的 profile，`appliedId`。
//!
//! 移植自 cc-switch `claude_desktop_config.rs` 的直连部分。和原版的差异：
//! - 没有本地路由（模型映射 / proxy 模式）：proxy 模式的卡直接拒绝；
//! - 官方卡按 `category == "official"` 认（原版按固定 id）；
//! - profile id 沿用原版的固定 UUID，从 cc-switch 迁过来时同一个 profile 原位接管。

use std::path::Path;

use serde_json::{json, Value};

use crate::env::Env;
use crate::error::AppError;
use crate::live::engine::LiveFile;
use crate::live::floor;
use crate::live::patch::json::{self as patch_json, ClearScope, JsonPatch};
use crate::live::patch::{KeyPath, LivePatch, LiveWriteError};
use crate::operation::{AppWrite, FileChange, OperationReport};
use crate::provider::{Provider, Tool};
use crate::state::PendingTarget;

/// CC Switch 的 profile id 和显示名（和 cc-switch 一致）。
pub const PROFILE_ID: &str = "00000000-0000-4000-8000-000000157210";
pub const PROFILE_NAME: &str = "CC Switch";

pub const CLAUDE_ROUTE_PREFIX: &str = "claude-";
pub const ANTHROPIC_CLAUDE_ROUTE_PREFIX: &str = "anthropic/claude-";
pub const ONE_M_CONTEXT_MARKER: &str = "[1m]";

/// 旧版写进 `claude_desktop_config.json` 的 `enterpriseConfig` 的网关设置。
const LEGACY_ENTERPRISE_GATEWAY_KEYS: &[&str] = &[
    "disableDeploymentModeChooser",
    "inferenceGatewayApiKey",
    "inferenceGatewayAuthScheme",
    "inferenceGatewayBaseUrl",
    "inferenceProvider",
];

pub fn profile_path(env: &Env) -> std::path::PathBuf {
    env.claude_desktop_library_dir()
        .join(format!("{PROFILE_ID}.json"))
}

pub fn meta_path(env: &Env) -> std::path::PathBuf {
    env.claude_desktop_library_dir().join("_meta.json")
}

pub fn is_official(provider: &Provider) -> bool {
    provider.is_official_category()
}

fn localized(key: &'static str, zh: impl Into<String>, en: impl Into<String>) -> AppError {
    AppError::localized(key, zh, en)
}

fn meta_str<'a>(provider: &'a Provider, key: &str) -> Option<&'a str> {
    provider.meta.extra.get(key).and_then(Value::as_str)
}

/// Desktop 的模型菜单只认 `claude-sonnet-*` 这类名字（fail-all：一个不合法整组拒收）。
pub fn is_claude_safe_model_id(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    if normalized.contains(ONE_M_CONTEXT_MARKER) {
        return false;
    }
    let Some(tail) = normalized
        .strip_prefix(ANTHROPIC_CLAUDE_ROUTE_PREFIX)
        .or_else(|| normalized.strip_prefix(CLAUDE_ROUTE_PREFIX))
    else {
        return false;
    };
    ["sonnet-", "opus-", "haiku-", "fable-"]
        .iter()
        .any(|prefix| {
            tail.strip_prefix(prefix)
                .is_some_and(|rest| !rest.is_empty())
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelSpec {
    name: String,
    label_override: Option<String>,
    supports_1m: bool,
}

fn model_json(spec: &ModelSpec) -> Value {
    if spec.supports_1m || spec.label_override.is_some() {
        let mut item = json!({ "name": spec.name });
        if let Some(label) = &spec.label_override {
            item["labelOverride"] = json!(label);
        }
        if spec.supports_1m {
            item["supports1m"] = json!(true);
        }
        item
    } else {
        Value::String(spec.name.clone())
    }
}

/// `meta.claudeDesktopModelRoutes`：直连模式只允许 Claude 官方模型名，不能映射。
fn model_specs(provider: &Provider) -> Result<Vec<ModelSpec>, AppError> {
    let Some(routes) = provider
        .meta
        .extra
        .get("claudeDesktopModelRoutes")
        .and_then(Value::as_object)
    else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for (route_id, route) in routes {
        let route_id = route_id.trim();
        if route_id.is_empty() {
            continue;
        }
        if !is_claude_safe_model_id(route_id) {
            return Err(localized(
                "claude_desktop.provider.route_invalid",
                format!("Claude Desktop 直连模型必须使用 claude-* 或 anthropic/claude-* 名称: {route_id}"),
                format!("Claude Desktop direct model must use a claude-* or anthropic/claude-* name: {route_id}"),
            ));
        }
        let upstream = route
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if !upstream.is_empty() && upstream != route_id {
            return Err(localized(
                "claude_desktop.provider.direct_mapping_unsupported",
                format!("Claude Desktop 直连模式不能映射模型: {route_id} -> {upstream}（ccswitch-lite 没有本地路由）"),
                format!("Claude Desktop direct mode cannot map models: {route_id} -> {upstream} (ccswitch-lite has no local routing)"),
            ));
        }
        result.push(ModelSpec {
            name: route_id.to_string(),
            label_override: route
                .get("labelOverride")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string),
            supports_1m: route
                .get("supports1m")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name).then(b.supports_1m.cmp(&a.supports_1m)));
    result.dedup_by(|a, b| a.name == b.name);
    Ok(result)
}

fn env_str<'a>(provider: &'a Provider, key: &str) -> Option<&'a str> {
    provider
        .settings_config
        .get("env")
        .and_then(|env| env.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// 校验直连卡；通过时返回要写进 profile 的网关字段。
pub fn validate(provider: &Provider) -> Result<(), AppError> {
    gateway_profile(provider).map(|_| ())
}

fn gateway_profile(provider: &Provider) -> Result<Option<Value>, AppError> {
    if is_official(provider) {
        return Ok(None);
    }
    if !provider.settings_config.is_object() {
        return Err(localized(
            "claude_desktop.provider.settings_not_object",
            "Claude Desktop 直连供应商配置必须是 JSON 对象",
            "Claude Desktop direct provider configuration must be a JSON object",
        ));
    }
    if meta_str(provider, "claudeDesktopMode") == Some("proxy") {
        return Err(localized(
            "claude_desktop.provider.mode_unsupported",
            "这张卡是 Claude Desktop 本地路由（模型映射）模式，ccswitch-lite 只支持直连",
            "This card uses Claude Desktop proxy (model mapping) mode; ccswitch-lite only supports direct mode",
        ));
    }
    if let Some(format) = meta_str(provider, "apiFormat").filter(|f| !f.trim().is_empty()) {
        if format != "anthropic" {
            return Err(localized(
                "claude_desktop.provider.api_format_unsupported",
                "Claude Desktop 直连只支持原生 Anthropic Messages API",
                "Claude Desktop direct mode only supports the native Anthropic Messages API",
            ));
        }
    }
    if matches!(
        meta_str(provider, "providerType"),
        Some("github_copilot" | "codex_oauth" | "xai_oauth")
    ) {
        return Err(localized(
            "claude_desktop.provider.type_unsupported",
            "Claude Desktop 直连模式不支持需要本地代理转换的供应商",
            "Claude Desktop direct mode does not support providers that require local proxy conversion",
        ));
    }
    if provider
        .meta
        .extra
        .get("isFullUrl")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Err(localized(
            "claude_desktop.provider.full_url_unsupported",
            "Claude Desktop 直连模式不支持完整 URL 端点配置",
            "Claude Desktop direct mode does not support full URL endpoint configuration",
        ));
    }
    let specs = model_specs(provider)?;
    let base_url = env_str(provider, "ANTHROPIC_BASE_URL").ok_or_else(|| {
        localized(
            "claude_desktop.provider.base_url_missing",
            "Claude Desktop 直连供应商缺少 ANTHROPIC_BASE_URL",
            "Claude Desktop direct provider is missing ANTHROPIC_BASE_URL",
        )
    })?;
    let api_key = env_str(provider, "ANTHROPIC_AUTH_TOKEN").ok_or_else(|| {
        localized(
            "claude_desktop.provider.auth_token_missing",
            "Claude Desktop 直连供应商缺少 ANTHROPIC_AUTH_TOKEN（Bearer Token）",
            "Claude Desktop direct provider is missing ANTHROPIC_AUTH_TOKEN (Bearer Token)",
        )
    })?;
    let mut profile = json!({
        "coworkEgressAllowedHosts": ["*"],
        "disableDeploymentModeChooser": true,
        "inferenceGatewayApiKey": api_key,
        "inferenceGatewayAuthScheme": "bearer",
        "inferenceGatewayBaseUrl": base_url,
        "inferenceProvider": "gateway",
    });
    if !specs.is_empty() {
        profile["inferenceModels"] = Value::Array(specs.iter().map(model_json).collect());
    }
    Ok(Some(profile))
}

fn deployment_mode_patch(mode: &str) -> JsonPatch {
    JsonPatch {
        set: vec![(KeyPath::new(&["deploymentMode"]), json!(mode))],
        ..JsonPatch::default()
    }
}

/// 清完旧网关设置后，`enterpriseConfig` 空了就整个删掉。
struct DropEmptyEnterpriseConfig(JsonPatch);

impl LivePatch for DropEmptyEnterpriseConfig {
    fn apply(&self, path: &Path, pre: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let (mut doc, style) = patch_json::parse(path, pre)?;
        self.0.apply_to(path, &mut doc)?;
        if let Some(obj) = doc.as_object_mut() {
            if obj
                .get("enterpriseConfig")
                .and_then(Value::as_object)
                .is_some_and(|enterprise| enterprise.is_empty())
            {
                obj.shift_remove("enterpriseConfig");
            }
        }
        patch_json::serialize(path, &doc, &style)
    }
}

/// profile 的补丁：关键字段整组替换，策略键缺失时才写。`profile` 为 `None` 只清关键字段。
fn profile_patch(profile: Option<Value>) -> JsonPatch {
    let mut patch = JsonPatch {
        clear: vec![ClearScope {
            parent: KeyPath::root(),
            is_floor: floor::desktop_profile_floor,
        }],
        ..JsonPatch::default()
    };
    if let Some(Value::Object(fields)) = profile {
        for (key, value) in fields {
            let key_path = KeyPath::new(&[&key]);
            if floor::DESKTOP_PROFILE_SEED.contains(&key.as_str()) {
                patch.seed.push((key_path, value));
            } else {
                patch.set.push((key_path, value));
            }
        }
    }
    patch
}

/// `_meta.json`：`entries` 里登记 CC Switch 的 profile，`appliedId` 指向生效的那个。
/// 已有条目原位更新，不挪位置；摘掉时 `appliedId` 换成剩下的第一个（没有就删掉）。
struct MetaPatch {
    applied: bool,
}

impl LivePatch for MetaPatch {
    fn apply(&self, path: &Path, pre: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let (mut doc, style) = patch_json::parse(path, pre)?;
        let obj = doc.as_object_mut().expect("parse guarantees an object");
        let not_an_array = || LiveWriteError::Shape {
            path: path.to_path_buf(),
            key_path: KeyPath::new(&["entries"]),
            expected: "数组",
        };
        let is_ours = |entry: &Value| entry.get("id").and_then(Value::as_str) == Some(PROFILE_ID);
        if self.applied {
            let entries = obj
                .entry("entries")
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(not_an_array)?;
            match entries.iter_mut().find(|entry| is_ours(entry)) {
                Some(Value::Object(entry)) => {
                    entry.insert("name".to_string(), json!(PROFILE_NAME));
                }
                _ => entries.push(json!({ "id": PROFILE_ID, "name": PROFILE_NAME })),
            }
            match obj.get_mut("appliedId") {
                Some(slot) => *slot = json!(PROFILE_ID),
                None => {
                    obj.insert("appliedId".to_string(), json!(PROFILE_ID));
                }
            }
        } else {
            let next_id = match obj.get_mut("entries") {
                Some(entries) => {
                    let entries = entries.as_array_mut().ok_or_else(not_an_array)?;
                    entries.retain(|entry| !is_ours(entry));
                    entries
                        .iter()
                        .find_map(|entry| entry.get("id").and_then(Value::as_str))
                        .map(str::to_string)
                }
                None => None,
            };
            if obj.get("appliedId").and_then(Value::as_str) == Some(PROFILE_ID) {
                match next_id {
                    Some(next_id) => {
                        obj.insert("appliedId".to_string(), json!(next_id));
                    }
                    None => {
                        obj.shift_remove("appliedId");
                    }
                }
            }
        }
        patch_json::serialize(path, &doc, &style)
    }
}

pub fn write(
    env: &Env,
    target: &Provider,
    op: &str,
    pending: PendingTarget,
) -> Result<OperationReport, AppError> {
    let profile = gateway_profile(target)?;
    let applied = profile.is_some();
    let profile_file = profile_path(env);
    let normal = deployment_mode_patch(if applied { "3p" } else { "1p" });
    let mut threep_inner = deployment_mode_patch(if applied { "3p" } else { "1p" });
    if !applied {
        threep_inner.remove = LEGACY_ENTERPRISE_GATEWAY_KEYS
            .iter()
            .map(|key| KeyPath::new(&["enterpriseConfig", key]))
            .collect();
    }
    let threep = DropEmptyEnterpriseConfig(threep_inner);
    let profile_patch = profile_patch(profile);
    let meta = MetaPatch { applied };

    let commit = |target: &PendingTarget| super::commit_target(env, Tool::ClaudeDesktop, target);
    let write = AppWrite::begin(env.device_store(), Tool::ClaudeDesktop.as_str(), &commit)?;
    let mut changes = vec![
        FileChange {
            file: LiveFile::shared(env.claude_desktop_config_path()),
            patch: &normal as &dyn LivePatch,
        },
        FileChange {
            file: LiveFile::shared(env.claude_desktop_3p_config_path()),
            patch: &threep,
        },
    ];
    // 切回官方时没有 profile 文件就不建一个空的。
    if applied || profile_file.exists() {
        changes.push(FileChange {
            file: LiveFile::private(&profile_file),
            patch: &profile_patch,
        });
    }
    changes.push(FileChange {
        file: LiveFile::shared(meta_path(env)),
        patch: &meta,
    });
    write.run(op, &changes, pending)
}

/// 首次导入：`_meta.json` 里 CC Switch 的 profile 正生效、profile 里有网关地址和 Key，就
/// 导入成一张卡。只读，不写客户端文件。
pub fn imported_card(env: &Env) -> Result<Option<Provider>, AppError> {
    let read = |path: &Path| -> Result<Value, AppError> {
        match crate::live::engine::read_current(path).map_err(AppError::from)? {
            Some(bytes) if !bytes.is_empty() => {
                serde_json::from_slice(&bytes).map_err(|source| AppError::json(path, source))
            }
            _ => Ok(json!({})),
        }
    };
    let meta = read(&meta_path(env))?;
    if meta.get("appliedId").and_then(Value::as_str) != Some(PROFILE_ID) {
        return Ok(None);
    }
    let profile = read(&profile_path(env))?;
    let text = |key: &str| {
        profile
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let (Some(base_url), Some(api_key)) = (
        text("inferenceGatewayBaseUrl"),
        text("inferenceGatewayApiKey"),
    ) else {
        return Ok(None);
    };
    let mut card = Provider::new(
        crate::import::IMPORTED_ID,
        "当前配置 (Imported)",
        json!({ "env": { "ANTHROPIC_BASE_URL": base_url, "ANTHROPIC_AUTH_TOKEN": api_key } }),
    );
    if let Some(models) = profile.get("inferenceModels").and_then(Value::as_array) {
        let mut routes = serde_json::Map::new();
        for item in models {
            let (name, label, one_m) = match item {
                Value::String(name) => (name.clone(), None, false),
                Value::Object(obj) => (
                    obj.get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    obj.get("labelOverride")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    obj.get("supports1m")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ),
                _ => continue,
            };
            if !is_claude_safe_model_id(&name) {
                continue;
            }
            let mut route = json!({ "model": name });
            if let Some(label) = label {
                route["labelOverride"] = json!(label);
            }
            if one_m {
                route["supports1m"] = json!(true);
            }
            routes.insert(name, route);
        }
        if !routes.is_empty() {
            card.meta
                .extra
                .insert("claudeDesktopModelRoutes".into(), Value::Object(routes));
        }
    }
    validate(&card)?;
    Ok(Some(card))
}

#[cfg(test)]
mod tests {
    //! 四文件操作的崩溃注入：发布到一半崩溃，启动时前滚到全部落定；发布前崩溃则丢弃。
    use super::*;
    use crate::operation::failpoint;
    use crate::operation::RecoveryOutcome;
    use crate::store;
    use crate::switch::{recover_on_startup, switch};
    use std::fs;

    fn sandbox() -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        store::add_provider(
            &env,
            Tool::ClaudeDesktop,
            Provider::new("official", "Claude", json!({"env": {}})).official(),
        )
        .unwrap();
        store::add_provider(
            &env,
            Tool::ClaudeDesktop,
            Provider::new(
                "gw",
                "Gateway",
                json!({"env": {"ANTHROPIC_BASE_URL": "https://gw.example", "ANTHROPIC_AUTH_TOKEN": "k-gw"}}),
            ),
        )
        .unwrap();
        store::set_current(&env, Tool::ClaudeDesktop, "official").unwrap();
        let normal = env.claude_desktop_config_path();
        fs::create_dir_all(normal.parent().unwrap()).unwrap();
        fs::write(&normal, r#"{"mcpServers": {}}"#).unwrap();
        (dir, env)
    }

    fn current(env: &Env) -> Option<String> {
        store::load(env)
            .unwrap()
            .app(Tool::ClaudeDesktop)
            .unwrap()
            .current
            .clone()
    }

    fn read(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn fully_applied(env: &Env) {
        assert_eq!(
            read(&env.claude_desktop_config_path())["deploymentMode"],
            "3p"
        );
        assert_eq!(
            read(&env.claude_desktop_3p_config_path())["deploymentMode"],
            "3p"
        );
        assert_eq!(read(&profile_path(env))["inferenceGatewayApiKey"], "k-gw");
        assert_eq!(read(&meta_path(env))["appliedId"], PROFILE_ID);
        assert_eq!(current(env).as_deref(), Some("gw"));
    }

    #[test]
    fn a_crash_mid_publish_rolls_forward_to_all_four_files() {
        for point in ["published:0", "published:1", "published:2", "published:3"] {
            let (_dir, env) = sandbox();
            failpoint::crash_at(Some(point));
            let result = switch(&env, Tool::ClaudeDesktop, "gw");
            failpoint::crash_at(None);
            assert!(result.is_err(), "{point}");
            assert_eq!(current(&env).as_deref(), Some("official"), "{point}");
            let outcomes = recover_on_startup(&env);
            assert!(
                matches!(outcomes[..], [(_, Ok(RecoveryOutcome::RolledForward))]),
                "{point}: {outcomes:?}"
            );
            fully_applied(&env);
        }
    }

    #[test]
    fn a_crash_before_publishing_is_discarded() {
        let (_dir, env) = sandbox();
        failpoint::crash_at(Some("pending"));
        assert!(switch(&env, Tool::ClaudeDesktop, "gw").is_err());
        failpoint::crash_at(None);
        let outcomes = recover_on_startup(&env);
        assert!(
            outcomes
                .iter()
                .all(|(_, o)| matches!(o, Ok(RecoveryOutcome::Discarded))),
            "{outcomes:?}"
        );
        assert_eq!(current(&env).as_deref(), Some("official"));
        assert!(!profile_path(&env).exists());
        assert!(!env.claude_desktop_3p_config_path().exists());
        assert_eq!(
            fs::read_to_string(env.claude_desktop_config_path()).unwrap(),
            r#"{"mcpServers": {}}"#
        );
        switch(&env, Tool::ClaudeDesktop, "gw").unwrap();
        fully_applied(&env);
    }

    #[test]
    fn a_crash_while_restoring_official_rolls_forward_and_scrubs_the_key() {
        let (_dir, env) = sandbox();
        switch(&env, Tool::ClaudeDesktop, "gw").unwrap();
        failpoint::crash_at(Some("published:1"));
        assert!(switch(&env, Tool::ClaudeDesktop, "official").is_err());
        failpoint::crash_at(None);
        recover_on_startup(&env);
        assert_eq!(current(&env).as_deref(), Some("official"));
        assert!(!fs::read_to_string(profile_path(&env))
            .unwrap()
            .contains("k-gw"));
        assert!(read(&meta_path(&env)).get("appliedId").is_none());
        assert_eq!(
            read(&env.claude_desktop_config_path())["deploymentMode"],
            "1p"
        );
    }
}
