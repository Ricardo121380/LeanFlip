//! Provider-map transactions shared by OpenCode, OpenClaw, Hermes, Pi and MCode.
pub mod hermes;
mod jsonc_entry;
pub mod mcode;
pub mod openclaw;
pub mod opencode;
pub mod pi;
pub(crate) mod yaml_sections;

use serde_json::{Map, Value};

use crate::env::Env;
use crate::error::AppError;
use crate::import::ImportReport;
use crate::live::engine::LiveFile;
use crate::live::patch::LivePatch;
use crate::operation::{AppWrite, FileChange, OperationReport};
use crate::provider::{Provider, Tool};
use crate::state::{PendingTarget, ProviderChange};
use crate::store;

/// 操作名（记在 pending 意图里）。
pub mod op {
    pub const ADD: &str = "coexist-add";
    pub const WRITE: &str = "coexist-write";
    pub const ACTIVATE: &str = "coexist-activate";
    pub const REMOVE: &str = "coexist-remove";
    pub const RESTORE_MODEL: &str = "coexist-restore-model";
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
        Err(AppError::localized(
            "tool.not_coexist",
            format!("{tool} 不是并存式工具"),
            format!("{tool} is not a coexisting tool"),
        ))
    }
}

pub(crate) fn rejected(error: AppError) -> crate::live::patch::LiveWriteError {
    crate::live::patch::LiveWriteError::Rejected(Box::new(error))
}

pub(crate) fn key_conflict(tool: Tool, id: &str) -> AppError {
    AppError::localized(
        "coexist.key_conflict",
        format!("{tool} 的配置里已经有一个叫「{id}」的供应商，内容和这个供应商不同。为避免覆盖，没有写入；要接管它请先导入"),
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
    LiveFile::private(live_path(env, tool))
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

/// 客户端配置里「默认」的那一家。只有 Hermes 有（顶层 `model.provider`），其他工具是 `None`。
pub fn active_entry(env: &Env, tool: Tool) -> Result<Option<String>, AppError> {
    ensure_coexist(tool)?;
    if tool != Tool::Hermes {
        return Ok(None);
    }
    let path = live_path(env, tool);
    let bytes = crate::live::engine::read_current(&path).map_err(AppError::from)?;
    let text = match &bytes {
        Some(bytes) => std::str::from_utf8(bytes)
            .map_err(|_| AppError::Config(format!("{} 不是 UTF-8 文本", path.display())))?,
        None => "",
    };
    hermes::active(text)
}

fn live_text(env: &Env, tool: Tool) -> Result<String, AppError> {
    let path = live_path(env, tool);
    let bytes = crate::live::engine::read_current(&path).map_err(AppError::from)?;
    match bytes {
        Some(bytes) => String::from_utf8(bytes)
            .map_err(|_| AppError::Config(format!("{} 不是 UTF-8 文本", path.display()))),
        None => Ok(String::new()),
    }
}

/// Hermes 顶层 `model:` 整段（默认供应商和模型）。其他工具是 `None`。
pub fn hermes_model(env: &Env) -> Result<Option<Value>, AppError> {
    hermes::model(&live_text(env, Tool::Hermes)?)
}

/// 把 Hermes 顶层 `model:` 整段写回成 `model`，其他段不动。撤销场景时用。
pub fn restore_hermes_model(env: &Env, model: &Value) -> Result<OperationReport, AppError> {
    let commit = |target: &PendingTarget| crate::switch::commit_target(env, Tool::Hermes, target);
    let write = AppWrite::begin(env.device_store(), Tool::Hermes.as_str(), &commit)?;
    let patch = hermes::ModelPatch { model };
    write.run(
        op::RESTORE_MODEL,
        &[FileChange {
            file: live_file(env, Tool::Hermes),
            patch: &patch,
        }],
        PendingTarget::pointer(None),
    )
}

/// 只校验、不写：这张卡能不能写进客户端配置。
pub fn validate(tool: Tool, provider: &Provider) -> Result<(), AppError> {
    ensure_coexist(tool)?;
    crate::config_shortcuts::provider(tool, &provider.settings_config)?;
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
    providers: Vec<ProviderChange>,
) -> Result<OperationReport, AppError> {
    if let Some(provider) = change.provider {
        crate::command_trust::require(env, tool, provider)?;
    }
    let commit = |target: &PendingTarget| crate::switch::commit_target(env, tool, target);
    let write = AppWrite::begin(env.device_store(), tool.as_str(), &commit)?;
    store::apply_changes(&mut store::load(env)?, &providers)?;
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
        PendingTarget {
            providers,
            ..PendingTarget::default()
        },
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
        Vec::new(),
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
        Vec::new(),
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
        Vec::new(),
    )
    .map(Some)
}

/// 新加一张卡；`add_to_live` 时同时写进客户端配置（同名不同内容的条目会被拒绝）。
/// 客户端写入和卡片变更属于同一恢复日志；发布后中断会在重试或启动时完成。
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
    if add_to_live {
        run(
            env,
            tool,
            op::ADD,
            upsert(&provider, WriteMode::CreateOnly, false),
            vec![ProviderChange {
                tool,
                before: None,
                after: Some(provider.clone()),
            }],
        )
        .map(Some)
    } else {
        store::add_provider(env, tool, provider)?;
        Ok(None)
    }
}

