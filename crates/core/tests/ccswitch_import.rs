//! 从 cc-switch 只读导入：SQLite 和旧版 config.json、跳过规则、Codex 登录剥离、账号。

mod common;

use common::Sandbox;
use leanflip_core::ccswitch_import::{self, ACCOUNTS_FILENAME, DB_FILENAME};
use leanflip_core::codex::oauth::testing::{FakeTransport, Identity};
use leanflip_core::codex::oauth::CodexOAuthManager;
use leanflip_core::{store, Provider, Tool};
use serde_json::json;
use std::sync::Arc;

fn make_db(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE providers (id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL,
          settings_config TEXT NOT NULL, website_url TEXT, category TEXT, created_at INTEGER,
          sort_index INTEGER, notes TEXT, icon TEXT, icon_color TEXT, meta TEXT NOT NULL DEFAULT '{}',
          is_current BOOLEAN NOT NULL DEFAULT 0, in_failover_queue BOOLEAN NOT NULL DEFAULT 0,
          PRIMARY KEY (id, app_type));",
    )
    .unwrap();
    let rows = [
        (
            "claude",
            "relay",
            "Relay",
            json!({"env": {"ANTHROPIC_BASE_URL": "https://r", "ANTHROPIC_AUTH_TOKEN": "k"}}),
            "{}",
        ),
        (
            "claude",
            "openai-ish",
            "Needs proxy",
            json!({"env": {"ANTHROPIC_BASE_URL": "https://o"}}),
            r#"{"apiFormat":"openai_chat"}"#,
        ),
        (
            "codex",
            "official",
            "OpenAI",
            json!({"auth": {"auth_mode": "chatgpt", "tokens": {"refresh_token": "rt-secret"}}, "config": ""}),
            "{}",
        ),
        (
            "codex",
            "copilot",
            "Copilot",
            json!({"auth": {}, "config": ""}),
            r#"{"providerType":"github_copilot"}"#,
        ),
        ("hermes", "h", "H", json!({"base_url": "https://h"}), "{}"),
        ("unknown-app", "x", "X", json!({}), "{}"),
    ];
    for (app, id, name, settings, meta) in rows {
        let category = if id == "official" {
            Some("official")
        } else {
            None
        };
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config, category, meta, website_url, is_current) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'https://site', 1)",
            rusqlite::params![id, app, name, settings.to_string(), category, meta],
        )
        .unwrap();
    }
}

#[test]
fn scan_does_not_create_a_lock_in_an_existing_data_directory() {
    let sb = Sandbox::new();
    std::fs::create_dir_all(&sb.env.data_dir).unwrap();
    let scan = ccswitch_import::scan(&sb.env, &sb.env.home.join("missing-source")).unwrap();
    assert!(scan.candidates.is_empty());
    assert_eq!(std::fs::read_dir(&sb.env.data_dir).unwrap().count(), 0);
}

#[test]
fn imports_from_the_database_read_only_with_skip_rules() {
    let sb = Sandbox::new();
    let dir = ccswitch_import::default_dir(&sb.env);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join(DB_FILENAME);
    make_db(&db);
    let before = std::fs::read(&db).unwrap();
    sb.add(
        Tool::Hermes,
        Provider::new("h", "existing", json!({"base_url": "https://mine"})),
    );

    let scan = ccswitch_import::scan(&sb.env, &dir).unwrap();
    assert_eq!(scan.candidates.len(), 5, "unknown apps dropped");
    let reason = |id: &str| {
        scan.candidates
            .iter()
            .find(|c| c.provider.id == id)
            .unwrap()
            .skip_reason
            .clone()
    };
    let openai = reason("openai-ish").unwrap();
    assert_eq!(openai.code, "routing");
    assert!(openai.zh.contains("openai_chat") && openai.zh.contains("本地路由"));
    assert!(openai.en.contains("openai_chat") && openai.en.contains("local routing"));
    let copilot = reason("copilot").unwrap();
    assert!(copilot.zh.contains("github_copilot") && copilot.en.contains("github_copilot"));
    let dup = reason("h").unwrap();
    assert_eq!(dup.code, "duplicate");
    assert!(dup.zh.contains("已有"));
    assert!(dup.en.contains("already exists"));
    for c in &scan.candidates {
        if let Some(r) = &c.skip_reason {
            assert!(r.en.is_ascii(), "English reason has no Chinese: {}", r.en);
        }
    }
    assert_eq!(reason("relay"), None);

    let report = ccswitch_import::apply(&sb.env, &dir, &Tool::ALL, None).unwrap();
    assert_eq!(
        report.created,
        [
            (Tool::Claude, "relay".to_string()),
            (Tool::Codex, "official".to_string())
        ]
    );
    assert_eq!(report.skipped.len(), 3);
    let data = store::load(&sb.env).unwrap();
    let official = data.provider(Tool::Codex, "official").unwrap();
    assert!(
        !serde_json::to_string(official)
            .unwrap()
            .contains("rt-secret"),
        "login stripped"
    );
    assert!(official.is_official_category());
    assert_eq!(
        data.provider(Tool::Claude, "relay").unwrap().extra["websiteUrl"],
        "https://site"
    );
    assert_eq!(
        data.app(Tool::Claude).unwrap().current,
        None,
        "pointer not imported"
    );
    assert_eq!(data.provider(Tool::Hermes, "h").unwrap().name, "existing");
    assert_eq!(
        std::fs::read(&db).unwrap(),
        before,
        "cc-switch db untouched"
    );
    // 再导一次：全部跳过。
    let again = ccswitch_import::apply(&sb.env, &dir, &Tool::ALL, None).unwrap();
    assert!(again.created.is_empty());
}

