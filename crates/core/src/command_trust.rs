//! Machine-local consent for executable credential helpers. Never exported with cards.
use crate::{fsutil, live::engine::digest, AppError, Env, Provider, Tool};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandPreview {
    pub commands: BTreeMap<String, Value>,
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
        _ => {}
    }
    Ok(result)
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
    // Claude helpers inherit the card's environment (PATH and script-specific variables
    // can change what an unchanged command executes). Environment changes revoke consent.
    let environment = (tool == Tool::Claude)
        .then(|| settings.get("env"))
        .flatten();
    let bytes = fsutil::sorted_json_bytes(&(tool, &commands, environment))?;
    let fingerprint = digest(Some(&bytes)).expect("present bytes");
    let approved = commands.is_empty() || approved(env)?.contains(&fingerprint);
    Ok(CommandPreview {
        commands,
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
            "此卡片包含客户端会执行的凭据命令。请编辑卡片，查看并授权命令后重试；CLI 使用 leanflip provider trust-commands。",
            "This card contains executable credential commands. Edit the card to review and approve them, or use leanflip provider trust-commands, then retry.",
        ));
    }
    Ok(())
}
