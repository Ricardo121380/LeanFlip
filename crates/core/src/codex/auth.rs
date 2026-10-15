//! Recognize Codex credential carriers without treating metadata as a login.
//! JWT decoding here is for local identity matching only; it is not signature verification.
use base64::{
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use serde_json::{json, Value};
fn nonblank(value: Option<&Value>) -> Option<&str> {
    value?.as_str().map(str::trim).filter(|s| !s.is_empty())
}
fn present(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(_) => true,
    }
}
pub fn jwt_claims(token: &str) -> Option<Value> {
    let segments: Vec<_> = token.split('.').collect();
    if segments.len() != 3 || segments[0].is_empty() {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(segments[1])
        .or_else(|_| URL_SAFE.decode(segments[1]))
        .ok()?;
    serde_json::from_slice(&bytes).ok().filter(Value::is_object)
}
pub fn extract_codex_id_token_subject(token: &str) -> Option<String> {
    let header = token.split('.').next()?;
    let decoded = URL_SAFE_NO_PAD
        .decode(header)
        .or_else(|_| URL_SAFE.decode(header))
        .ok()?;
    let header: Value = serde_json::from_slice(&decoded).ok()?;
    nonblank(header.get("alg"))?;
    nonblank(jwt_claims(token)?.get("sub")).map(str::to_string)
}
pub fn extract_codex_id_token_user_identity(token: &str) -> Option<String> {
    extract_codex_id_token_subject(token).map(|s| format!("sub:{s}"))
}
pub fn extract_codex_auth_user_identity(auth: &Value) -> Option<String> {
    extract_codex_id_token_user_identity(auth.pointer("/tokens/id_token")?.as_str()?)
}
/// Workspace (`chatgpt_account_id`) from the id token, for stored accounts that lack the field.
pub fn extract_chatgpt_account_id(token: &str) -> Option<String> {
    let claims = jwt_claims(token)?;
    nonblank(
        claims
            .get("https://api.openai.com/auth")
            .and_then(|a| a.get("chatgpt_account_id")),
    )
    .map(str::to_string)
}
/// ChatGPT user id from the id token. Codex session records store it as `creator_user_id`;
/// it differs from `sub`, which [`extract_codex_id_token_user_identity`] uses.
pub fn extract_chatgpt_user_id(token: &str) -> Option<String> {
    let claims = jwt_claims(token)?;
    let auth = claims.get("https://api.openai.com/auth");
    nonblank(auth.and_then(|a| a.get("chatgpt_user_id")))
        .or_else(|| nonblank(auth.and_then(|a| a.get("user_id"))))
        .map(str::to_string)
}
#[doc(hidden)]
pub fn test_id_token_with_claims(claims: &Value) -> String {
    format!(
        "{}.{}.",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}
#[doc(hidden)]
pub fn test_codex_id_token(subject: &str) -> String {
    test_id_token_with_claims(&json!({"sub":subject}))
}
pub fn extract_codex_auth_api_key(auth: &Value) -> Option<String> {
    nonblank(auth.get("OPENAI_API_KEY")).map(str::to_string)
}
pub fn codex_auth_has_credential_login_material(auth: &Value) -> bool {
    [
        "personal_access_token",
        "agent_identity",
        "bedrock_api_key",
        "bedrock_access_keys",
    ]
    .iter()
    .any(|key| present(auth.get(*key)))
        || ["id_token", "access_token", "refresh_token"]
            .iter()
            .any(|key| {
                auth.get("tokens")
                    .is_some_and(|t| nonblank(t.get(*key)).is_some())
            })
}
pub fn codex_auth_has_login_material(auth: &Value) -> bool {
    codex_auth_has_credential_login_material(auth) || extract_codex_auth_api_key(auth).is_some()
}
pub fn codex_live_auth_is_stale_third_party_residue(auth: &Value) -> bool {
    extract_codex_auth_api_key(auth).is_some() && !codex_auth_has_credential_login_material(auth)
}
pub fn codex_auth_has_openai_account_material(auth: &Value) -> bool {
    let mode = match auth.get("auth_mode") {
        Some(Value::String(s)) => s.as_str(),
        Some(Value::Null) | None => {
            if auth
                .get("personal_access_token")
                .is_some_and(|v| !v.is_null())
            {
                "personalAccessToken"
            } else if auth.get("bedrock_api_key").is_some_and(|v| !v.is_null()) {
                "bedrockApiKey"
            } else if auth
                .get("bedrock_access_keys")
                .is_some_and(|v| !v.is_null())
            {
                "bedrockAccessKeys"
            } else if auth.get("OPENAI_API_KEY").is_some_and(|v| !v.is_null()) {
                "apikey"
            } else {
                "chatgpt"
            }
        }
        _ => return false,
    };
    match mode {
        "apikey" => extract_codex_auth_api_key(auth).is_some(),
        "personalAccessToken" => present(auth.get("personal_access_token")),
        "agentIdentity" => present(auth.get("agent_identity")),
        "chatgpt" | "chatgptAuthTokens" => ["id_token", "access_token", "refresh_token"]
            .iter()
            .any(|key| {
                auth.get("tokens")
                    .is_some_and(|t| nonblank(t.get(*key)).is_some())
            }),
        _ => false,
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexAuthStoreMode {
    File,
    Keyring,
    Auto,
    Ephemeral,
    Unknown,
}
pub fn codex_config_auth_store_mode(source: &str) -> CodexAuthStoreMode {
    let Ok(root) = source.parse::<toml::Table>() else {
        return CodexAuthStoreMode::Unknown;
    };
    match root.get("cli_auth_credentials_store") {
        None => CodexAuthStoreMode::File,
        Some(toml::Value::String(s)) => match s.as_str() {
            "file" => CodexAuthStoreMode::File,
            "keyring" => CodexAuthStoreMode::Keyring,
            "auto" => CodexAuthStoreMode::Auto,
            "ephemeral" => CodexAuthStoreMode::Ephemeral,
            _ => CodexAuthStoreMode::Unknown,
        },
        _ => CodexAuthStoreMode::Unknown,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chatgpt_user_id_comes_from_the_openai_claim_not_sub() {
        let token = test_id_token_with_claims(&json!({
            "sub": "auth0|abc",
            "https://api.openai.com/auth": {"chatgpt_user_id": "user-1", "user_id": "user-2"}
        }));
        assert_eq!(extract_chatgpt_user_id(&token).as_deref(), Some("user-1"));
        let fallback = test_id_token_with_claims(&json!({
            "https://api.openai.com/auth": {"chatgpt_user_id": " ", "user_id": "user-2"}
        }));
        assert_eq!(
            extract_chatgpt_user_id(&fallback).as_deref(),
            Some("user-2")
        );
        assert_eq!(
            extract_chatgpt_user_id(&test_codex_id_token("only-sub")),
            None
        );
        assert_eq!(extract_chatgpt_user_id("not-a-jwt"), None);
        let workspace = test_id_token_with_claims(&json!({
            "https://api.openai.com/auth": {"chatgpt_account_id": "ws-1"}
        }));
        assert_eq!(
            extract_chatgpt_account_id(&workspace).as_deref(),
            Some("ws-1")
        );
        assert_eq!(extract_chatgpt_account_id(&token), None);
    }
    #[test]
    fn metadata_is_not_login_and_invalid_store_types_are_unknown() {
        assert!(!codex_auth_has_login_material(
            &json!({"last_refresh":"today","tokens":{"account_id":"workspace"}})
        ));
        assert_eq!(
            codex_config_auth_store_mode("cli_auth_credentials_store=17"),
            CodexAuthStoreMode::Unknown
        );
        assert_eq!(
            codex_config_auth_store_mode("[broken"),
            CodexAuthStoreMode::Unknown
        );
        assert!(!codex_live_auth_is_stale_third_party_residue(
            &json!({"OPENAI_API_KEY":"old","bedrock_access_keys":{"secret_access_key":"test"}})
        ));
    }
}
