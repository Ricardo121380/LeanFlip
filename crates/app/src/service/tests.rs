use std::path::Path;

use ccsl_core::codex::oauth::testing::{FakeTransport, Identity};
use serde_json::{json, Value};

use super::*;

struct Sandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        Self { _dir: dir, home }
    }

    fn env(&self) -> Env {
        let env = Env::with_home(&self.home);
        for tool in Tool::ALL {
            for path in env.live_paths(tool) {
                assert!(path.starts_with(&self.home), "{}", path.display());
            }
        }
        env
    }

    fn service(&self) -> Service {
        Service::start(self.env(), None).unwrap()
    }

    fn write(&self, path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

fn claude_card(id: &str, url: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({"env": {"ANTHROPIC_BASE_URL": url, "ANTHROPIC_AUTH_TOKEN": format!("sk-{id}")}}),
    )
}

#[test]
fn switch_tool_flow_through_the_service() {
    let sb = Sandbox::new();
    let env = sb.env();
    sb.write(
        &env.claude_settings_path(),
        r#"{"env":{"ANTHROPIC_BASE_URL":"https://old"},"theme":"dark"}"#,
    );
    let service = sb.service();

    let created = service.import_live(Tool::Claude).unwrap().created;
    assert!(!created.is_empty());
    service
        .add_provider(Tool::Claude, claude_card("relay", "https://relay"), false)
        .unwrap();
    let report = service.switch_provider(Tool::Claude, "relay").unwrap();
    assert!(
        report.changed.contains(&env.claude_settings_path()),
        "{:?}",
        report.changed
    );

    let live: Value =
        serde_json::from_slice(&std::fs::read(env.claude_settings_path()).unwrap()).unwrap();
    assert_eq!(live["env"]["ANTHROPIC_BASE_URL"], "https://relay");
    assert_eq!(live["theme"], "dark");

    let overview = service.overview().unwrap();
    let claude = overview.iter().find(|t| t.tool == Tool::Claude).unwrap();
    assert!(claude.detected);
    assert_eq!(claude.card_count, created.len() + 1);
    assert_eq!(claude.current.as_deref(), Some("relay"));
    assert_eq!(claude.kind, ToolKind::Switch);

    // 当前卡不能删；别的卡可以。
    assert!(service.delete_provider(Tool::Claude, "relay").is_err());
    for id in &created {
        service.delete_provider(Tool::Claude, id).unwrap();
    }
    assert_eq!(
        service
            .list_providers(Tool::Claude)
            .unwrap()
            .providers
            .len(),
        1
    );

    // 新卡带 addToLive：加完直接切过去。
    service
        .add_provider(Tool::Claude, claude_card("other", "https://other"), true)
        .unwrap();
    assert_eq!(
        service
            .list_providers(Tool::Claude)
            .unwrap()
            .current
            .as_deref(),
        Some("other")
    );
}

#[test]
fn codex_login_tokens_never_reach_the_frontend_and_survive_edits() {
    let sb = Sandbox::new();
    let env = sb.env();
    let card = Provider::new(
        "official",
        "OpenAI",
        json!({"auth": {"auth_mode": "chatgpt", "OPENAI_API_KEY": null,
                         "tokens": {"refresh_token": "rt-top-secret", "id_token": "eyJ.secret"}},
               "config": "model = \"gpt-5\"\n"}),
    )
    .official();
    store::add_provider(&env, Tool::Codex, card.clone()).unwrap();
    let service = sb.service();

    let listing = service.list_providers(Tool::Codex).unwrap();
    let wire = serde_json::to_string(&listing).unwrap();
    assert!(
        !wire.contains("rt-top-secret") && !wire.contains("eyJ.secret"),
        "{wire}"
    );

    let overview = serde_json::to_string(&service.overview().unwrap()).unwrap();
    assert!(!overview.contains("rt-top-secret"));

    let mut edited = listing.providers[0].clone();
    edited.settings_config["config"] = json!("model = \"gpt-5.5\"\n");
    service
        .update_provider(Tool::Codex, edited.clone())
        .unwrap();
    let stored = store::load(&env).unwrap();
    let saved = stored.provider(Tool::Codex, "official").unwrap();
    assert_eq!(saved.settings_config["auth"], card.settings_config["auth"]);
    assert_eq!(saved.settings_config["config"], "model = \"gpt-5.5\"\n");

    // 占位符不能借新卡写进文件。
    edited.id = "copy".into();
    let err = service
        .add_provider(Tool::Codex, edited, false)
        .unwrap_err();
    assert!(err.0.contains("隐藏"), "{err}");
    assert!(store::load(&env)
        .unwrap()
        .provider(Tool::Codex, "copy")
        .is_none());
}

