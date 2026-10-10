//! 写 Gemini CLI 的 `.env` 和 `settings.json`：只替换关键字段，其余行、键和注释不碰。
//! 两个文件和指针在同一个操作里提交。
//!
//! 移植自 cc-switch `services/provider/gemini_direct.rs`（去掉数据库、编辑器全局改动）。

use serde_json::Value;

use crate::env::Env;
use crate::error::AppError;
use crate::live::engine::LiveFile;
use crate::live::project::gemini::GeminiProjection;
use crate::operation::{AppWrite, FileChange, OperationReport};
use crate::provider::{Provider, Tool};
use crate::state::PendingTarget;

/// Google 官方卡的推广标记（和 cc-switch `gemini_auth.rs` 一致）。
const GOOGLE_OFFICIAL_PARTNER_KEY: &str = "google-official";

/// 官方卡（Google 登录）：分类是官方，或按推广标记、名称认出来的 Google 官方。
pub fn is_official(provider: &Provider) -> bool {
    if provider.is_official_category() {
        return true;
    }
    if let Some(key) = provider
        .meta
        .extra
        .get("partnerPromotionKey")
        .and_then(Value::as_str)
    {
        return key.eq_ignore_ascii_case(GOOGLE_OFFICIAL_PARTNER_KEY);
    }
    let name = provider.name.to_ascii_lowercase();
    name == "google" || name.starts_with("google ")
}

fn localized(key: &'static str, zh: &str, en: &str) -> AppError {
    AppError::localized(key, zh, en)
}

/// 供应商的关键字段。第三方卡要带凭据（环境变量全空的是走 Google 登录的卡，不要求），
/// 写之前校验，校验不过什么都不写。
pub fn projection(provider: &Provider) -> Result<GeminiProjection, AppError> {
    let settings = &provider.settings_config;
    if settings.get("env").is_some_and(|env| !env.is_object()) {
        return Err(localized(
            "gemini.validation.invalid_env",
            "Gemini 配置格式错误: env 必须是对象",
            "Gemini config invalid: env must be an object",
        ));
    }
    if settings
        .get("config")
        .is_some_and(|config| !(config.is_object() || config.is_null()))
    {
        return Err(localized(
            "gemini.validation.invalid_config",
            "Gemini 配置格式错误: config 必须是对象",
            "Gemini config invalid: config must be an object",
        ));
    }
    let official = is_official(provider);
    if !official {
        let env = settings.get("env").and_then(Value::as_object);
        let non_empty = env.is_some_and(|env| env.values().any(Value::is_string));
        let has_credential = env.is_some_and(|env| {
            [
                "GEMINI_API_KEY",
                "GOOGLE_API_KEY",
                "GOOGLE_GENAI_USE_VERTEXAI",
            ]
            .iter()
            .any(|key| env.get(*key).is_some_and(Value::is_string))
        });
        if non_empty && !has_credential {
            return Err(localized(
                "gemini.validation.missing_api_key",
                "Gemini 配置缺少必需字段: GEMINI_API_KEY",
                "Gemini config missing required field: GEMINI_API_KEY",
            ));
        }
    }
    Ok(GeminiProjection::of(settings, official))
}

/// `.env`：里面有 Key，按 0600 写。
pub fn env_file(env: &Env) -> LiveFile {
    LiveFile::private(env.gemini_env_path())
}

/// `settings.json`：沿用现有权限。
pub fn settings_file(env: &Env) -> LiveFile {
    LiveFile::shared(env.gemini_settings_path())
}

pub fn write(
    env: &Env,
    target: &Provider,
    op: &str,
    pending: PendingTarget,
) -> Result<OperationReport, AppError> {
    let projection = projection(target)?;
    let env_patch = projection.env_patch();
    let settings_patch = projection.settings_patch();
    let commit = |target: &PendingTarget| super::commit_target(env, Tool::Gemini, target);
    let write = AppWrite::begin(env.device_store(), Tool::Gemini.as_str(), &commit)?;
    write.run(
        op,
        &[
            FileChange {
                file: env_file(env),
                patch: &env_patch,
            },
            FileChange {
                file: settings_file(env),
                patch: &settings_patch,
            },
        ],
        pending,
    )
}
