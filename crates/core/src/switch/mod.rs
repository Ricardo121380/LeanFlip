//! 切换入口：把某个工具的当前供应商换成另一张卡，或把当前卡重新写进客户端文件。
//!
//! 每次写入都是 [`crate::operation`] 里的一次操作：先在内存里算好所有文件，记下
//! pending，再逐个原子发布，最后落定当前指针（写进供应商库）。中途崩溃，下次操作或启动
//! 时（[`recover_on_startup`]）按 pending 前滚或丢弃。
//!
//! 调用方必须在整个高层写操作期间持有 `env.device_store().lock_app("service")`，
//! 包括卡片编辑、OAuth 准备和发布；App / CLI / TUI 由 Service 保证这一点。
//! 工具事务还会在锁内刷新或验证上一供应商快照，拒绝使用并发切换前的旧所有者。

pub mod claude;
pub mod claude_desktop;
pub mod codex;
pub mod gemini;
pub mod grok;
pub mod status;

use crate::codex::oauth::CodexOAuthManager;
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

/// 切换要用到的长生命周期服务（桌面应用里只建一次）。
#[derive(Default, Clone, Copy)]
pub struct Services<'a> {
    /// 托管 ChatGPT 账号。没有时涉及托管账号卡的 Codex 切换会被拒绝。
    pub codex_oauth: Option<&'a CodexOAuthManager>,
}

/// 切到 `id` 这张卡：同一个操作里改写客户端文件、再把当前指针改成它。
///
/// 目标行有问题（比如 Codex 第三方卡没有 Key、却会让 Codex 拿 ChatGPT 登录访问第三方
/// 地址）时在算补丁阶段就报错，什么都不写、指针也不动。
pub fn switch(env: &Env, tool: Tool, id: &str) -> Result<OperationReport, AppError> {
    switch_with(env, Services::default(), tool, id)
}

/// 同 [`switch`]，带上托管账号管理器等服务。
pub fn switch_with(
    env: &Env,
    services: Services<'_>,
    tool: Tool,
    id: &str,
) -> Result<OperationReport, AppError> {
    if tool.is_coexist() {
        // 并存式工具没有当前指针：「切换」= 写进客户端配置（Hermes 还改顶层 model:）。
        return crate::coexist::activate(env, tool, id);
    }
    settle(env, tool)?;
    let data = store::load(env)?;
    let target = data
        .provider(tool, id)
        .ok_or_else(|| AppError::NotFound(format!("{tool} 供应商 {id}")))?;
    let prev = data.current(tool);
    write(
        env,
        services,
        &data,
        tool,
        prev,
        target,
        op::SWITCH,
        Some(id),
    )
}

/// 把当前卡重新写进客户端文件（编辑当前卡之后），不改指针。`prev` 是 live 现在对应的
/// 那一版行（编辑前的行），用来删掉它带进来、新版不再有的独有字段。
pub fn reapply(
    env: &Env,
    tool: Tool,
    prev: Option<&Provider>,
) -> Result<OperationReport, AppError> {
    reapply_with(env, Services::default(), tool, prev)
}

pub fn reapply_with(
    env: &Env,
    services: Services<'_>,
    tool: Tool,
    prev: Option<&Provider>,
) -> Result<OperationReport, AppError> {
    settle(env, tool)?;
    let data = store::load(env)?;
    let target = data
        .current(tool)
        .ok_or_else(|| AppError::NotFound(format!("{tool} 没有当前供应商")))?;
    write(env, services, &data, tool, prev, target, op::APPLY, None)
}

/// 编辑一张卡。是当前卡时同时把新内容写进客户端文件；写入被拒绝（行有问题）时库里
/// 也不改。
pub fn update_provider(
    env: &Env,
    tool: Tool,
    provider: Provider,
) -> Result<Option<OperationReport>, AppError> {
    update_provider_with(env, Services::default(), tool, provider)
}