#[test]
fn coexist_toggle_writes_and_removes_entries() {
    let sb = Sandbox::new();
    let env = sb.env();
    sb.write(
        &env.opencode_dir.join("opencode.json"),
        "{\n  \"provider\": {}\n}\n",
    );
    let service = sb.service();
    let card = Provider::new(
        "relay",
        "Relay",
        json!({"npm": "@ai-sdk/openai-compatible", "name": "Relay",
               "options": {"baseURL": "https://r", "apiKey": "sk"}, "models": {"m": {}}}),
    );
    service.add_provider(Tool::OpenCode, card, false).unwrap();
    assert!(service
        .list_providers(Tool::OpenCode)
        .unwrap()
        .in_live
        .is_empty());

    service.set_in_live(Tool::OpenCode, "relay", true).unwrap();
    let listing = service.list_providers(Tool::OpenCode).unwrap();
    assert_eq!(listing.in_live, ["relay"]);
    assert_eq!(listing.kind, ToolKind::Coexist);
    assert_eq!(listing.current, None);

    service.set_in_live(Tool::OpenCode, "relay", false).unwrap();
    assert!(service
        .list_providers(Tool::OpenCode)
        .unwrap()
        .in_live
        .is_empty());
    service.delete_provider(Tool::OpenCode, "relay").unwrap();
}

#[test]
fn broken_coexist_config_is_reported_not_fatal() {
    let sb = Sandbox::new();
    let env = sb.env();
    sb.write(&env.opencode_dir.join("opencode.json"), "{ not json");
    let service = sb.service();
    let listing = service.list_providers(Tool::OpenCode).unwrap();
    assert!(listing.live_error.is_some());
    assert!(service.overview().is_ok());
}

#[test]
fn settings_round_trip_and_dir_overrides_apply_immediately() {
    let sb = Sandbox::new();
    let env = sb.env();
    let service = sb.service();
    let view = service.settings().unwrap();
    assert!(!view.keep_codex_login);
    assert_eq!(view.app, AppSettings::default());
    assert_eq!(view.ccswitch_dir, sb.home.join(".cc-switch"));
    assert_eq!(view.dirs["claudeDir"], sb.home.join(".claude"));
    assert_eq!(view.dirs.len(), 10, "one per DirOverrides field");
    let fields = serde_json::to_value(ccsl_core::env::DirOverrides::default()).unwrap();
    for key in fields.as_object().unwrap().keys() {
        assert!(view.dirs.contains_key(key), "{key}");
    }

    let mut app = view.app.clone();
    app.language = Some("fr".into());
    let result = service
        .save_settings(SettingsInput {
            app: app.clone(),
            keep_codex_login: true,
        })
        .unwrap();
    assert!(!result.dirs_changed);
    assert!(
        store::load(&env)
            .unwrap()
            .settings
            .preserve_codex_official_auth_on_switch
    );
    assert_eq!(
        service.app_settings().language,
        None,
        "unknown language dropped"
    );

    app.language = Some("en".into());
    app.dir_overrides.claude_dir = Some(sb.home.join("alt-claude"));
    let result = service
        .save_settings(SettingsInput {
            app,
            keep_codex_login: false,
        })
        .unwrap();
    assert!(result.dirs_changed);
    // 不用重启：立刻生效。
    assert_eq!(
        service.env().claude_settings_path(),
        sb.home.join("alt-claude").join("settings.json")
    );
    assert_eq!(
        service.settings().unwrap().dirs["claudeDir"],
        sb.home.join("alt-claude")
    );

    // 重新启动后也一样。
    let restarted = sb.service();
    assert_eq!(
        restarted.env().claude_settings_path(),
        sb.home.join("alt-claude").join("settings.json")
    );
    assert_eq!(restarted.app_settings().language.as_deref(), Some("en"));
    assert!(!restarted.settings().unwrap().keep_codex_login);
}

