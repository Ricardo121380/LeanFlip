//! 写 Claude Code 的 `settings.json`：只替换关键字段和独有字段，其余字节不碰。
//!
//! 移植自 cc-switch `services/provider/claude_direct.rs`。不回填、不合并通用配置片段、
//! 不注入上下文默认值：用户的设置本来就留在 live 里。

use crate::env::Env;
use crate::error::AppError;
use crate::live::engine::LiveFile;
use crate::live::project::claude::{direct_patch, ClaudeProjection};
use crate::operation::{AppWrite, FileChange, OperationReport};
use crate::provider::{Provider, Tool};
use crate::state::PendingTarget;

/// `settings.json`（旧安装可能是 `claude.json`）。里面有 Key，按 0600 写。
pub fn settings_file(env: &Env) -> LiveFile {
    LiveFile::private(env.claude_settings_path())
}

/// 从 `prev` 切到 `target`。`prev` 是 live 现在对应的供应商（当前指针指向的那家），
/// 用来删它带进来的独有字段；没有就只做残留清理。
pub fn write(
    env: &Env,
    prev: Option<&Provider>,
    target: &Provider,
    op: &str,
    pending: PendingTarget,
) -> Result<OperationReport, AppError> {
    let prev = prev.map(|provider| ClaudeProjection::of(&provider.settings_config));
    let patch = direct_patch(
        prev.as_ref(),
        &ClaudeProjection::of(&target.settings_config),
    );
    let commit = |target: &PendingTarget| super::commit_target(env, Tool::Claude, target);
    // 补丁是按调用方读到的指针算的：拿锁时又补完了上一次的操作就停下，让调用方重来。
    let write = AppWrite::begin(env.device_store(), Tool::Claude.as_str(), &commit)?;
    write.run(
        op,
        &[FileChange {
            file: settings_file(env),
            patch: &patch,
        }],
        pending,
    )
}
