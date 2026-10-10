//! Codex `auth.json` 的判定函数（移植自 cc-switch `codex_config.rs`，逻辑未改）。
//!
//! 这些函数决定「`auth.json` 里有没有登录」「是不是旧版留下的第三方 Key 残留」「是谁
//! 的登录」，是混合登录（ChatGPT OAuth 与 API Key 供应商并存）保护的基础，见
//! [`crate::codex::login`]。

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use toml_edit::DocumentMut;

/// 从原生 auth.json 的 id_token 提取跨刷新稳定的用户身份。
pub fn extract_codex_auth_user_identity(auth: &Value) -> Option<String> {
    let id_token = auth.pointer("/tokens/id_token")?.as_str()?;
    extract_codex_id_token_user_identity(id_token)
}

pub fn extract_codex_id_token_user_identity(id_token: &str) -> Option<String> {
    extract_codex_id_token_subject(id_token).map(|subject| format!("sub:{subject}"))
}

pub fn extract_codex_id_token_subject(id_token: &str) -> Option<String> {
    let mut segments = id_token.split('.');
    let header = segments.next()?;
    let payload = segments.next()?;
    segments.next()?;
    if segments.next().is_some() {
        return None;
    }

    let header: Value = URL_SAFE_NO_PAD
        .decode(header)
        .ok()
        .and_then(|decoded| serde_json::from_slice(&decoded).ok())?;
    header
        .get("alg")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    let claims: Value = URL_SAFE_NO_PAD
        .decode(payload)
        .ok()
        .and_then(|decoded| serde_json::from_slice(&decoded).ok())?;
    claims
        .get("sub")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 测试用：只带 `sub` 的未签名 id_token（和 Codex 写出的一样能解出用户）。
#[doc(hidden)]
pub fn test_codex_id_token(subject: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
    let payload = URL_SAFE_NO_PAD.encode(json!({ "sub": subject }).to_string());
    format!("{header}.{payload}.")
}


pub fn extract_codex_auth_api_key(auth: &Value) -> Option<String> {
    auth.get("OPENAI_API_KEY")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}


pub fn codex_auth_has_login_material(auth: &Value) -> bool {
    let Some(obj) = auth.as_object() else {
        return false;
    };

    obj.iter().any(|(key, value)| {
        if key == "auth_mode" {
            return false;
        }

        if key == "OPENAI_API_KEY" {
            return value
                .as_str()
                .map(str::trim)
                .is_some_and(|token| !token.is_empty());
        }

        match value {
            Value::Null => false,
            Value::String(text) => !text.trim().is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Object(map) => !map.is_empty(),
            _ => true,
        }
    })
}

/// The auth mode Codex resolves for an `auth.json` payload
/// (`AuthDotJson::resolved_mode`, `codex-rs/login/src/auth/manager.rs`,
/// 0.153.2): an explicit `auth_mode` wins outright; otherwise presence
/// decides in this order — `personal_access_token`, `bedrock_api_key`,
/// `bedrock_access_keys`, `OPENAI_API_KEY` — and everything else is
/// ChatGPT. Presence is `Option::is_some`, i.e. any non-null value, even an
/// empty one; the material itself is checked afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexResolvedAuthMode {
    ApiKey,
    Chatgpt,
    ChatgptAuthTokens,
    Headers,
    AgentIdentity,
    PersonalAccessToken,
    BedrockApiKey,
    BedrockAccessKeys,
    /// An `auth_mode` string Codex's serde rejects (`rename_all =
    /// "lowercase"` plus explicit camelCase renames, exact match): the whole
    /// file fails to load, which Codex reports as signed out.
    Unrecognized,
}

