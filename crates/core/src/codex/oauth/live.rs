//! 托管账号和 Codex live `auth.json` 的关系：所有权标记（marker）、读回 Codex CLI 轮换过的
//! refresh token、刷新后同步 live、切走前的代数核对。
//!
//! 移植自 cc-switch `codex_config.rs` 的托管 OAuth 部分。只认 v3 标记（本地账号 id +
//! workspace + 用户身份）：同一个 Team workspace 里可以有几个用户，只看 workspace 会把
//! 别人的原生登录当成托管登录删掉。ccswitch-lite 从不写 v1/v2 标记，没有迁移逻辑。

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::codex::auth::extract_codex_auth_user_identity;
use crate::env::Env;
use crate::error::AppError;
use crate::live::engine::read_current;
use crate::provider::Tool;

/// 标记文件（和 cc-switch 同名，放在 ccswitch-lite 的数据目录里）。
pub const MARKER_FILENAME: &str = "codex_managed_oauth_live_auth.json";
pub const MARKER_VERSION: u32 = 3;

/// 「live 的 `auth.json` 是哪个托管账号写的」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub version: u32,
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_identity: Option<String>,
}

pub fn marker_path(env: &Env) -> std::path::PathBuf {
    env.data_file(MARKER_FILENAME)
}

/// 读标记。不存在或解析不了都是 `None`：坏掉的标记证明不了任何所有权。
pub fn read_marker(env: &Env) -> Option<Marker> {
    let bytes = read_current(&marker_path(env)).ok().flatten()?;
    match serde_json::from_slice::<Marker>(&bytes) {
        Ok(marker) => Some(marker),
        Err(err) => {
            log::warn!("Codex 托管账号标记无法解析，按没有标记处理: {err}");
            None
        }
    }
}

/// ChatGPT 登录形状里的 workspace id。只接受托管 bundle 的形状（`auth_mode` 为
/// chatgpt、没有 API Key、tokens 只有这四个键），用户自己加了别的东西的不算。
pub fn managed_workspace(auth: &Value) -> Option<String> {
    let obj = auth.as_object()?;
    if obj.keys().any(|key| {
        !matches!(
            key.as_str(),
            "auth_mode" | "OPENAI_API_KEY" | "tokens" | "last_refresh"
        )
    }) {
        return None;
    }
    if auth.get("auth_mode").and_then(Value::as_str) != Some("chatgpt") {
        return None;
    }
    let key_clearable = auth
        .get("OPENAI_API_KEY")
        .is_none_or(|value| value.is_null() || value.as_str() == Some("PROXY_MANAGED"));
    if !key_clearable {
        return None;
    }
    let tokens = auth.get("tokens").and_then(Value::as_object)?;
    if tokens.keys().any(|key| {
        !matches!(
            key.as_str(),
            "access_token" | "account_id" | "id_token" | "refresh_token"
        )
    }) {
        return None;
    }
    tokens
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())?;
    tokens
        .get("account_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// `auth` 是不是托管账号 `account_id` 写进去的登录：标记里的本地账号、workspace、用户
/// 三者都要对上。
pub fn auth_matches_marker(auth: &Value, marker: Option<&Marker>, account_id: &str) -> bool {
    let (Some(marker), Some(workspace)) = (marker, managed_workspace(auth)) else {
        return false;
    };
    let user = extract_codex_auth_user_identity(auth);
    marker.version == MARKER_VERSION
        && marker.account_id == account_id.trim()
        && !account_id.trim().is_empty()
        && marker.chatgpt_account_id.as_deref() == Some(workspace.as_str())
        && marker
            .user_identity
            .as_deref()
            .is_some_and(|identity| user.as_deref() == Some(identity))
}

/// 按盘上的标记判断（[`auth_matches_marker`]）。
pub fn live_is_managed(env: &Env, auth: &Value, account_id: &str) -> bool {
    auth_matches_marker(auth, read_marker(env).as_ref(), account_id)
}

/// 托管账号写 `auth.json` 时同一次提交里写的标记。`auth` 不是托管 bundle 的形状时 `None`。
pub fn marker_bytes(auth: &Value, account_id: &str) -> Result<Option<Vec<u8>>, AppError> {
    let account_id = account_id.trim();
    let Some(workspace) = managed_workspace(auth) else {
        return Ok(None);
    };
    if account_id.is_empty() {
        return Ok(None);
    }
    let user = extract_codex_auth_user_identity(auth).ok_or_else(|| {
        AppError::localized(
            "codex.oauth.missing_identity",
            "托管账号的 id_token 缺少稳定的用户身份，无法记录 auth.json 的所有权，请重新登录这个账号",
            "The managed account's id_token has no stable user identity, so ownership of auth.json cannot be recorded. Sign in to this account again",
        )
    })?;
    let marker = Marker {
        version: MARKER_VERSION,
        account_id: account_id.to_string(),
        chatgpt_account_id: Some(workspace),
        user_identity: Some(user),
    };
    serde_json::to_vec_pretty(&marker)
        .map(Some)
        .map_err(|source| AppError::JsonSerialize { source })
}

/// 托管账号写进 `auth.json` 的登录（和 Codex 原生浏览器登录同形）。
pub fn managed_auth_value(
    workspace: &str,
    access_token: &str,
    id_token: Option<&str>,
    refresh_token: &str,
    last_refresh: &str,
) -> Value {
    let mut tokens = Map::new();
    if let Some(id_token) = id_token {
        tokens.insert("id_token".into(), Value::String(id_token.to_string()));
    }
    tokens.insert("access_token".into(), Value::String(access_token.into()));
    tokens.insert("refresh_token".into(), Value::String(refresh_token.into()));
    tokens.insert("account_id".into(), Value::String(workspace.into()));
    json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": Value::Object(tokens),
        "last_refresh": last_refresh,
    })
}

