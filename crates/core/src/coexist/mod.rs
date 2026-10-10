//! 并存式工具：OpenCode、OpenClaw、Hermes、Pi、MiniMax Code（MCode）。
//!
//! 这些工具的配置里本来就能同时放多家供应商，所以没有「当前供应商」，卡片对应客户端
//! 配置里以 id 为键的一个条目，操作只有：
//! - [`write_to_live`]：把卡写进客户端配置（已有同名条目就原位替换）；
//! - [`activate`]：同上，Hermes 额外把顶层 `model:` 指向这家（`provider` 和第一个模型）；
//! - [`remove_from_live`]：从客户端配置里移出，卡留着；
//! - [`add_provider`] / [`update_provider`] / [`delete_provider`]：卡的增改删，和客户端
//!   配置保持一致（在 live 里的才同步）。
//!
//! 每次修改都是写入引擎里的一个操作（首写备份、暂存、pending 意图、崩溃恢复），
//! 和切换式工具一样；只是不改指针。条目以外的内容一律原样保留：
//! - OpenCode / OpenClaw / Pi：JSONC / JSON5，用移植来的保留格式编辑器（注释、尾逗号、
//!   缩进都在）；
//! - Hermes：YAML，只替换 `custom_providers:`（和激活时的 `model:`）这两个顶层段，
//!   其余段的文本和注释原样保留；
//! - MCode：YAML，整份重写（和 cc-switch 一致，会丢注释），写时拿 MCode 自己的目录锁。
//!
//! 新加卡时如果客户端配置里已经有同名但内容不同的条目，拒绝（不覆盖用户在客户端里
//! 配的那家）；要接管它，先导入。

pub mod hermes;
mod jsonc_entry;
pub mod mcode;
pub mod openclaw;
pub mod opencode;
pub mod pi;

use serde_json::{Map, Value};

use crate::env::Env;
use crate::error::AppError;
use crate::import::ImportReport;
use crate::live::engine::LiveFile;
use crate::live::patch::LivePatch;
use crate::operation::{AppWrite, FileChange, OperationReport};
use crate::provider::{Provider, Tool};
use crate::state::PendingTarget;
use crate::store;

/// 操作名（记在 pending 意图里）。
pub mod op {
    pub const ADD: &str = "coexist-add";
    pub const WRITE: &str = "coexist-write";
    pub const ACTIVATE: &str = "coexist-activate";
    pub const REMOVE: &str = "coexist-remove";
}

/// 对一个条目做什么。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Edit {
    Upsert(Value),
    Remove,
}

/// 写入时遇到同名条目怎么办。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteMode {
    /// 没有或内容相同才写；有不同内容的同名条目就拒绝（新加卡）。
    CreateOnly,
    /// 原位替换（卡本来就在 live 里）。
    Replace,
}

/// 一次条目修改。
pub(crate) struct EntryChange<'a> {
    pub provider: Option<&'a Provider>,
    pub id: &'a str,
    pub edit: Edit,
    pub mode: WriteMode,
    /// Hermes：顺带把顶层 `model:` 指向这家。
    pub activate: bool,
}

fn ensure_coexist(tool: Tool) -> Result<(), AppError> {
    if tool.is_coexist() {
        Ok(())
    } else {
        Err(AppError::InvalidInput(format!("{tool} 不是并存式工具")))
    }
}

pub(crate) fn rejected(error: AppError) -> crate::live::patch::LiveWriteError {
    crate::live::patch::LiveWriteError::Rejected(Box::new(error))
}

pub(crate) fn key_conflict(tool: Tool, id: &str) -> AppError {
    AppError::localized(
        "coexist.key_conflict",
        format!("{tool} 的配置里已经有一个叫「{id}」的供应商，内容和这张卡不同。为避免覆盖，没有写入；要接管它请先导入"),
        format!("{tool} config already has a different provider named '{id}'. Nothing was written; import it first to take it over"),
    )
}

