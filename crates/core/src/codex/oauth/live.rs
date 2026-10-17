//! Credential ownership is the tuple (local account, subject, workspace).
use crate::codex::auth::extract_codex_auth_user_identity;
use crate::{
    env::Env,
    error::AppError,
    live::engine::{digest, read_current},
    provider::Tool,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub const MARKER_FILENAME: &str = "codex_managed_oauth_live_auth.json";
pub const MARKER_VERSION: u32 = 3;
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
pub fn read_marker(env: &Env) -> Option<Marker> {
    serde_json::from_slice(&read_current(&marker_path(env)).ok()??).ok()
}
fn text(value: Option<&Value>) -> Option<&str> {
    value?.as_str().map(str::trim).filter(|s| !s.is_empty())
}
pub fn managed_workspace(auth: &Value) -> Option<String> {
    if auth.get("auth_mode").and_then(Value::as_str) != Some("chatgpt") {
        return None;
    }
    if auth
        .get("OPENAI_API_KEY")
        .is_some_and(|v| !v.is_null() && v.as_str() != Some("PROXY_MANAGED"))
    {
        return None;
    }
    let root = auth.as_object()?;
    if root
        .keys()
        .any(|k| !["auth_mode", "OPENAI_API_KEY", "tokens", "last_refresh"].contains(&k.as_str()))
    {
        return None;
    }
    let tokens = auth.get("tokens")?.as_object()?;
    if tokens
        .keys()
        .any(|k| !["id_token", "access_token", "refresh_token", "account_id"].contains(&k.as_str()))
    {
        return None;
    }
    text(tokens.get("access_token"))?;
    text(tokens.get("account_id")).map(str::to_string)
}
pub fn auth_matches_marker(auth: &Value, marker: Option<&Marker>, account: &str) -> bool {
    let Some(marker) = marker.filter(|m| {
        m.version == MARKER_VERSION && !account.trim().is_empty() && m.account_id == account.trim()
    }) else {
        return false;
    };
    let workspace = managed_workspace(auth);
    let user = extract_codex_auth_user_identity(auth);
    workspace.is_some()
        && user.is_some()
        && workspace == marker.chatgpt_account_id
        && user == marker.user_identity
}
pub fn live_is_managed(env: &Env, auth: &Value, account: &str) -> bool {
    auth_matches_marker(auth, read_marker(env).as_ref(), account)
}
pub fn marker_bytes(auth: &Value, account: &str) -> Result<Option<Vec<u8>>, AppError> {
    let Some(workspace) = managed_workspace(auth).filter(|_| !account.trim().is_empty()) else {
        return Ok(None);
    };
    let identity = extract_codex_auth_user_identity(auth).ok_or_else(|| {
        AppError::localized(
            "codex.oauth.missing_identity",
            "登录信息缺少用户身份，请重新登录",
            "Sign in again; the token has no user identity",
        )
    })?;
    serde_json::to_vec_pretty(&Marker {
        version: MARKER_VERSION,
        account_id: account.trim().into(),
        chatgpt_account_id: Some(workspace),
        user_identity: Some(identity),
    })
    .map(Some)
    .map_err(|source| AppError::JsonSerialize { source })
}
pub fn managed_auth_value(
    workspace: &str,
    access_token: &str,
    id_token: Option<&str>,
    refresh_token: &str,
    last_refresh: &str,
) -> Value {
    let mut auth = json!({"auth_mode":"chatgpt","OPENAI_API_KEY":null,"last_refresh":last_refresh,"tokens":{"account_id":workspace,"access_token":access_token,"refresh_token":refresh_token}});
    if let Some(token) = id_token {
        auth["tokens"]["id_token"] = json!(token);
    }
    auth
}
pub fn format_last_refresh(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}
pub fn parse_last_refresh(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|v| v.timestamp_millis())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRefresh {
    pub refresh_token: String,
    pub id_token: Option<String>,
    pub last_refresh_ms: Option<i64>,
    pub workspace: String,
}
fn read_auth(env: &Env) -> Result<Option<Value>, AppError> {
    let path = env.codex_auth_path();
    read_current(&path)?
        .map(|b| serde_json::from_slice(&b).map_err(|e| AppError::json(&path, e)))
        .transpose()
}
pub fn read_live_refresh(env: &Env, account: &str) -> Result<Option<LiveRefresh>, AppError> {
    let Some(auth) = read_auth(env)? else {
        return Ok(None);
    };
    if !live_is_managed(env, &auth, account) {
        return Ok(None);
    }
    Ok(
        text(auth.pointer("/tokens/refresh_token")).map(|token| LiveRefresh {
            refresh_token: token.into(),
            id_token: text(auth.pointer("/tokens/id_token")).map(str::to_string),
            last_refresh_ms: text(auth.get("last_refresh")).and_then(parse_last_refresh),
            workspace: managed_workspace(&auth).unwrap(),
        }),
    )
}
fn conflict() -> AppError {
    AppError::localized(
        "codex.oauth.live_changed",
        "Codex 登录在操作期间发生变化，请重试",
        "Codex credentials changed during the operation; retry",
    )
}
pub fn ensure_live_unchanged(env: &Env, account: &str, expected: &str) -> Result<(), AppError> {
    if read_live_refresh(env, account)?.is_some_and(|v| v.refresh_token == expected.trim()) {
        Ok(())
    } else {
        Err(conflict())
    }
}
pub fn sync_live_after_refresh(
    env: &Env,
    account: &str,
    expected: &str,
    refreshed: &Value,
) -> Result<bool, AppError> {
    let device = env.device_store();
    let _lock = device.lock_app(Tool::Codex.as_str())?;
    if crate::state::pending(&device, Tool::Codex.as_str())?.is_some() {
        return Ok(false);
    }
    let path = env.codex_auth_path();
    let before = read_current(&path)?;
    let Some(auth) = before
        .as_deref()
        .and_then(|b| serde_json::from_slice::<Value>(b).ok())
    else {
        return Ok(false);
    };
    let marker = read_marker(env);
    if !auth_matches_marker(&auth, marker.as_ref(), account)
        || text(auth.pointer("/tokens/refresh_token")) != Some(expected.trim())
    {
        return Ok(false);
    }
    // Refresh must keep the same owner. The existing marker remains valid, so
    // only one file needs publication and there is no two-file crash gap.
    if !auth_matches_marker(refreshed, marker.as_ref(), account) {
        return Err(AppError::Config(
            "刷新响应改变了账号身份 / Refresh changed account identity".into(),
        ));
    }
    let staged = crate::fsutil::stage_write(
        &path,
        &crate::fsutil::sorted_json_bytes(refreshed)?,
        Some(0o600),
        true,
    )?;
    let publish = (|| {
        if digest(read_current(&path)?.as_deref()) != digest(before.as_deref())
            || read_marker(env) != marker
        {
            return Ok(false);
        }
        crate::fsutil::commit_staged(staged.tmp_path(), &path)?;
        Ok(true)
    })();
    // This stage is not journal-owned. A conflict or I/O error must not leave
    // an untracked copy of refreshed credentials behind.
    if !matches!(publish, Ok(true)) {
        let _ = crate::fsutil::delete_file(staged.tmp_path());
    }
    publish
}
/// Capture the exact owned files while the caller holds the Codex write lock.
/// Guarded patches refuse to delete a login replaced by the client meanwhile.
pub(super) fn prepare_account_removal(
    env: &Env,
    accounts: &[String],
) -> Result<Vec<(crate::live::engine::LiveFile, crate::live::patch::Guarded)>, AppError> {
    use crate::live::{
        engine::LiveFile,
        patch::{Guarded, WholeFile},
    };
    let marker_path = marker_path(env);
    let marker_bytes = read_current(&marker_path)?;
    let marker: Option<Marker> = marker_bytes
        .as_deref()
        .and_then(|bytes| serde_json::from_slice(bytes).ok());
    let auth_path = env.codex_auth_path();
    let auth_bytes = read_current(&auth_path)?;
    let auth: Option<Value> = auth_bytes
        .as_deref()
        .map(|bytes| {
            serde_json::from_slice(bytes).map_err(|error| AppError::json(&auth_path, error))
        })
        .transpose()?;
    let mut changes = Vec::new();
    if auth.as_ref().is_some_and(|auth| {
        accounts
            .iter()
            .any(|id| auth_matches_marker(auth, marker.as_ref(), id))
    }) {
        changes.push((
            LiveFile::private(auth_path),
            Guarded {
                expected_pre: digest(auth_bytes.as_deref()),
                then: WholeFile::Delete,
            },
        ));
    }
    if marker.is_some_and(|marker| accounts.contains(&marker.account_id)) {
        changes.push((
            LiveFile::private(marker_path),
            Guarded {
                expected_pre: digest(marker_bytes.as_deref()),
                then: WholeFile::Delete,
            },
        ));
    }
    Ok(changes)
}

pub fn clear_live_for_account(env: &Env, account: &str) -> Result<(), AppError> {
    let _lock = env.device_store().lock_app(Tool::Codex.as_str())?;
    let marker = read_marker(env);
    let auth = read_auth(env)?;
    if auth
        .as_ref()
        .is_some_and(|a| auth_matches_marker(a, marker.as_ref(), account))
    {
        crate::fsutil::delete_file(&env.codex_auth_path())?;
    }
    if marker.is_some_and(|m| m.account_id == account.trim()) {
        crate::fsutil::delete_file(&marker_path(env))?;
    }
    Ok(())
}