fn codex_auth_resolved_mode(auth: &serde_json::Map<String, Value>) -> CodexResolvedAuthMode {
    let present = |key: &str| auth.get(key).is_some_and(|value| !value.is_null());

    // `auth_mode: null` deserializes to `None` (serde default) and falls
    // through to the implicit precedence below.
    if let Some(mode) = auth.get("auth_mode").filter(|value| !value.is_null()) {
        return match mode.as_str() {
            Some("apikey") => CodexResolvedAuthMode::ApiKey,
            Some("chatgpt") => CodexResolvedAuthMode::Chatgpt,
            Some("chatgptAuthTokens") => CodexResolvedAuthMode::ChatgptAuthTokens,
            Some("headers") => CodexResolvedAuthMode::Headers,
            Some("agentIdentity") => CodexResolvedAuthMode::AgentIdentity,
            Some("personalAccessToken") => CodexResolvedAuthMode::PersonalAccessToken,
            Some("bedrockApiKey") => CodexResolvedAuthMode::BedrockApiKey,
            Some("bedrockAccessKeys") => CodexResolvedAuthMode::BedrockAccessKeys,
            _ => CodexResolvedAuthMode::Unrecognized,
        };
    }
    if present("personal_access_token") {
        return CodexResolvedAuthMode::PersonalAccessToken;
    }
    if present("bedrock_api_key") {
        return CodexResolvedAuthMode::BedrockApiKey;
    }
    if present("bedrock_access_keys") {
        return CodexResolvedAuthMode::BedrockAccessKeys;
    }
    if present("OPENAI_API_KEY") {
        return CodexResolvedAuthMode::ApiKey;
    }
    CodexResolvedAuthMode::Chatgpt
}

/// True when Codex would load `auth` as a signed-in OpenAI account for a
/// `requires_openai_auth` provider — the state its login screen and
/// `ConfiguredModelProvider::account_state` (0.149+) go by. The auth mode is
/// resolved exactly as Codex does (`codex_auth_resolved_mode`) and only then
/// is the matching credential checked, so a Bedrock credential outranks a
/// stale `OPENAI_API_KEY` sitting next to it just as it does in Codex, where
/// that probe returns `UnsupportedBedrockApiKeyAuth` and fails TUI startup.
/// Modes Codex cannot load from storage (`headers`, unrecognized) are signed
/// out. The credential must be non-blank (stricter than Codex's `is_some`,
/// erring toward "signed out"); metadata such as `last_refresh` never counts.
pub fn codex_auth_has_openai_account_material(auth: &Value) -> bool {
    let Some(obj) = auth.as_object() else {
        return false;
    };

    let value_present = |value: &Value| match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
        _ => true,
    };
    let has = |key: &str| obj.get(key).is_some_and(value_present);

    match codex_auth_resolved_mode(obj) {
        CodexResolvedAuthMode::ApiKey => extract_codex_auth_api_key(auth).is_some(),
        CodexResolvedAuthMode::PersonalAccessToken => has("personal_access_token"),
        CodexResolvedAuthMode::AgentIdentity => has("agent_identity"),
        CodexResolvedAuthMode::Chatgpt | CodexResolvedAuthMode::ChatgptAuthTokens => obj
            .get("tokens")
            .and_then(Value::as_object)
            .is_some_and(|tokens| {
                ["id_token", "access_token", "refresh_token"]
                    .iter()
                    .any(|key| tokens.get(*key).is_some_and(value_present))
            }),
        CodexResolvedAuthMode::Headers
        | CodexResolvedAuthMode::BedrockApiKey
        | CodexResolvedAuthMode::BedrockAccessKeys
        | CodexResolvedAuthMode::Unrecognized => false,
    }
}

/// Where Codex keeps CLI auth, per the top-level `cli_auth_credentials_store`
/// key (`codex-rs/config/src/types.rs`, serde lowercase; unset = `file`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexAuthStoreMode {
    /// `auth.json` is the only store — the file decides login state.
    File,
    /// Keyring only; `auth.json` is never read and is deleted after a save.
    Keyring,
    /// Keyring first, `auth.json` as fallback for both load and save.
    Auto,
    /// In-process only; nothing on disk is ever a login.
    Ephemeral,
    /// Unparsable config or a value Codex would reject.
    Unknown,
}

pub fn codex_config_auth_store_mode(config_text: &str) -> CodexAuthStoreMode {
    if !config_text.contains("cli_auth_credentials_store") {
        return CodexAuthStoreMode::File;
    }
    let Ok(doc) = config_text.parse::<DocumentMut>() else {
        return CodexAuthStoreMode::Unknown;
    };
    match doc
        .get("cli_auth_credentials_store")
        .and_then(|item| item.as_str())
    {
        None => CodexAuthStoreMode::File,
        Some("file") => CodexAuthStoreMode::File,
        Some("keyring") => CodexAuthStoreMode::Keyring,
        Some("auto") => CodexAuthStoreMode::Auto,
        Some("ephemeral") => CodexAuthStoreMode::Ephemeral,
        Some(_) => CodexAuthStoreMode::Unknown,
    }
}