pub fn format_last_refresh(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

pub fn parse_last_refresh(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.timestamp_millis())
}

/// live 里属于某个托管账号的那一代 token。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRefresh {
    pub refresh_token: String,
    pub id_token: Option<String>,
    pub last_refresh_ms: Option<i64>,
    pub workspace: String,
}

fn read_live_auth(env: &Env) -> Result<Option<Value>, AppError> {
    let path = env.codex_auth_path();
    let Some(bytes) = read_current(&path)? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| AppError::json(&path, source))
}

fn refresh_of(auth: &Value) -> Option<&str> {
    auth.pointer("/tokens/refresh_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

/// 读回 live 里属于 `account_id` 的 refresh token（只在标记证明是它写的时候）。
pub fn read_live_refresh(env: &Env, account_id: &str) -> Result<Option<LiveRefresh>, AppError> {
    let Some(auth) = read_live_auth(env)? else {
        return Ok(None);
    };
    if !live_is_managed(env, &auth, account_id) {
        return Ok(None);
    }
    let Some(refresh_token) = refresh_of(&auth) else {
        return Ok(None);
    };
    Ok(Some(LiveRefresh {
        refresh_token: refresh_token.to_string(),
        id_token: auth
            .pointer("/tokens/id_token")
            .and_then(Value::as_str)
            .map(str::to_string),
        last_refresh_ms: auth
            .get("last_refresh")
            .and_then(Value::as_str)
            .and_then(parse_last_refresh),
        workspace: managed_workspace(&auth).unwrap_or_default(),
    }))
}

fn changed_during_switch(account_id: &str) -> AppError {
    AppError::localized(
        "codex.oauth.live_changed",
        format!("Codex CLI 账号 {account_id} 的 live 凭据在切换期间已刷新；为避免覆盖新的 refresh token，本次操作已取消，请重试"),
        format!("The live credentials of Codex account {account_id} were refreshed during the switch; to avoid overwriting the new refresh token nothing was changed. Please try again"),
    )
}

/// 切走托管账号前采纳过的那一代，到写锁里还在：不在说明 Codex CLI 在这期间又刷新或
/// 登出了，停下。
pub fn ensure_live_unchanged(
    env: &Env,
    account_id: &str,
    expected_refresh: &str,
) -> Result<(), AppError> {
    let auth = read_live_auth(env)?.ok_or_else(|| changed_during_switch(account_id))?;
    if !live_is_managed(env, &auth, account_id)
        || refresh_of(&auth) != Some(expected_refresh.trim())
    {
        return Err(changed_during_switch(account_id));
    }
    Ok(())
}

/// 管理器刷新了托管账号的 token 后，把新的一代写回 live（Codex CLI 拿旧的 refresh token
/// 会被服务端当成重放）。只在 live 仍是这个账号、仍是刷新前那一代、Codex 没有未完成的
/// 切换时写；拿 Codex 的写锁，和切换互斥。Codex CLI 不参与这把锁，只能缩小窗口。
pub fn sync_live_after_refresh(
    env: &Env,
    account_id: &str,
    expected_refresh: &str,
    refreshed: &Value,
) -> Result<bool, AppError> {
    let _guard = env.device_store().lock_app(Tool::Codex.as_str());
    if crate::state::pending(&env.device_store(), Tool::Codex.as_str())?.is_some() {
        return Ok(false);
    }
    let Some(auth) = read_live_auth(env)? else {
        return Ok(false);
    };
    if !live_is_managed(env, &auth, account_id)
        || refresh_of(&auth) != Some(expected_refresh.trim())
    {
        return Ok(false);
    }
    let bytes = crate::fsutil::sorted_json_bytes(refreshed)?;
    crate::fsutil::atomic_write_private(&env.codex_auth_path(), &bytes)?;
    if let Some(marker) = marker_bytes(refreshed, account_id)? {
        crate::fsutil::atomic_write_private(&marker_path(env), &marker)?;
    }
    Ok(true)
}

/// 删除托管账号时清掉它留在 live 的登录和标记。live 不是它的就只清标记（如果标记是它的）。
pub fn clear_live_for_account(env: &Env, account_id: &str) -> Result<(), AppError> {
    let _guard = env.device_store().lock_app(Tool::Codex.as_str());
    let marker = read_marker(env);
    let auth = read_live_auth(env).unwrap_or(None);
    if auth
        .as_ref()
        .is_some_and(|auth| auth_matches_marker(auth, marker.as_ref(), account_id))
    {
        crate::fsutil::delete_file(&env.codex_auth_path())?;
        crate::fsutil::delete_file(&marker_path(env))?;
        return Ok(());
    }
    let marker_is_ours = marker
        .as_ref()
        .is_some_and(|marker| marker.account_id == account_id.trim());
    let marker_unreadable = marker.is_none() && marker_path(env).exists();
    if marker_is_ours || marker_unreadable {
        crate::fsutil::delete_file(&marker_path(env))?;
    }
    Ok(())
}
