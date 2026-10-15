//! Claude Code transaction adapter.
use crate::{
    env::Env,
    error::AppError,
    live::{
        engine::LiveFile,
        project::claude::{direct_patch, ClaudeProjection},
    },
    operation::{AppWrite, FileChange, OperationReport},
    provider::{Provider, Tool},
    state::PendingTarget,
};
pub fn settings_file(env: &Env) -> LiveFile {
    LiveFile::private(env.claude_settings_path())
}
/// Validate without echoing values, which may contain credentials.
pub fn validate(provider: &Provider) -> Result<(), AppError> {
    let invalid =
        |key: &str| AppError::Config(format!("Claude 配置字段无效 / Invalid Claude field: {key}"));
    let settings = provider
        .settings_config
        .as_object()
        .ok_or_else(|| invalid("settings"))?;
    if let Some(env) = settings.get("env") {
        let env = env.as_object().ok_or_else(|| invalid("env"))?;
        for (key, value) in env {
            let selector = crate::live::floor::CLAUDE_PROTOCOL_SELECTORS.contains(&key.as_str());
            if !value.is_string() && !(selector && value.is_boolean()) {
                return Err(invalid(&format!("env.{key}")));
            }
            if key == "ANTHROPIC_BASE_URL" {
                validate_url(value.as_str().unwrap_or_default())
                    .map_err(|_| invalid("env.ANTHROPIC_BASE_URL"))?;
            }
        }
    }
    for key in [
        "model",
        "advisorModel",
        "apiBaseUrl",
        "apiKey",
        "apiKeyHelper",
    ] {
        if settings.get(key).is_some_and(|v| !v.is_string()) {
            return Err(invalid(key));
        }
    }
    if let Some(url) = settings
        .get("apiBaseUrl")
        .and_then(serde_json::Value::as_str)
    {
        validate_url(url).map_err(|_| invalid("apiBaseUrl"))?;
    }
    Ok(())
}
// Validate options being written, separately from live provider identity checks.
pub(super) fn validate_capacity(provider: &Provider) -> Result<(), AppError> {
    for key in [
        "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
        "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
        "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
    ] {
        if let Some(value) = provider
            .settings_config
            .get("env")
            .and_then(|env| env.get(key))
        {
            if !value.as_str().is_some_and(|s| {
                !s.is_empty()
                    && s.bytes().all(|c| c.is_ascii_digit())
                    && s.parse::<u64>().is_ok_and(|n| {
                        n > 0 && (key != "CLAUDE_CODE_MAX_OUTPUT_TOKENS" || n <= 128000)
                    })
            }) {
                return Err(AppError::Config(format!(
                    "Claude 配置字段无效 / Invalid Claude field: env.{key}"
                )));
            }
        }
    }
    Ok(())
}
fn validate_url(value: &str) -> Result<(), ()> {
    if value.is_empty() {
        return Ok(());
    }
    let Some((scheme, rest)) = value.split_once("://") else {
        return Err(());
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest[..end].rsplit('@').next().unwrap_or("");
    let normalized = format!("{scheme}://{authority}{}", &rest[end..]);
    if value.chars().any(|c| c.is_control() || c.is_whitespace())
        || !crate::coexist::mcode::http_address(&normalized)
    {
        return Err(());
    }
    Ok(())
}
pub fn write(
    env: &Env,
    prev: Option<&Provider>,
    target: &Provider,
    op: &str,
    pending: PendingTarget,
) -> Result<OperationReport, AppError> {
    validate(target)?;
    validate_capacity(target)?;
    let commit = |target: &PendingTarget| super::commit_target(env, Tool::Claude, target);
    let transaction = AppWrite::begin(env.device_store(), Tool::Claude.as_str(), &commit)?;
    // A direct core caller may have waited behind another switch. Read the
    // outgoing owner only after taking the tool lock; reapply retains its old row.
    let data = crate::store::load(env)?;
    let prev = if op == crate::state::op::SWITCH {
        data.current(Tool::Claude)
    } else {
        prev
    };
    let outgoing = prev.map(|p| ClaudeProjection::of(&p.settings_config));
    let next = ClaudeProjection::of(&target.settings_config);
    let patch = direct_patch(outgoing.as_ref(), &next);
    transaction.run(
        op,
        &[FileChange {
            file: settings_file(env),
            patch: &patch,
        }],
        pending,
    )
}