fn live_path(env: &Env, tool: Tool) -> std::path::PathBuf {
    match tool {
        Tool::OpenCode => env.opencode_config_path(),
        Tool::OpenClaw => env.openclaw_config_path(),
        Tool::Hermes => env.hermes_config_path(),
        Tool::Pi => env.pi_models_path(),
        Tool::Mcode => env.mcode_config_path(),
        _ => unreachable!("not a coexist tool"),
    }
}

fn live_file(env: &Env, tool: Tool) -> LiveFile {
    let path = live_path(env, tool);
    match tool {
        // 和 cc-switch 一样按私有文件写（里面是 Key）。
        Tool::Pi | Tool::Mcode => LiveFile::private(path),
        _ => LiveFile::shared(path),
    }
}

/// 客户端配置里现在有哪些条目（id → 条目）。只读，不写客户端文件。
pub fn live_entries(env: &Env, tool: Tool) -> Result<Map<String, Value>, AppError> {
    ensure_coexist(tool)?;
    let path = live_path(env, tool);
    let bytes = crate::live::engine::read_current(&path).map_err(AppError::from)?;
    let text = match &bytes {
        Some(bytes) => std::str::from_utf8(bytes)
            .map_err(|_| AppError::Config(format!("{} 不是 UTF-8 文本", path.display())))?,
        None => "",
    };
    Ok(match tool {
        Tool::OpenCode => opencode::entries(&path, text)?
            .into_iter()
            .map(|(id, (value, _))| (id, value))
            .collect(),
        Tool::OpenClaw => openclaw::entries(&path, text)?,
        Tool::Hermes => hermes::entries(text)?,
        Tool::Pi => pi::entries(&path, text)?,
        Tool::Mcode => mcode::entries(text)?,
        _ => unreachable!(),
    })
}

/// 只校验、不写：这张卡能不能写进客户端配置。
pub fn validate(tool: Tool, provider: &Provider) -> Result<(), AppError> {
    ensure_coexist(tool)?;
    match tool {
        Tool::OpenCode => opencode::validate(provider),
        Tool::OpenClaw => openclaw::validate(provider),
        Tool::Hermes => hermes::validate(provider),
        Tool::Pi => pi::validate(provider),
        Tool::Mcode => mcode::validate(&provider.id, &provider.settings_config),
        _ => unreachable!(),
    }
}

fn patch_for<'a>(tool: Tool, change: &'a EntryChange<'a>) -> Box<dyn LivePatch + 'a> {
    match tool {
        Tool::OpenCode => Box::new(opencode::patch(change)),
        Tool::OpenClaw => Box::new(openclaw::patch(change)),
        Tool::Hermes => Box::new(hermes::HermesPatch { change }),
        Tool::Pi => Box::new(pi::patch(change)),
        Tool::Mcode => Box::new(mcode::McodePatch { change }),
        _ => unreachable!(),
    }
}

fn run(
    env: &Env,
    tool: Tool,
    op: &str,
    change: EntryChange<'_>,
) -> Result<OperationReport, AppError> {
    let commit = |target: &PendingTarget| crate::switch::commit_target(env, tool, target);
    let write = AppWrite::begin(env.device_store(), tool.as_str(), &commit)?;
    // MCode 自己写配置时用一个目录锁（proper-lockfile）；拿着它写，不和 MCode 打架。
    let _mcode_lock = if tool == Tool::Mcode {
        Some(mcode::DirLock::acquire(&env.mcode_config_path())?)
    } else {
        None
    };
    let patch = patch_for(tool, &change);
    write.run(
        op,
        &[FileChange {
            file: live_file(env, tool),
            patch: patch.as_ref(),
        }],
        PendingTarget::pointer(None),
    )
}

fn card(env: &Env, tool: Tool, id: &str) -> Result<Provider, AppError> {
    crate::switch::settle(env, tool)?;
    store::load(env)?
        .provider(tool, id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("{tool} 供应商 {id}")))
}

/// 把卡写进客户端配置（同名条目原位替换）。
pub fn write_to_live(env: &Env, tool: Tool, id: &str) -> Result<OperationReport, AppError> {
    ensure_coexist(tool)?;
    let provider = card(env, tool, id)?;
    validate(tool, &provider)?;
    run(
        env,
        tool,
        op::WRITE,
        upsert(&provider, WriteMode::Replace, false),
    )
}

