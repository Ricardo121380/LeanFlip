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
pub fn preflight(data: &store::StoreData, tool: Tool, provider: &Provider) -> Result<(), AppError> {
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

#[cfg(test)]
mod tests {
    //! 崩溃恢复和并发刷新（用 `operation::failpoint` 注入），全部在临时 HOME 里。
    use super::*;
    use crate::codex::auth::test_codex_id_token;
    use crate::operation::failpoint;
    use serde_json::{json, Value};
    use std::fs;

    fn login(refresh: &str) -> Value {
        json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": test_codex_id_token("alice"),
                "access_token": "access",
                "refresh_token": refresh,
                "account_id": "acc-plus",
            },
        })
    }

    fn codex_sandbox() -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        store::add_provider(
            &env,
            Tool::Codex,
            Provider::new("official", "Official", json!({"auth": {}, "config": ""})).official(),
        )
        .unwrap();
        store::add_provider(
            &env,
            Tool::Codex,
            Provider::new(
                "relay",
                "Relay",
                json!({
                    "auth": {"OPENAI_API_KEY": "sk-relay"},
                    "config": "model_provider = \"relay\"\n\n[model_providers.relay]\nname = \"Relay\"\nbase_url = \"https://relay.example/v1\"\n",
                }),
            ),
        )
        .unwrap();
        store::set_current(&env, Tool::Codex, "official").unwrap();
        fs::create_dir_all(&env.codex_dir).unwrap();
        fs::write(env.codex_config_path(), "model = \"gpt-5.5\"\n").unwrap();
        fs::write(env.codex_auth_path(), login("refresh-1").to_string()).unwrap();
        (dir, env)
    }

    fn current(env: &Env, tool: Tool) -> Option<String> {
        store::load(env).unwrap().app(tool).unwrap().current.clone()
    }

    /// auth.json 已经删掉、登录还没写进暂存时进程崩溃：启动时前滚，登录进了暂存，
    /// 指针落定到目标卡。登录不会丢。
    #[test]
    fn a_crash_between_deleting_auth_json_and_stashing_rolls_forward() {
        let (_dir, env) = codex_sandbox();
        failpoint::crash_at(Some("published:0"));
        let result = switch(&env, Tool::Codex, "relay");
        failpoint::crash_at(None);
        assert!(result.is_err());
        assert!(
            !env.codex_auth_path().exists(),
            "auth.json was published (deleted)"
        );
        let stash = env.data_file(crate::codex::login::STASH_FILENAME);
        assert!(!stash.exists(), "the stash was not written yet");
        assert_eq!(current(&env, Tool::Codex).as_deref(), Some("official"));

        let outcomes = recover_on_startup(&env);
        assert_eq!(outcomes.len(), 1);
        assert!(
            matches!(outcomes[0].1, Ok(RecoveryOutcome::RolledForward)),
            "{outcomes:?}"
        );
        let stash: Value = serde_json::from_slice(&fs::read(&stash).unwrap()).unwrap();
        assert_eq!(
            stash["logins"]["sub:alice|account:acc-plus"]["tokens"]["refresh_token"],
            "refresh-1"
        );
        assert!(fs::read_to_string(env.codex_config_path())
            .unwrap()
            .contains("sk-relay"));
        assert_eq!(current(&env, Tool::Codex).as_deref(), Some("relay"));
        assert!(crate::state::pending(&env.device_store(), "codex")
            .unwrap()
            .is_none());
    }

    /// 发布前 Codex CLI 刚好刷新了 auth.json：auth.json 排第一个、带写前 hash 守卫，
    /// 整个切换在发布任何文件之前停下，新登录不会被删，也不会把旧登录存进暂存。
    #[test]
    fn a_codex_refresh_right_before_publishing_aborts_the_whole_switch() {
        let (_dir, env) = codex_sandbox();
        let auth_path = env.codex_auth_path();
        let refreshed = login("refresh-2").to_string();
        let hook_path = auth_path.clone();
        let hook_bytes = refreshed.clone();
        failpoint::on_before_publish(Some(Box::new(move |index, path| {
            if index == 0 && path == hook_path {
                fs::write(path, &hook_bytes).unwrap();
            }
        })));
        let result = switch(&env, Tool::Codex, "relay");
        failpoint::on_before_publish(None);
        assert!(result.is_err(), "must stop: {result:?}");
        assert_eq!(fs::read_to_string(&auth_path).unwrap(), refreshed);
        assert_eq!(
            fs::read_to_string(env.codex_config_path()).unwrap(),
            "model = \"gpt-5.5\"\n"
        );
        assert!(!env.data_file(crate::codex::login::STASH_FILENAME).exists());
        assert_eq!(current(&env, Tool::Codex).as_deref(), Some("official"));
        assert!(crate::state::pending(&env.device_store(), "codex")
            .unwrap()
            .is_none());
    }

    /// 文件都写完、指针还没落定时崩溃：下一次操作前先补完，指针跟上文件。
    #[test]
    fn a_crash_before_the_pointer_is_committed_is_completed_by_the_next_operation() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        for (id, url) in [("a", "https://a.example"), ("b", "https://b.example")] {
            store::add_provider(
                &env,
                Tool::Claude,
                Provider::new(id, id, json!({"env": {"ANTHROPIC_BASE_URL": url}})),
            )
            .unwrap();
        }
        store::set_current(&env, Tool::Claude, "a").unwrap();
        // settings.json 已发布、指针还没落定时崩溃。
        failpoint::crash_at(Some("published:0"));
        assert!(switch(&env, Tool::Claude, "b").is_err());
        failpoint::crash_at(None);
        let live = fs::read_to_string(env.claude_settings_path()).unwrap();
        assert!(live.contains("https://b.example"));
        assert_eq!(current(&env, Tool::Claude).as_deref(), Some("a"));

        // settle（每个入口都会先调）补完上一次的操作。
        assert!(matches!(
            settle(&env, Tool::Claude).unwrap(),
            Some(RecoveryOutcome::RolledForward)
        ));
        assert_eq!(current(&env, Tool::Claude).as_deref(), Some("b"));
    }

    /// 编辑当前卡：新内容写进 live；被拒绝的编辑不进库。
    #[test]
    fn editing_the_current_card_reapplies_it() {
        let (_dir, env) = codex_sandbox();
        switch(&env, Tool::Codex, "relay").unwrap();
        let mut edited = store::load(&env)
            .unwrap()
            .provider(Tool::Codex, "relay")
            .unwrap()
            .clone();
        edited.settings_config["auth"]["OPENAI_API_KEY"] = json!("sk-rotated");
        assert!(update_provider(&env, Tool::Codex, edited)
            .unwrap()
            .is_some());
        let live = fs::read_to_string(env.codex_config_path()).unwrap();
        assert!(
            live.contains("sk-rotated") && !live.contains("sk-relay"),
            "{live}"
        );

        let mut broken = store::load(&env)
            .unwrap()
            .provider(Tool::Codex, "relay")
            .unwrap()
            .clone();
        broken.settings_config["auth"] = json!({});
        broken.settings_config["config"] =
            json!("model = \"m\"\nopenai_base_url = \"https://x.example/v1\"\n");
        assert!(update_provider(&env, Tool::Codex, broken).is_err());
        let row = store::load(&env)
            .unwrap()
            .provider(Tool::Codex, "relay")
            .unwrap()
            .clone();
        assert_eq!(row.settings_config["auth"]["OPENAI_API_KEY"], "sk-rotated");
    }
}
