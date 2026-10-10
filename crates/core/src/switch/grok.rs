//! 写 Grok Build 的 `config.toml`：只替换 `models.default` 和我们写的那张模型表，其余
//! 字节不碰。文件、指针和写入记录在同一个操作里提交。
//!
//! 移植自 cc-switch `services/provider/grok_direct.rs`（去掉数据库、编辑器全局改动）。

use crate::env::Env;
use crate::error::AppError;
use crate::live::engine::{DeviceStore, LiveFile};
use crate::live::project::claude::PROXY_TOKEN_PLACEHOLDER;
use crate::live::project::grok::{GrokConfigPatch, GrokProjection};
use crate::operation::{AppWrite, FileChange, OperationReport};
use crate::provider::{Provider, Tool};
use crate::state::{self, PendingTarget, Written};

/// `~/.grok/config.toml`：模型表里有 Key，按 0600 写。
pub fn config_file(env: &Env) -> LiveFile {
    LiveFile::private(env.grok_config_path())
}

/// 官方卡：不写模型表，Grok 回落到内置模型和自带的 xAI 登录。
pub fn is_official(provider: &Provider) -> bool {
    provider.is_official_category()
}

/// 供应商要写的模型表。写之前校验，校验不过什么都不写。
pub fn projection(provider: &Provider) -> Result<GrokProjection, AppError> {
    GrokProjection::of(&provider.settings_config, is_official(provider))
}

/// 上次写进 live 的表。有写入记录就按记录；这台设备上还没写过时，按 live 现在对应的
/// 那张卡推断（从 cc-switch 迁过来、或者首次导入后第一次切换）。用户自己的其他表证明
/// 不了是我们写的，不删。
pub fn retired_tables(
    store: &DeviceStore,
    live_owner: Option<&Provider>,
) -> Result<Vec<String>, AppError> {
    if let Some(written) = state::written(store, Tool::GrokBuild.as_str())? {
        return Ok(written.tables);
    }
    let Some(owner) = live_owner.filter(|owner| !is_official(owner)) else {
        return Ok(Vec::new());
    };
    if let Ok(projection) = projection(owner) {
        return Ok(projection.written_tables());
    }
    Ok(owner
        .settings_config
        .get("config")
        .and_then(|config| config.as_str())
        .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|doc| {
            doc.get("models")
                .and_then(|models| models.get("default"))
                .and_then(toml_edit::Item::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
        .into_iter()
        .collect())
}

pub fn write(
    env: &Env,
    live_owner: Option<&Provider>,
    target: &Provider,
    op: &str,
    mut pending: PendingTarget,
) -> Result<OperationReport, AppError> {
    let projection = projection(target)?;
    let commit = |target: &PendingTarget| super::commit_target(env, Tool::GrokBuild, target);
    // 先补完上一次没做完的操作：它可能改了写入记录和指针，下面要按最新的记录删表。
    let write = AppWrite::begin(env.device_store(), Tool::GrokBuild.as_str(), &commit)?;
    pending.written = Some(Written {
        tables: projection.written_tables(),
        ..Written::default()
    });
    let patch = GrokConfigPatch::direct(
        &projection,
        retired_tables(&write.store, live_owner)?,
        PROXY_TOKEN_PLACEHOLDER,
    );
    write.run(
        op,
        &[FileChange {
            file: config_file(env),
            patch: &patch,
        }],
        pending,
    )
}