/// 「切换」到这张卡：写进客户端配置；Hermes 还把顶层 `model:` 指向它。
pub fn activate(env: &Env, tool: Tool, id: &str) -> Result<OperationReport, AppError> {
    ensure_coexist(tool)?;
    let provider = card(env, tool, id)?;
    validate(tool, &provider)?;
    run(
        env,
        tool,
        op::ACTIVATE,
        upsert(&provider, WriteMode::Replace, tool == Tool::Hermes),
    )
}

fn upsert(provider: &Provider, mode: WriteMode, activate: bool) -> EntryChange<'_> {
    EntryChange {
        provider: Some(provider),
        id: &provider.id,
        edit: Edit::Upsert(provider.settings_config.clone()),
        mode,
        activate,
    }
}

/// 从客户端配置里移出（卡留着）。不在里面时什么都不做，返回 `None`。
pub fn remove_from_live(
    env: &Env,
    tool: Tool,
    id: &str,
) -> Result<Option<OperationReport>, AppError> {
    ensure_coexist(tool)?;
    crate::switch::settle(env, tool)?;
    if !live_entries(env, tool)?.contains_key(id) {
        return Ok(None);
    }
    run(
        env,
        tool,
        op::REMOVE,
        EntryChange {
            provider: None,
            id,
            edit: Edit::Remove,
            mode: WriteMode::Replace,
            activate: false,
        },
    )
    .map(Some)
}

/// 新加一张卡；`add_to_live` 时同时写进客户端配置（同名不同内容的条目会被拒绝）。
/// 写客户端配置失败时库里不加；库写失败时把刚写进去的条目撤掉。
pub fn add_provider(
    env: &Env,
    tool: Tool,
    provider: Provider,
    add_to_live: bool,
) -> Result<Option<OperationReport>, AppError> {
    ensure_coexist(tool)?;
    crate::switch::settle(env, tool)?;
    validate(tool, &provider)?;
    if store::load(env)?.provider(tool, &provider.id).is_some() {
        return Err(AppError::InvalidInput(format!(
            "{tool} 已经有 id 为 {} 的供应商",
            provider.id
        )));
    }
    let report = if add_to_live {
        let existed = live_entries(env, tool)?.contains_key(&provider.id);
        let report = run(
            env,
            tool,
            op::ADD,
            upsert(&provider, WriteMode::CreateOnly, false),
        )?;
        Some((report, existed))
    } else {
        None
    };
    if let Err(error) = store::add_provider(env, tool, provider.clone()) {
        if let Some((_, false)) = report {
            let _ = remove_from_live(env, tool, &provider.id);
        }
        return Err(error);
    }
    Ok(report.map(|(report, _)| report))
}

/// 编辑一张卡。卡在客户端配置里时同时改写那个条目；写入被拒绝时库里也不改。
pub fn update_provider(
    env: &Env,
    tool: Tool,
    provider: Provider,
) -> Result<Option<OperationReport>, AppError> {
    ensure_coexist(tool)?;
    let _old = card(env, tool, &provider.id)?;
    validate(tool, &provider)?;
    let report = if live_entries(env, tool)?.contains_key(&provider.id) {
        Some(run(
            env,
            tool,
            op::WRITE,
            upsert(&provider, WriteMode::Replace, false),
        )?)
    } else {
        None
    };
    store::replace_provider(env, tool, provider)?;
    Ok(report)
}

/// 删除一张卡：先从客户端配置里移出（在里面的话），再删卡。
pub fn delete_provider(env: &Env, tool: Tool, id: &str) -> Result<Provider, AppError> {
    ensure_coexist(tool)?;
    card(env, tool, id)?;
    remove_from_live(env, tool, id)?;
    store::remove_provider(env, tool, id)
}

