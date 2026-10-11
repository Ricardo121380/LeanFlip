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
pub fn write(
    env: &Env,
    prev: Option<&Provider>,
    target: &Provider,
    op: &str,
    pending: PendingTarget,
) -> Result<OperationReport, AppError> {
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