#[test]
fn imports_the_legacy_config_json_and_managed_accounts() {
    let sb = Sandbox::new();
    let dir = ccswitch_import::default_dir(&sb.env);
    sb.write(
        &dir.join("config.json"),
        &json!({"claude": {"providers": {
            "a": {"name": "A", "settingsConfig": {"env": {}}},
            "cp": {"name": "Copilot", "providerType": "github_copilot", "settingsConfig": {"env": {}}}
        }, "current": "a"}})
            .to_string(),
    );
    // cc-switch 的账号库：用假的 OAuth 先造一个。
    let other = Sandbox::new();
    let fake = Arc::new(FakeTransport::new());
    let theirs = CodexOAuthManager::new(&other.env, Some(fake.clone())).unwrap();
    let account = theirs
        .add_account_from_tokens(&fake.issue(&Identity::new("u", "ws-team", "team")), None)
        .unwrap();
    std::fs::copy(theirs.store_path(), dir.join(ACCOUNTS_FILENAME)).unwrap();

    let ours = CodexOAuthManager::new(&sb.env, Some(fake)).unwrap();
    let scan = ccswitch_import::scan(&sb.env, &dir).unwrap();
    assert_eq!(scan.accounts, 1);
    let report =
        ccswitch_import::apply(&sb.env, &dir, &[Tool::Claude, Tool::Codex], Some(&ours)).unwrap();
    assert_eq!(report.created, [(Tool::Claude, "a".to_string())]);
    assert_eq!(
        report.skipped.len(),
        1,
        "top-level providerType is honoured too"
    );
    assert_eq!(report.accounts_added, vec![account.id.clone()]);
    let listed = ours.list_accounts();
    assert_eq!(listed[0].plan.as_deref(), Some("team"));
    assert!(listed[0].is_default);
    // 已有的账号不覆盖。
    assert!(ours
        .import_accounts(vec![ours.account(&account.id).unwrap()], None)
        .unwrap()
        .is_empty());
}

#[test]
fn detects_ccswitch_directory_preferences_within_the_supplied_home() {
    let sb = Sandbox::new();
    let root = if cfg!(target_os = "macos") {
        "Library/Application Support"
    } else if cfg!(windows) {
        "AppData/Roaming"
    } else {
        ".local/share"
    };
    let preference = sb
        .env
        .home
        .join(root)
        .join("com.ccswitch.desktop/app_paths.json");
    assert_eq!(
        ccswitch_import::detected_dir(&sb.env).unwrap(),
        sb.env.home.join(".cc-switch")
    );
    let custom = sb.env.home.join("custom-source");
    std::fs::create_dir_all(&custom).unwrap();
    sb.write(
        &preference,
        &json!({"app_config_dir_override":"~/custom-source"}).to_string(),
    );
    assert_eq!(ccswitch_import::detected_dir(&sb.env).unwrap(), custom);
    sb.write(
        &preference,
        &json!({"app_config_dir_override":sb.env.home.join("missing")}).to_string(),
    );
    assert_eq!(
        ccswitch_import::detected_dir(&sb.env).unwrap(),
        sb.env.home.join(".cc-switch")
    );
    sb.write(&preference, "{broken");
    assert!(ccswitch_import::detected_dir(&sb.env).is_err());
    sb.write(
        &preference,
        &json!({"app_config_dir_override":"relative"}).to_string(),
    );
    assert!(ccswitch_import::detected_dir(&sb.env).is_err());
}