/// 按客户端配置补卡：live 里有、库里没有的条目各建一张卡（可以重复调用，已有的卡
/// 不动）。只读客户端文件。
pub fn import_live(env: &Env, tool: Tool) -> Result<ImportReport, AppError> {
    ensure_coexist(tool)?;
    crate::switch::settle(env, tool)?;
    let cards: Vec<Provider> = match tool {
        Tool::OpenCode => {
            let path = live_path(env, tool);
            let bytes = crate::live::engine::read_current(&path).map_err(AppError::from)?;
            let text = bytes
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            opencode::entries(&path, &text)?
                .into_iter()
                .map(|(id, (value, format))| opencode::imported_card(&id, value, format))
                .collect()
        }
        _ => live_entries(env, tool)?
            .into_iter()
            .map(|(id, value)| imported_card(tool, &id, value))
            .collect(),
    };
    store::update(env, |data| {
        let app = data.app_mut(tool);
        let mut created = Vec::new();
        for card in cards {
            if app.get(&card.id).is_none() {
                created.push(card.id.clone());
                app.providers.push(card);
            }
        }
        Ok(ImportReport {
            created,
            current: None,
        })
    })
}

fn imported_card(tool: Tool, id: &str, mut value: Value) -> Provider {
    if tool == Tool::Hermes {
        if let Some(obj) = value.as_object_mut() {
            // 写回时按 id 重算，不进卡。
            obj.shift_remove("model");
        }
    }
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(id)
        .to_string();
    Provider::new(id, name, value)
}

#[cfg(test)]
mod tests {
    //! 并存式写入的崩溃注入：发布后崩溃前滚，发布前崩溃丢弃；都不碰指针。
    use super::*;
    use crate::operation::{failpoint, RecoveryOutcome};
    use crate::switch::recover_on_startup;
    use serde_json::json;
    use std::fs;

    fn sandbox() -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        fs::create_dir_all(&env.hermes_dir).unwrap();
        fs::write(env.hermes_config_path(), "# mine\nterminal: {}\n").unwrap();
        store::add_provider(
            &env,
            Tool::Hermes,
            Provider::new(
                "relay",
                "Relay",
                json!({"base_url": "https://r", "models": [{"id": "m"}]}),
            ),
        )
        .unwrap();
        (dir, env)
    }

    #[test]
    fn a_crash_after_publishing_rolls_forward() {
        let (_dir, env) = sandbox();
        failpoint::crash_at(Some("published:0"));
        assert!(activate(&env, Tool::Hermes, "relay").is_err());
        failpoint::crash_at(None);
        let outcomes = recover_on_startup(&env);
        assert!(
            matches!(outcomes[..], [(_, Ok(RecoveryOutcome::RolledForward))]),
            "{outcomes:?}"
        );
        let text = fs::read_to_string(env.hermes_config_path()).unwrap();
        assert!(
            text.starts_with("# mine\n") && text.contains("provider: relay"),
            "{text}"
        );
        assert!(store::load(&env)
            .unwrap()
            .app(Tool::Hermes)
            .unwrap()
            .current
            .is_none());
    }

    #[test]
    fn a_crash_before_publishing_is_discarded() {
        let (_dir, env) = sandbox();
        failpoint::crash_at(Some("pending"));
        assert!(write_to_live(&env, Tool::Hermes, "relay").is_err());
        failpoint::crash_at(None);
        let outcomes = recover_on_startup(&env);
        assert!(
            outcomes
                .iter()
                .all(|(_, o)| matches!(o, Ok(RecoveryOutcome::Discarded))),
            "{outcomes:?}"
        );
        assert_eq!(
            fs::read_to_string(env.hermes_config_path()).unwrap(),
            "# mine\nterminal: {}\n"
        );
        write_to_live(&env, Tool::Hermes, "relay").unwrap();
        assert!(live_entries(&env, Tool::Hermes)
            .unwrap()
            .contains_key("relay"));
    }

    #[test]
    fn coexist_tools_refuse_switching_apis_and_vice_versa() {
        let (_dir, env) = sandbox();
        assert!(write_to_live(&env, Tool::Claude, "x").is_err());
        assert!(crate::switch::reapply(&env, Tool::Hermes, None).is_err());
    }
}