pub fn update_provider_with(
    env: &Env,
    services: Services<'_>,
    tool: Tool,
    provider: Provider,
) -> Result<Option<OperationReport>, AppError> {
    if tool.is_coexist() {
        return crate::coexist::update_provider(env, tool, provider);
    }
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
    match reapply_with(env, services, tool, Some(&old)) {
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
        Tool::Claude => claude::validate(provider),
        Tool::Codex => codex::plan(data, None, provider).map(|_| ()),
        Tool::Gemini => gemini::projection(provider).map(|_| ()),
        Tool::GrokBuild => grok::projection(provider).map(|_| ()),
        Tool::ClaudeDesktop => claude_desktop::validate(provider),
        Tool::OpenCode | Tool::OpenClaw | Tool::Hermes | Tool::Pi | Tool::Mcode => {
            crate::coexist::validate(tool, provider)
        }
    }
}

/// Non-blocking notices for fields outside the client's connection projection.
/// Report field names only; configuration values may contain credentials.
pub fn provider_warnings(tool: Tool, provider: &Provider) -> Vec<crate::ccswitch_export::Reason> {
    if tool != Tool::Claude {
        return Vec::new();
    }
    let ignored = crate::live::project::claude::ignored_keys(&provider.settings_config);
    if ignored.is_empty() {
        return Vec::new();
    }
    let keys = ignored.join(", ");
    vec![crate::ccswitch_export::Reason::new(
        "claude.ignored_fields",
        format!("这些字段不会写入 Claude，客户端原值保持不变：{keys}"),
        format!(
            "These fields are not written to Claude; existing client values are preserved: {keys}"
        ),
    )]
}