/// 编辑一张卡。卡在客户端配置里时同时改写那个条目；写入被拒绝时库里也不改。
pub fn update_provider(
    env: &Env,
    tool: Tool,
    provider: Provider,
) -> Result<Option<OperationReport>, AppError> {
    ensure_coexist(tool)?;
    let old = card(env, tool, &provider.id)?;
    validate(tool, &provider)?;
    if live_entries(env, tool)?.contains_key(&provider.id) {
        run(
            env,
            tool,
            op::WRITE,
            upsert(&provider, WriteMode::Replace, false),
            vec![ProviderChange {
                tool,
                before: Some(old),
                after: Some(provider.clone()),
            }],
        )
        .map(Some)
    } else {
        store::replace_provider(env, tool, provider)?;
        Ok(None)
    }
}

/// 删除一张卡：先从客户端配置里移出（在里面的话），再删卡。
pub fn delete_provider(env: &Env, tool: Tool, id: &str) -> Result<Provider, AppError> {
    ensure_coexist(tool)?;
    let old = card(env, tool, id)?;
    if live_entries(env, tool)?.contains_key(id) {
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
            vec![ProviderChange {
                tool,
                before: Some(old.clone()),
                after: None,
            }],
        )?;
        Ok(old)
    } else {
        store::remove_provider(env, tool, id)
    }
}

/// 按客户端配置补卡：live 里有、库里没有的条目各建一张卡（可以重复调用，已有的卡
/// 不动）。只读客户端文件。
pub fn import_live(env: &Env, tool: Tool) -> Result<ImportReport, AppError> {
    ensure_coexist(tool)?;
    crate::switch::settle(env, tool)?;
    let mut warnings = Vec::new();
    let cards: Vec<Provider> = match tool {
        Tool::OpenCode => {
            let path = live_path(env, tool);
            let bytes = crate::live::engine::read_current(&path).map_err(AppError::from)?;
            let text = bytes
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            let (entries, problems) = opencode::import_entries(&path, &text)?;
            warnings = problems;
            entries
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
            warnings,
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
mod transaction_tests;
#[cfg(test)]
mod tests {
    //! 并存式写入的崩溃注入：发布后崩溃前滚，发布前崩溃丢弃；都不碰指针。
    use super::*;
    use crate::operation::{failpoint, RecoveryOutcome};
    use crate::switch::recover_on_startup;
    use serde_json::json;
    use std::fs;

    #[test]
    fn validation_errors_are_bilingual() {
        let bad = [
            (Tool::Mcode, Provider::new("bad key!", "x", json!({}))),
            (Tool::Mcode, Provider::new("k", "x", json!({"api": "nope"}))),
            (Tool::Pi, Provider::new("p", "x", json!([]))),
            (Tool::OpenClaw, Provider::new("o", "x", json!({"x": 1}))),
            (Tool::Hermes, Provider::new("h", "x", json!(1))),
        ];
        for (tool, provider) in bad {
            match validate(tool, &provider) {
                Err(AppError::Localized { zh, en, .. }) => {
                    assert!(!zh.is_ascii(), "{tool}: {zh}");
                    assert!(en.is_ascii(), "{tool}: {en}");
                }
                other => panic!("{tool}: expected a localized error, got {other:?}"),
            }
        }
        assert!(matches!(
            validate(Tool::Claude, &Provider::new("c", "x", json!({}))),
            Err(AppError::Localized { .. })
        ));
    }

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
