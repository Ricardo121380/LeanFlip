//! Machine-local consent for executable helpers and provider packages. Never exported with cards.
use crate::{fsutil, live::engine::digest, AppError, Env, Provider, Tool};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandPreview {
    pub commands: BTreeMap<String, Value>,
    pub destinations: BTreeMap<String, Value>,
    pub fingerprint: String,
    pub approved: bool,
}
const FILE: &str = "trusted-credential-commands.json";

fn commands(tool: Tool, settings: &Value) -> Result<BTreeMap<String, Value>, AppError> {
    let mut result = BTreeMap::new();
    match tool {
        Tool::Claude => {
            for field in [
                "apiKeyHelper",
                "awsAuthRefresh",
                "awsCredentialExport",
                "gcpAuthRefresh",
            ] {
                if let Some(value) = settings
                    .get(field)
                    .filter(|v| !v.is_null() && v.as_str() != Some(""))
                {
                    result.insert(field.into(), value.clone());
                }
            }
        }
        Tool::Codex => {
            let config = settings
                .get("config")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let parsed: toml::Value = config
                .parse()
                .map_err(|_| AppError::Config("Invalid Codex TOML".into()))?;
            if let Some(providers) = parsed
                .get("model_providers")
                .and_then(toml::Value::as_table)
            {
                for (id, provider) in providers {
                    if let Some(auth) = provider
                        .get("auth")
                        .filter(|auth| auth.get("command").is_some())
                    {
                        result.insert(
                            format!("model_providers.{id}.auth"),
                            serde_json::to_value(auth)
                                .map_err(|source| AppError::JsonSerialize { source })?,
                        );
                    }
                }
            }
        }
        Tool::OpenCode => package_references(settings, "", &mut result),
        _ => {}
    }
    Ok(result)
}

// Include provider- and model-level overrides, including full V1/V2 config wrappers.
fn package_references(value: &Value, prefix: &str, result: &mut BTreeMap<String, Value>) {
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            if matches!(key.as_str(), "npm" | "package") && !value.is_null() {
                result.insert(path, value.clone());
            } else {
                package_references(value, &path, result);
            }
        }
    }
}

fn destinations(value: &Value, prefix: &str, result: &mut BTreeMap<String, Value>) {
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            if matches!(
                key.as_str(),
                "apiBaseUrl" | "base_url" | "openai_base_url" | "baseURL" | "api"
            ) || key.ends_with("BASE_URL")
            {
                result.insert(path, value.clone());
            } else {
                destinations(value, &path, result);
            }
        }
    }
}
fn approved(env: &Env) -> Result<BTreeSet<String>, AppError> {
    let path = env.data_file(FILE);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| AppError::json(&path, error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(error) => Err(AppError::io(&path, error)),
    }
}
pub fn preview(env: &Env, tool: Tool, settings: &Value) -> Result<CommandPreview, AppError> {
    let commands = commands(tool, settings)?;
    // Bind consent to the execution environment AND credential destination. Versioning
    // deliberately invalidates approvals made before destination binding was introduced.
    let context = match tool {
        Tool::Claude => {
            serde_json::json!({"env": settings.get("env"), "apiBaseUrl": settings.get("apiBaseUrl")})
        }
        Tool::Codex => {
            let parsed: toml::Value = settings
                .get("config")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .parse()
                .map_err(|_| AppError::Config("Invalid Codex TOML".into()))?;
            serde_json::json!({"model_provider": parsed.get("model_provider"), "openai_base_url": parsed.get("openai_base_url"), "model_providers": parsed.get("model_providers")})
        }
        Tool::OpenCode => settings.clone(),
        _ => Value::Null,
    };
    let mut destination_preview = BTreeMap::new();
    destinations(&context, "", &mut destination_preview);
    let bytes = fsutil::sorted_json_bytes(&(2, tool, &commands, context))?;
    let fingerprint = digest(Some(&bytes)).expect("present bytes");
    let approved = commands.is_empty() || approved(env)?.contains(&fingerprint);
    Ok(CommandPreview {
        commands,
        destinations: destination_preview,
        fingerprint,
        approved,
    })
}
pub fn approve(env: &Env, tool: Tool, settings: &Value, expected: &str) -> Result<(), AppError> {
    let _guard = env.device_store().lock_app("command-trust")?;
    let preview = preview(env, tool, settings)?;
    if preview.fingerprint != expected {
        return Err(AppError::Conflict(
            "Credential commands changed; review them again".into(),
        ));
    }
    if !preview.approved {
        let mut trusted = approved(env)?;
        trusted.insert(preview.fingerprint);
        fsutil::atomic_write_private(&env.data_file(FILE), &fsutil::sorted_json_bytes(&trusted)?)?;
    }
    Ok(())
}
pub fn require(env: &Env, tool: Tool, provider: &Provider) -> Result<(), AppError> {
    if !preview(env, tool, &provider.settings_config)?.approved {
        return Err(AppError::localized(
            "provider.command_confirmation_required",
            "此卡片包含客户端会执行的凭据命令或供应商包。请编辑卡片，查看并授权后重试；CLI 先保存卡片，再使用 leanflip provider trust-commands。",
            "This card contains executable credential commands or provider packages. Edit the card to review and approve them, or save it first and use leanflip provider trust-commands, then retry.",
        ));
    }
    Ok(())
}
