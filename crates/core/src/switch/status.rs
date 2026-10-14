//! Read-only comparison of routing and authentication, not model or client preferences.
//! Switching supplies the expected route; only its identity is compared. No credentials leave core.
use super::*;
use crate::live::{
    engine::read_current,
    patch::{dotenv, LivePatch},
    project::{
        claude::{direct_patch, ClaudeProjection},
        grok::GrokConfigPatch,
    },
};
use serde_json::{json, Value};
use std::path::Path;

fn read(path: &Path) -> Result<Vec<u8>, AppError> {
    Ok(read_current(path)?.unwrap_or_default())
}
fn json_value(path: &Path, bytes: &[u8]) -> Result<Value, AppError> {
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|e| AppError::json(path, e))?;
    if !value.is_object() {
        return Err(AppError::Config(format!(
            "{}: expected an object",
            path.display()
        )));
    }
    Ok(value)
}
fn fields(value: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|key| value.get(*key).map(|v| ((*key).into(), v.clone())))
            .collect(),
    )
}
fn claude_identity(value: &Value) -> Value {
    let mut identity = fields(
        value,
        &[
            "apiBaseUrl",
            "apiKey",
            "apiKeyHelper",
            "awsAuthRefresh",
            "awsCredentialExport",
            "gcpAuthRefresh",
        ],
    );
    let connection = value
        .get("env")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(key, _)| {
            crate::live::floor::CLAUDE_PROTOCOL_SELECTORS.contains(&key.as_str())
                || (key.starts_with("CLAUDE_CODE_SKIP_") && key.ends_with("_AUTH"))
                || key.starts_with("VERTEX_REGION_")
                || [
                    "ANTHROPIC_BASE_URL",
                    "ANTHROPIC_AUTH_TOKEN",
                    "ANTHROPIC_API_KEY",
                    "ANTHROPIC_CUSTOM_HEADERS",
                    "ANTHROPIC_FOUNDRY_BASE_URL",
                    "ANTHROPIC_FOUNDRY_API_KEY",
                    "ANTHROPIC_FOUNDRY_RESOURCE",
                    "CLAUDE_CODE_OAUTH_TOKEN",
                    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
                    "CLAUDE_CODE_OAUTH_SCOPES",
                    "AWS_ACCESS_KEY_ID",
                    "AWS_SECRET_ACCESS_KEY",
                    "AWS_SESSION_TOKEN",
                    "AWS_BEARER_TOKEN_BEDROCK",
                    "AWS_PROFILE",
                    "AWS_REGION",
                    "AWS_DEFAULT_REGION",
                    "AWS_ENDPOINT_URL",
                    "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
                    "CLOUD_ML_REGION",
                    "GOOGLE_APPLICATION_CREDENTIALS",
                ]
                .contains(&key.as_str())
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    identity["env"] = Value::Object(connection);
    identity
}
fn same_json(
    path: &Path,
    patch: &dyn LivePatch,
    identity: fn(&Value) -> Value,
) -> Result<bool, AppError> {
    let before = read(path)?;
    Ok(identity(&json_value(path, &before)?)
        == identity(&json_value(path, &patch.apply(path, Some(&before))?)?))
}
fn same_toml(
    path: &Path,
    patch: &dyn LivePatch,
    identity: fn(&Value) -> Value,
) -> Result<bool, AppError> {
    let before = read(path)?;
    let parse = |bytes: &[u8]| -> Result<Value, AppError> {
        let value = std::str::from_utf8(bytes)
            .ok()
            .and_then(|s| s.parse::<toml::Value>().ok())
            .ok_or_else(|| AppError::Config(format!("{}: invalid TOML", path.display())))?;
        Ok(serde_json::to_value(value).expect("TOML value"))
    };
    Ok(identity(&parse(&before)?) == identity(&parse(&patch.apply(path, Some(&before))?)?))
}
fn codex_identity(value: &Value) -> Value {
    // An omitted selector means OpenAI. Only the selected table participates; legacy
    // and unused provider tables cannot change the active route.
    let id = value
        .get("model_provider")
        .and_then(Value::as_str)
        .unwrap_or("openai");
    let table = &value["model_providers"][id];
    let mut route = fields(
        table,
        &[
            "base_url",
            "wire_api",
            "env_key",
            "experimental_bearer_token",
            "requires_openai_auth",
            "http_headers",
            "env_http_headers",
            "query_params",
            "auth",
            "aws",
        ],
    );
    if table.is_object() {
        route["requires_openai_auth"] =
            json!(table["requires_openai_auth"].as_bool().unwrap_or(false));
        route["wire_api"] = json!(table["wire_api"].as_str().unwrap_or("responses"));
    }
    // Imports can retain the client's original table name. Aliases of the same
    // endpoint/authentication are the same provider; the selector still determines
    // WHICH table is compared.
    let provider = if table.is_object() && !id.starts_with("amazon-bedrock") {
        "custom"
    } else {
        id
    };
    json!({"provider": provider,
        "root": fields(value, &["openai_base_url", "base_url", "experimental_bearer_token", "wire_api"]),
        "route": route
    })
}
fn grok_identity(value: &Value) -> Value {
    let selected = value["models"]["default"].as_str();
    let table = selected
        .map(|id| &value["model"][id])
        .unwrap_or(&Value::Null);
    json!({"custom": selected.is_some(), "route": fields(table, &["base_url", "api_key", "api_backend", "http_headers", "env_key"])})
}
pub fn matches_live(
    env: &Env,
    data: &store::StoreData,
    tool: Tool,
    target: &Provider,
) -> Result<bool, AppError> {
    preflight(data, tool, target)?;
    match tool {
        Tool::Claude => {
            let projection = ClaudeProjection::of(&target.settings_config);
            let path = env.claude_settings_path();
            let bytes = read(&path)?;
            let live = json_value(&path, &bytes)?;
            claude::validate(&Provider::new("live", "live", live.clone()))?;
            let expected =
                direct_patch(Some(&projection), &projection).apply(&path, Some(&bytes))?;
            Ok(claude_identity(&live) == claude_identity(&json_value(&path, &expected)?))
        }
        Tool::Codex => {
            let mut planned = codex::plan(data, Some(target), target)?;
            let path = env.codex_auth_path();
            let auth = json_value(&path, &read(&path)?)?;
            if let Some(account) = target.managed_codex_account() {
                let marker = crate::codex::oauth::live::read_marker(env);
                let account = if account.is_empty() {
                    marker.as_ref().map(|m| m.account_id.as_str()).unwrap_or("")
                } else {
                    account
                };
                if !crate::codex::oauth::live::auth_matches_marker(&auth, marker.as_ref(), account)
                {
                    return Ok(false);
                }
            }
            if let Some(identity) = target.codex_login_ref() {
                if crate::codex::login::login_id(&auth).as_deref() != Some(identity) {
                    return Ok(false);
                }
            }
            if let Some(expected) = target
                .settings_config
                .get("auth")
                .and_then(|v| v.get("OPENAI_API_KEY"))
                .and_then(Value::as_str)
            {
                if target.is_official_category()
                    && auth.get("OPENAI_API_KEY").and_then(Value::as_str) != Some(expected)
                {
                    return Ok(false);
                }
            }
            let source = read(&env.codex_config_path())?;
            planned.use_login_state(
                &String::from_utf8_lossy(&source),
                data.settings.preserve_codex_official_auth_on_switch,
                crate::codex::auth::codex_auth_has_openai_account_material(&auth),
            );
            same_toml(&env.codex_config_path(), planned.config(), codex_identity)
        }
        Tool::Gemini => {
            let p = gemini::projection(target)?;
            let path = env.gemini_env_path();
            let before = read(&path)?;
            let after = p.env_patch().apply(&path, Some(&before))?;
            let entries = |bytes: &[u8]| {
                dotenv::entries(&String::from_utf8_lossy(bytes))
                    .into_iter()
                    .filter(|(key, _)| {
                        crate::live::floor::gemini_floor_env(key) && key != "GEMINI_MODEL"
                    })
                    .collect::<std::collections::BTreeMap<_, _>>()
            };
            Ok(entries(&before) == entries(&after)
                && same_json(&env.gemini_settings_path(), &p.settings_patch(), |v| {
                    v.pointer("/security/auth/selectedType")
                        .cloned()
                        .unwrap_or(json!("oauth-personal"))
                })?)
        }
        Tool::GrokBuild => {
            let p = grok::projection(target)?;
            let patch = GrokConfigPatch::direct(
                &p,
                grok::retired_tables(&env.device_store(), Some(target))?,
                crate::live::project::claude::PROXY_TOKEN_PLACEHOLDER,
            );
            same_toml(&env.grok_config_path(), &patch, grok_identity)
        }
        Tool::ClaudeDesktop => {
            for path in [
                env.claude_desktop_config_path(),
                env.claude_desktop_3p_config_path(),
            ] {
                let live = json_value(&path, &read(&path)?)?;
                if live
                    .get("deploymentMode")
                    .and_then(Value::as_str)
                    .unwrap_or("1p")
                    != if target.is_official_category() {
                        "1p"
                    } else {
                        "3p"
                    }
                {
                    return Ok(false);
                }
            }
            if target.is_official_category() {
                return Ok(true);
            }
            match claude_desktop::imported_card(env)? {
                None => Ok(false),
                Some(live) => {
                    Ok(["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"]
                        .iter()
                        .all(|key| {
                            live.settings_config["env"][key] == target.settings_config["env"][key]
                        }))
                }
            }
        }
        _ => Ok(true),
    }
}
