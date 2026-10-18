//! Gemini's dotenv and settings edits commit together.
use crate::{
    env::Env,
    error::AppError,
    live::{engine::LiveFile, project::gemini::GeminiProjection},
    operation::{AppWrite, FileChange, OperationReport},
    provider::{Provider, Tool},
    state::PendingTarget,
};
use serde_json::Value;
pub fn is_official(provider: &Provider) -> bool {
    if provider.is_official_category() {
        return true;
    }
    if let Some(partner) = provider
        .meta
        .extra
        .get("partnerPromotionKey")
        .and_then(Value::as_str)
    {
        return partner.eq_ignore_ascii_case("google-official");
    }
    let name = provider.name.to_ascii_lowercase();
    ["google", "google official"].contains(&name.as_str())
        && !provider
            .settings_config
            .get("env")
            .and_then(Value::as_object)
            .is_some_and(|env| {
                [
                    "GEMINI_API_KEY",
                    "GOOGLE_API_KEY",
                    "GOOGLE_GEMINI_BASE_URL",
                    "GOOGLE_GENAI_USE_VERTEXAI",
                ]
                .iter()
                .any(|key| {
                    env.get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty())
                })
            })
}
pub fn projection(provider: &Provider) -> Result<GeminiProjection, AppError> {
    let error = |field: &str| {
        AppError::Config(format!(
            "Gemini 配置字段无效 / Invalid Gemini field: {field}"
        ))
    };
    let settings = &provider.settings_config;
    if !settings.is_object() {
        return Err(error("settings"));
    }
    for key in ["env", "config"] {
        if settings
            .get(key)
            .is_some_and(|v| !v.is_object() && !v.is_null())
        {
            return Err(error(key));
        }
    }
    let official = is_official(provider);
    let mut result = GeminiProjection::of(settings, official);
    if !official {
        let value = |key: &str| {
            settings
                .get("env")
                .and_then(|v| v.get(key))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        let vertex = result.selected_type == Some("vertex-ai")
            || value("GOOGLE_GENAI_USE_VERTEXAI")
                .is_some_and(|s| s == "1" || s.eq_ignore_ascii_case("true"));
        if vertex {
            result.selected_type = Some("vertex-ai");
        }
        if !result.env.is_empty()
            && !vertex
            && value("GEMINI_API_KEY")
                .or_else(|| value("GOOGLE_API_KEY"))
                .is_none()
        {
            return Err(error("GEMINI_API_KEY"));
        }
    }
    Ok(result)
}
pub fn env_file(env: &Env) -> LiveFile {
    LiveFile::private(env.gemini_env_path())
}
pub fn settings_file(env: &Env) -> LiveFile {
    LiveFile::shared(env.gemini_settings_path())
}
pub fn write(
    env: &Env,
    target: &Provider,
    op: &str,
    pending: PendingTarget,
) -> Result<OperationReport, AppError> {
    let selected = projection(target)?;
    let edits = (selected.env_patch(), selected.settings_patch());
    let commit = |target: &PendingTarget| super::commit_target(env, Tool::Gemini, target);
    AppWrite::begin(env.device_store(), Tool::Gemini.as_str(), &commit)?.run(
        op,
        &[
            FileChange {
                file: env_file(env),
                patch: &edits.0,
            },
            FileChange {
                file: settings_file(env),
                patch: &edits.1,
            },
        ],
        pending,
    )
}