#[test]
fn errors_follow_the_ui_language() {
    let error = AppError::localized("k", "中文", "English");
    assert_eq!(
        ApiError::from_core(error.clone_for_test(), true).0,
        "English"
    );
    assert_eq!(ApiError::from_core(error, false).0, "中文");
}

trait CloneForTest {
    fn clone_for_test(&self) -> AppError;
}

impl CloneForTest for AppError {
    fn clone_for_test(&self) -> AppError {
        match self {
            AppError::Localized { key, zh, en } => AppError::localized(key, zh.clone(), en.clone()),
            other => AppError::Message(other.to_string()),
        }
    }
}

#[test]
fn managed_accounts_and_background_refresh_go_through_the_manager() {
    let sb = Sandbox::new();
    let fake = Arc::new(FakeTransport::default());
    let service = Service::start(sb.env(), Some(fake.clone())).unwrap();

    let id = Identity::new("user-1", "ws-1", "plus");
    let flow = service.codex_start_login(None).unwrap();
    fake.approve(&flow.device_code, &id);
    let account = match service.codex_poll_login(&flow.device_code).unwrap() {
        PollResult::Added { account } => account,
        PollResult::Pending => panic!("fake authorizes immediately"),
    };
    assert_eq!(account.plan.as_deref(), Some("plus"));
    let wire = serde_json::to_string(&service.codex_accounts()).unwrap();
    assert!(!wire.to_lowercase().contains("refresh_token"), "{wire}");

    // 刚登录的不刷新。
    let before = fake.refresh_calls().len();
    service.refresh_stale_accounts();
    assert_eq!(fake.refresh_calls().len(), before);

    service.codex_remove_account(&account.id).unwrap();
    assert!(service.codex_accounts().is_empty());
}

#[test]
fn dir_overrides_apply_live_and_keep_the_account_manager_when_codex_dir_is_unchanged() {
    let sb = Sandbox::new();
    let env = sb.env();
    let alt = sb.home.join("alt-claude");
    sb.write(
        &alt.join("settings.json"),
        r#"{"env":{"ANTHROPIC_BASE_URL":"https://alt"}}"#,
    );
    let fake = Arc::new(FakeTransport::default());
    let service = Service::start(env.clone(), Some(fake.clone())).unwrap();
    service
        .add_provider(Tool::Claude, claude_card("relay", "https://relay"), false)
        .unwrap();

    // 设备码登录进行中时只改 Claude 目录：登录照样能完成。
    let flow = service.codex_start_login(None).unwrap();
    let mut app = service.app_settings();
    app.dir_overrides.claude_dir = Some(alt.clone());
    assert!(
        service
            .save_settings(SettingsInput {
                app: app.clone(),
                keep_codex_login: false
            })
            .unwrap()
            .dirs_changed
    );
    fake.approve(&flow.device_code, &Identity::new("u", "ws", "plus"));
    assert!(matches!(
        service.codex_poll_login(&flow.device_code).unwrap(),
        PollResult::Added { .. }
    ));

    // 切换写到新目录，默认目录不动。
    service.switch_provider(Tool::Claude, "relay").unwrap();
    let live: Value =
        serde_json::from_slice(&std::fs::read(alt.join("settings.json")).unwrap()).unwrap();
    assert_eq!(live["env"]["ANTHROPIC_BASE_URL"], "https://relay");
    assert!(!env.claude_settings_path().exists());

    // 再存一次同样的设置：没有变化。
    assert!(
        !service
            .save_settings(SettingsInput {
                app,
                keep_codex_login: false
            })
            .unwrap()
            .dirs_changed
    );
}