/// True only when the auth carries material Codex itself authenticates with
/// ahead of the API-key fallback: OAuth tokens or another first-class login
/// carrier. Unlike `codex_auth_has_oauth_login_material`, pure metadata such
/// as `last_refresh` or `tokens.account_id` does NOT count — metadata must not
/// shield a stale third-party `OPENAI_API_KEY` from post-switch cleanup.
pub fn codex_auth_has_credential_login_material(auth: &Value) -> bool {
    let Some(obj) = auth.as_object() else {
        return false;
    };

    let value_present = |value: &Value| match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
        _ => true,
    };

    if ["personal_access_token", "agent_identity", "bedrock_api_key"]
        .iter()
        .any(|key| obj.get(*key).is_some_and(value_present))
    {
        return true;
    }

    obj.get("tokens")
        .and_then(Value::as_object)
        .is_some_and(|tokens| {
            ["id_token", "access_token", "refresh_token"]
                .iter()
                .any(|key| tokens.get(*key).is_some_and(value_present))
        })
}

/// True when live `auth.json` is the shape a preserve-off third-party switch
/// leaves behind: an `OPENAI_API_KEY` (possibly alongside metadata like
/// `auth_mode` / `last_refresh`) with no real login credential next to it.
pub fn codex_live_auth_is_stale_third_party_residue(live_auth: &Value) -> bool {
    if codex_auth_has_credential_login_material(live_auth) {
        return false;
    }
    live_auth
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|key| !key.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_comes_from_the_id_token_subject() {
        let auth = json!({"tokens": {"id_token": test_codex_id_token("alice")}});
        assert_eq!(
            extract_codex_auth_user_identity(&auth).as_deref(),
            Some("sub:alice")
        );
        assert_eq!(extract_codex_auth_user_identity(&json!({"tokens": {"id_token": "x.y"}})), None);
    }

    #[test]
    fn login_material_ignores_metadata() {
        let metadata = json!({"auth_mode": "chatgpt", "last_refresh": "t", "tokens": {"account_id": "a"}});
        assert!(!codex_auth_has_credential_login_material(&metadata));
        assert!(!codex_auth_has_openai_account_material(&metadata));
        let oauth = json!({"tokens": {"refresh_token": "r"}});
        assert!(codex_auth_has_credential_login_material(&oauth));
        assert!(codex_auth_has_openai_account_material(&oauth));
        assert!(!codex_auth_has_login_material(&json!({})));
    }

    #[test]
    fn bedrock_outranks_a_stale_api_key() {
        let auth = json!({"bedrock_api_key": "b", "OPENAI_API_KEY": "sk"});
        assert!(!codex_auth_has_openai_account_material(&auth));
        assert!(codex_auth_has_openai_account_material(&json!({"OPENAI_API_KEY": "sk"})));
    }

    #[test]
    fn residue_is_a_key_without_a_login() {
        assert!(codex_live_auth_is_stale_third_party_residue(
            &json!({"OPENAI_API_KEY": "sk-relay", "auth_mode": "apikey"})
        ));
        assert!(!codex_live_auth_is_stale_third_party_residue(
            &json!({"OPENAI_API_KEY": "sk", "tokens": {"access_token": "a"}})
        ));
    }

    #[test]
    fn store_mode_follows_cli_auth_credentials_store() {
        assert_eq!(codex_config_auth_store_mode(""), CodexAuthStoreMode::File);
        assert_eq!(
            codex_config_auth_store_mode("cli_auth_credentials_store = \"keyring\"\n"),
            CodexAuthStoreMode::Keyring
        );
        assert_eq!(
            codex_config_auth_store_mode("cli_auth_credentials_store = \"auto\"\n"),
            CodexAuthStoreMode::Auto
        );
        assert_eq!(
            codex_config_auth_store_mode("cli_auth_credentials_store = \"nope\"\n"),
            CodexAuthStoreMode::Unknown
        );
        assert_eq!(
            codex_config_auth_store_mode("cli_auth_credentials_store = [\n"),
            CodexAuthStoreMode::Unknown
        );
    }
}
