//! 切换入口：把某个工具的当前供应商换成另一张卡，或把当前卡重新写进客户端文件。
//!
//! 每次写入都是 [`crate::operation`] 里的一次操作：先在内存里算好所有文件，记下
//! pending，再逐个原子发布，最后落定当前指针（写进供应商库）。中途崩溃，下次操作或启动
//! 时（[`recover_on_startup`]）按 pending 前滚或丢弃。

pub mod claude;
pub mod codex;

use crate::env::Env;
use crate::error::AppError;
use crate::operation::{self, OperationReport, RecoveryOutcome};
use crate::provider::{Provider, Tool};
use crate::state::{op, PendingTarget};
use crate::store;

/// 落定一次操作的目标状态：指针写进供应商库，写入记录写进 `live-state.json`。
/// 可以重复执行（崩溃恢复可能再跑一次）。
pub(crate) fn commit_target(env: &Env, tool: Tool, target: &PendingTarget) -> Result<(), AppError> {
    if let Some(id) = target.pointer.as_deref() {
        store::set_current(env, tool, id)?;
    }
    operation::commit_device_state(&env.device_store(), tool.as_str(), target)
}

/// 先补完这个工具上一次没做完的操作，再读指针：读到的才是落定过的状态。
pub fn settle(env: &Env, tool: Tool) -> Result<Option<RecoveryOutcome>, AppError> {
    operation::settle(&env.device_store(), tool.as_str(), &|target| {
        commit_target(env, tool, target)
    })
}

/// 启动时补完所有工具未完成的写入。要在任何写客户端文件的步骤之前调用。
pub fn recover_on_startup(env: &Env) -> Vec<(String, Result<RecoveryOutcome, AppError>)> {
    operation::recover_all(&env.device_store(), &|app, target| {
        commit_target(env, app.parse()?, target)
    })
}

/// 切到 `id` 这张卡：同一个操作里改写客户端文件、再把当前指针改成它。
///
/// 目标行有问题（比如 Codex 第三方卡没有 Key、却会让 Codex 拿 ChatGPT 登录访问第三方
/// 地址）时在算补丁阶段就报错，什么都不写、指针也不动。
pub fn switch(env: &Env, tool: Tool, id: &str) -> Result<OperationReport, AppError> {
    settle(env, tool)?;
    let data = store::load(env)?;
    let target = data
        .provider(tool, id)
        .ok_or_else(|| AppError::NotFound(format!("{tool} 供应商 {id}")))?;
    let prev = data.current(tool);
    write(env, &data, tool, prev, target, op::SWITCH, Some(id))
}

/// 把当前卡重新写进客户端文件（编辑当前卡之后），不改指针。`prev` 是 live 现在对应的
/// 那一版行（编辑前的行），用来删掉它带进来、新版不再有的独有字段。
pub fn reapply(
    env: &Env,
    tool: Tool,
    prev: Option<&Provider>,
) -> Result<OperationReport, AppError> {
    settle(env, tool)?;
    let data = store::load(env)?;
    let target = data
        .current(tool)
        .ok_or_else(|| AppError::NotFound(format!("{tool} 没有当前供应商")))?;
    write(env, &data, tool, prev, target, op::APPLY, None)
}

/// 编辑一张卡。是当前卡时同时把新内容写进客户端文件；写入被拒绝（行有问题）时库里
/// 也不改。
pub fn update_provider(
    env: &Env,
    tool: Tool,
    provider: Provider,
) -> Result<Option<OperationReport>, AppError> {
    settle(env, tool)?;
    let data = store::load(env)?;
    let old = data
        .provider(tool, &provider.id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("{tool} 供应商 {}", provider.id)))?;
    let is_current = data.current(tool).is_some_and(|c| c.id == provider.id);
    // 先校验（会被拒绝的行不进库）。
    preflight(&data, tool, &provider)?;
    store::replace_provider(env, tool, provider)?;
    if !is_current {
        return Ok(None);
    }
    match reapply(env, tool, Some(&old)) {
        Ok(report) => Ok(Some(report)),
        Err(err) => {
            // 还没开始发布就失败（并发冲突等）：库里回到旧版本，和 live 保持一致。已经
            // 开始发布的会在下次操作时前滚，行要留着。
            if operation::pending_published(&env.device_store(), tool.as_str())? != Some(true) {
                store::replace_provider(env, tool, old)?;
            }
            Err(err)
        }
    }
}

/// 只校验、不写：目标行能不能投影。
pub fn preflight(
    data: &store::StoreData,
    tool: Tool,
    provider: &Provider,
) -> Result<(), AppError> {
    match tool {
        Tool::Claude => Ok(()),
        Tool::Codex => codex::plan(data, None, provider).map(|_| ()),
    }
}

fn write(
    env: &Env,
    data: &store::StoreData,
    tool: Tool,
    prev: Option<&Provider>,
    target: &Provider,
    op: &str,
    pointer: Option<&str>,
) -> Result<OperationReport, AppError> {
    let pending = PendingTarget::pointer(pointer.map(str::to_string));
    match tool {
        Tool::Claude => claude::write(env, prev, target, op, pending),
        Tool::Codex => {
            let planned = codex::plan(data, prev, target)?;
            codex::run(env, data, op, planned, pending)
        }
    }
}
