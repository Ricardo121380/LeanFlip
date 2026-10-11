//! Grok model selection with recorded ownership.
use crate::{
    env::Env,
    error::AppError,
    live::{
        engine::{DeviceStore, LiveFile},
        project::{
            claude::PROXY_TOKEN_PLACEHOLDER,
            grok::{GrokConfigPatch, GrokProjection},
        },
    },
    operation::{AppWrite, FileChange, OperationReport},
    provider::{Provider, Tool},
    state::{self, PendingTarget, Written},
};
pub fn config_file(env: &Env) -> LiveFile {
    LiveFile::private(env.grok_config_path())
}
pub fn is_official(provider: &Provider) -> bool {
    provider.is_official_category()
}
pub fn projection(provider: &Provider) -> Result<GrokProjection, AppError> {
    GrokProjection::of(&provider.settings_config, is_official(provider))
}
pub fn retired_tables(
    store: &DeviceStore,
    owner: Option<&Provider>,
) -> Result<Vec<String>, AppError> {
    match state::written(store, Tool::GrokBuild.as_str())? {
        Some(record) => Ok(record.tables),
        None => Ok(owner
            .and_then(|p| projection(p).ok())
            .map(|p| p.written_tables())
            .unwrap_or_default()),
    }
}
pub fn write(
    env: &Env,
    owner: Option<&Provider>,
    target: &Provider,
    op: &str,
    mut pending: PendingTarget,
) -> Result<OperationReport, AppError> {
    let next = projection(target)?;
    let commit = |target: &PendingTarget| super::commit_target(env, Tool::GrokBuild, target);
    let transaction = AppWrite::begin(env.device_store(), Tool::GrokBuild.as_str(), &commit)?;
    let data = crate::store::load(env)?;
    let owner = if op == crate::state::op::SWITCH {
        data.current(Tool::GrokBuild)
    } else {
        owner
    };
    let patch = GrokConfigPatch::direct(
        &next,
        retired_tables(&transaction.store, owner)?,
        PROXY_TOKEN_PLACEHOLDER,
    );
    pending.written = Some(Written {
        tables: next.written_tables(),
        ..Default::default()
    });
    transaction.run(
        op,
        &[FileChange {
            file: config_file(env),
            patch: &patch,
        }],
        pending,
    )
}