#[allow(clippy::too_many_arguments)]
fn write(
    env: &Env,
    services: Services<'_>,
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
            // 可能联网（刷新托管账号的 token），在拿写锁之前做。
            let prepared = codex::prepare(services.codex_oauth, prev, target)?;
            codex::run(env, data, op, planned, &prepared, pending)
        }
        Tool::Gemini => gemini::write(env, target, op, pending),
        Tool::GrokBuild => grok::write(env, prev, target, op, pending),
        Tool::ClaudeDesktop => claude_desktop::write(env, target, op, pending),
        Tool::OpenCode | Tool::OpenClaw | Tool::Hermes | Tool::Pi | Tool::Mcode => Err(
            AppError::InvalidInput(format!("{tool} 是并存式工具，没有当前供应商")),
        ),
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
    #[test]
    fn claude_switch_reads_the_outgoing_owner_after_the_tool_lock() {
        let root = tempfile::tempdir().unwrap();
        let env = Env::with_home(root.path());
        let a = Provider::new(
            "a",
            "A",
            json!({"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":"100"}}),
        );
        let b = Provider::new(
            "b",
            "B",
            json!({"env":{"CLAUDE_CODE_MAX_OUTPUT_TOKENS":"200"}}),
        );
        let c = Provider::new("c", "C", json!({"env":{"ANTHROPIC_AUTH_TOKEN":"fixture"}}));
        for provider in [&a, &b, &c] {
            store::add_provider(&env, Tool::Claude, provider.clone()).unwrap();
        }
        switch(&env, Tool::Claude, "a").unwrap();
        switch(&env, Tool::Claude, "b").unwrap();
        // Simulate a caller that read A before waiting for B to finish.
        claude::write(
            &env,
            Some(&a),
            &c,
            op::SWITCH,
            PendingTarget::pointer(Some("c".into())),
        )
        .unwrap();
        let live: Value =
            serde_json::from_slice(&fs::read(env.claude_settings_path()).unwrap()).unwrap();
        assert!(live["env"].get("CLAUDE_CODE_MAX_OUTPUT_TOKENS").is_none());
        assert!(live["env"].get("CLAUDE_CODE_MAX_CONTEXT_TOKENS").is_none());
        assert_eq!(live["env"]["ANTHROPIC_AUTH_TOKEN"], "fixture");
    }

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

    #[test]
    fn codex_rejects_a_stale_catalog_before_using_its_outgoing_projection() {
        let (_dir, env) = codex_sandbox();
        let data = store::load(&env).unwrap();
        let target = data.provider(Tool::Codex, "relay").unwrap();
        let planned = codex::plan(&data, data.current(Tool::Codex), target).unwrap();
        let prepared = codex::prepare(None, data.current(Tool::Codex), target).unwrap();
        let before = fs::read(env.codex_config_path()).unwrap();
        store::set_current(&env, Tool::Codex, "relay").unwrap();
        let error = codex::run(
            &env,
            &data,
            op::SWITCH,
            planned,
            &prepared,
            PendingTarget::pointer(Some("relay".into())),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AppError::Localized {
                key: "live.catalog_changed",
                ..
            }
        ));
        assert_eq!(fs::read(env.codex_config_path()).unwrap(), before);
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

    fn gemini_sandbox() -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        for (id, url, key) in [
            ("a", "https://a.example", "ka"),
            ("b", "https://b.example", "kb"),
        ] {
            store::add_provider(
                &env,
                Tool::Gemini,
                Provider::new(
                    id,
                    id,
                    json!({"env": {"GOOGLE_GEMINI_BASE_URL": url, "GEMINI_API_KEY": key},
                           "config": {"model": {"name": format!("m-{id}")}}}),
                ),
            )
            .unwrap();
        }
        switch(&env, Tool::Gemini, "a").unwrap();
        (dir, env)
    }

    /// Gemini 两个文件：`.env` 已经替换、`settings.json` 还没换时崩溃，启动时前滚，两个
    /// 文件和指针一起落到目标卡，不会停在「Key 是 B、模型是 A」的半截状态。
    #[test]
    fn a_gemini_crash_between_the_two_files_rolls_forward_both() {
        let (_dir, env) = gemini_sandbox();
        failpoint::crash_at(Some("published:0"));
        let result = switch(&env, Tool::Gemini, "b");
        failpoint::crash_at(None);
        assert!(result.is_err());
        let dotenv = fs::read_to_string(env.gemini_env_path()).unwrap();
        assert!(dotenv.contains("GEMINI_API_KEY=kb"), "{dotenv}");
        let settings = fs::read_to_string(env.gemini_settings_path()).unwrap();
        assert!(
            settings.contains("m-a"),
            "settings not published yet: {settings}"
        );
        assert_eq!(current(&env, Tool::Gemini).as_deref(), Some("a"));

        let outcomes = recover_on_startup(&env);
        assert!(
            matches!(outcomes[..], [(_, Ok(RecoveryOutcome::RolledForward))]),
            "{outcomes:?}"
        );
        let settings: Value =
            serde_json::from_str(&fs::read_to_string(env.gemini_settings_path()).unwrap()).unwrap();
        assert_eq!(settings["model"]["name"], "m-b");
        assert_eq!(current(&env, Tool::Gemini).as_deref(), Some("b"));
    }

    /// 写之前崩溃（只留下暂存文件）：启动时丢弃，live 和指针都还是 A。
    #[test]
    fn a_gemini_crash_before_publishing_is_discarded() {
        let (_dir, env) = gemini_sandbox();
        let before = fs::read(env.gemini_env_path()).unwrap();
        failpoint::crash_at(Some("pending"));
        assert!(switch(&env, Tool::Gemini, "b").is_err());
        failpoint::crash_at(None);
        let outcomes = recover_on_startup(&env);
        assert!(
            outcomes
                .iter()
                .all(|(_, outcome)| matches!(outcome, Ok(RecoveryOutcome::Discarded))),
            "{outcomes:?}"
        );
        assert_eq!(fs::read(env.gemini_env_path()).unwrap(), before);
        assert_eq!(current(&env, Tool::Gemini).as_deref(), Some("a"));
    }

    fn grok_row(name: &str) -> Value {
        json!({"config": format!(
            "[models]\ndefault = \"{name}\"\n\n[model.{name}]\nmodel = \"m\"\nbase_url = \"https://{name}.example/v1\"\nname = \"{name}\"\napi_key = \"k-{name}\"\napi_backend = \"responses\"\ncontext_window = 1000\n"
        )})
    }

    /// Grok：文件已替换、指针和写入记录还没落定时崩溃。下一次操作先前滚，写入记录跟着
    /// 落到 B 的表，再从 B 切到 C 时删的是 B 的表，A 的表早已删掉，不会留下两家的表。
    #[test]
    fn a_grok_crash_before_the_target_commits_keeps_the_written_record_consistent() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        for id in ["ra", "rb", "rc"] {
            store::add_provider(&env, Tool::GrokBuild, Provider::new(id, id, grok_row(id)))
                .unwrap();
        }
        fs::create_dir_all(&env.grok_dir).unwrap();
        fs::write(env.grok_config_path(), "[model.mine]\nmodel = \"x\"\n").unwrap();
        switch(&env, Tool::GrokBuild, "ra").unwrap();
        failpoint::crash_at(Some("published:0"));
        assert!(switch(&env, Tool::GrokBuild, "rb").is_err());
        failpoint::crash_at(None);
        assert_eq!(current(&env, Tool::GrokBuild).as_deref(), Some("ra"));

        switch(&env, Tool::GrokBuild, "rc").unwrap();
        let text = fs::read_to_string(env.grok_config_path()).unwrap();
        assert!(text.contains("[model.mine]"), "{text}");
        assert!(text.contains("[model.rc]"), "{text}");
        assert!(!text.contains("[model.ra]"), "{text}");
        assert!(!text.contains("[model.rb]"), "{text}");
        assert_eq!(current(&env, Tool::GrokBuild).as_deref(), Some("rc"));
        let written = crate::state::written(&env.device_store(), "grokbuild")
            .unwrap()
            .unwrap();
        assert_eq!(written.tables, vec!["rc".to_string()]);
    }

    // ==================== 托管账号的崩溃注入 ====================

    use crate::codex::oauth::testing::{FakeTransport, Identity};
    use crate::codex::oauth::CodexOAuthManager;
    use crate::provider::AuthBinding;
    use std::sync::Arc;

    struct Managed {
        _dir: tempfile::TempDir,
        env: Env,
        mgr: CodexOAuthManager,
        account: String,
        alice: Value,
    }

    fn managed_sandbox() -> Managed {
        let (dir, env) = codex_sandbox();
        let fake = Arc::new(FakeTransport::new());
        let mgr = CodexOAuthManager::new(&env, Some(fake.clone())).unwrap();
        let bob = Identity::new("bob", "ws-team", "team");
        let account = mgr
            .add_account_from_tokens(&fake.issue(&bob), None)
            .unwrap()
            .id;
        let mut card = Provider::new("bob", "Bob", json!({"auth": {}, "config": ""})).official();
        card.meta.auth_binding = Some(AuthBinding {
            auth_provider: "codex_oauth".into(),
            account_id: Some(account.clone()),
        });
        store::add_provider(&env, Tool::Codex, card).unwrap();
        let alice: Value =
            serde_json::from_slice(&fs::read(env.codex_auth_path()).unwrap()).unwrap();
        Managed {
            _dir: dir,
            env,
            mgr,
            account,
            alice,
        }
    }

    impl Managed {
        fn go(&self, id: &str) -> Result<OperationReport, AppError> {
            switch_with(
                &self.env,
                Services {
                    codex_oauth: Some(&self.mgr),
                },
                Tool::Codex,
                id,
            )
        }
        fn live(&self) -> Option<Value> {
            fs::read(self.env.codex_auth_path())
                .ok()
                .map(|b| serde_json::from_slice(&b).unwrap())
        }
        fn marker(&self) -> bool {
            crate::codex::oauth::live::marker_path(&self.env).exists()
        }
        fn stash(&self) -> Value {
            fs::read(self.env.data_file(crate::codex::login::STASH_FILENAME))
                .ok()
                .map(|b| serde_json::from_slice(&b).unwrap())
                .unwrap_or(Value::Null)
        }
        fn managed_live(&self) -> bool {
            self.live().is_some_and(|auth| {
                crate::codex::oauth::live::live_is_managed(&self.env, &auth, &self.account)
            })
        }
    }

    fn crash(point: &str, run: impl FnOnce() -> Result<OperationReport, AppError>) {
        failpoint::crash_at(Some(point));
        let result = run();
        failpoint::crash_at(None);
        assert!(result.is_err(), "crash at {point} did not abort");
    }

    /// 原生 → 托管，每个发布点之后崩溃：启动时前滚，auth.json、标记、暂存、指针一起落定；
    /// alice 的原生登录进了暂存，不会因为崩溃丢掉，托管登录被标记认出来。
    #[test]
    fn a_crash_while_switching_to_a_managed_account_rolls_forward_completely() {
        for point in ["published:0", "published:1", "published:2"] {
            let m = managed_sandbox();
            crash(point, || m.go("bob"));
            assert_eq!(current(&m.env, Tool::Codex).as_deref(), Some("official"));
            let outcomes = recover_on_startup(&m.env);
            assert!(
                matches!(outcomes[..], [(_, Ok(RecoveryOutcome::RolledForward))]),
                "{point}: {outcomes:?}"
            );
            assert_eq!(
                current(&m.env, Tool::Codex).as_deref(),
                Some("bob"),
                "{point}"
            );
            assert!(m.marker(), "{point}: marker written");
            assert!(m.managed_live(), "{point}: live is recognised as managed");
            assert_eq!(
                m.stash()["logins"]["sub:alice|account:acc-plus"],
                m.alice,
                "{point}: alice is stashed"
            );
            // 落定后照常切走：alice 回来，标记删掉。
            m.go("official").unwrap();
            assert_eq!(m.live(), Some(m.alice.clone()), "{point}");
            assert!(!m.marker(), "{point}");
        }
    }

    /// 托管 → 第三方，删掉 auth.json 之后、删标记之前崩溃：启动时前滚，标记也删掉，
    /// 不会留下「标记指向一个不存在的登录」。
    #[test]
    fn a_crash_after_deleting_the_managed_login_rolls_forward_and_drops_the_marker() {
        let m = managed_sandbox();
        m.go("bob").unwrap();
        crash("published:0", || m.go("relay"));
        assert!(m.live().is_none());
        assert!(m.marker(), "marker not deleted yet");
        let outcomes = recover_on_startup(&m.env);
        assert!(
            matches!(outcomes[..], [(_, Ok(RecoveryOutcome::RolledForward))]),
            "{outcomes:?}"
        );
        assert!(!m.marker());
        assert_eq!(current(&m.env, Tool::Codex).as_deref(), Some("relay"));
        assert!(!m.stash().to_string().contains("rt-bob"));
        m.go("bob").unwrap();
        assert!(m.managed_live());
    }

    /// 托管 → 官方，发布之前崩溃：丢弃，托管登录和标记都还在，再切一次照常。
    #[test]
    fn a_crash_before_publishing_a_managed_switch_away_is_discarded() {
        let m = managed_sandbox();
        m.go("bob").unwrap();
        let before = m.live();
        crash("pending", || m.go("official"));
        let outcomes = recover_on_startup(&m.env);
        assert!(
            outcomes
                .iter()
                .all(|(_, o)| matches!(o, Ok(RecoveryOutcome::Discarded))),
            "{outcomes:?}"
        );
        assert_eq!(m.live(), before);
        assert!(m.marker() && m.managed_live());
        assert_eq!(current(&m.env, Tool::Codex).as_deref(), Some("bob"));
        m.go("official").unwrap();
        assert_eq!(m.live(), Some(m.alice.clone()));
    }

    /// 崩溃留下的操作没补完时，管理器刷新后不去同步 live（会和恢复打架）。恢复后 live 是
    /// 可证明更旧的一代，再写一次当前卡就换成新一代。
    #[test]
    fn a_refresh_does_not_touch_live_while_a_codex_operation_is_pending() {
        let m = managed_sandbox();
        m.go("bob").unwrap();
        let stale = m.live();
        crash("pending", || m.go("relay"));
        m.mgr.test_forget_access_tokens();
        let bundle = m.mgr.token_bundle(&m.account).unwrap();
        assert_eq!(
            m.live(),
            stale,
            "live untouched while the operation is pending"
        );
        recover_on_startup(&m.env);
        assert_eq!(current(&m.env, Tool::Codex).as_deref(), Some("bob"));
        assert_eq!(m.live(), stale);
        m.go("bob").unwrap();
        assert_eq!(
            m.live().unwrap()["tokens"]["refresh_token"],
            json!(bundle.refresh_token)
        );
        assert!(m.managed_live());
    }
}
