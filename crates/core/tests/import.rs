//! 首次运行导入：只写供应商库，不碰客户端文件；ChatGPT OAuth 登录绝不进库。

mod common;

use common::Sandbox;
use leanflip_core::import::import_live;
use leanflip_core::{store, Tool};
use serde_json::json;

#[test]
fn claude_live_with_a_relay_becomes_a_card_plus_official() {
    let sb = Sandbox::new();
    let path = sb.env.claude_dir.join("settings.json");
    let live = r#"{"env":{"ANTHROPIC_BASE_URL":"https://r.example","ANTHROPIC_AUTH_TOKEN":"sk-r","MY_VAR":"x"},"permissions":{"allow":[]}}"#;
    sb.write(&path, live);
    let report = import_live(&sb.env, Tool::Claude).unwrap();
    assert_eq!(report.created, vec!["default", "official"]);
    assert_eq!(report.current.as_deref(), Some("default"));
    let data = store::load(&sb.env).unwrap();
    let row = data.provider(Tool::Claude, "default").unwrap();
    assert_eq!(
        row.settings_config,
        json!({"env": {"ANTHROPIC_BASE_URL": "https://r.example", "ANTHROPIC_AUTH_TOKEN": "sk-r"}}),
        "only key fields are imported"
    );
    assert!(data
        .provider(Tool::Claude, "official")
        .unwrap()
        .is_official_category());
    assert_eq!(sb.read(&path), live, "live untouched");
    // Repeating the same capture is idempotent.
    assert_eq!(
        import_live(&sb.env, Tool::Claude).unwrap().created,
        Vec::<String>::new()
    );
    assert_eq!(
        store::load(&sb.env).unwrap().providers(Tool::Claude).len(),
        2
    );
}

#[test]
fn claude_without_live_or_key_fields_is_official() {
    let sb = Sandbox::new();
    assert_eq!(
        import_live(&sb.env, Tool::Claude)
            .unwrap()
            .current
            .as_deref(),
        Some("official")
    );
    let sb = Sandbox::new();
    sb.write(
        &sb.env.claude_dir.join("settings.json"),
        r#"{"permissions":{}}"#,
    );
    let report = import_live(&sb.env, Tool::Claude).unwrap();
    assert_eq!(report.created, vec!["official"]);
}

#[test]
fn codex_oauth_login_never_enters_the_store() {
    let sb = Sandbox::new();
    sb.write(&sb.env.codex_config_path(), "model = \"gpt-5.5\"\n");
    sb.write(
        &sb.env.codex_auth_path(),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"secret-access","refresh_token":"secret-refresh"}}"#,
    );
    let report = import_live(&sb.env, Tool::Codex).unwrap();
    assert_eq!(report.created, vec!["official"]);
    let text = sb.read(&sb.env.store_path());
    assert!(!text.contains("secret"), "{text}");
}

#[test]
fn codex_relay_with_legacy_key_in_auth_json_is_imported() {
    let sb = Sandbox::new();
    sb.write(
        &sb.env.codex_config_path(),
        "model = \"m\"\nopenai_base_url = \"https://r.example/v1\"\n",
    );
    sb.write(
        &sb.env.codex_auth_path(),
        r#"{"OPENAI_API_KEY":"sk-legacy"}"#,
    );
    let report = import_live(&sb.env, Tool::Codex).unwrap();
    assert_eq!(report.created, vec!["default", "official"]);
    let data = store::load(&sb.env).unwrap();
    let row = data.provider(Tool::Codex, "default").unwrap();
    assert_eq!(row.settings_config["auth"]["OPENAI_API_KEY"], "sk-legacy");

    // 有登录的 auth.json 绝不当成第三方 Key：同样的配置，导入被拒绝。
    let sb = Sandbox::new();
    sb.write(
        &sb.env.codex_config_path(),
        "model = \"m\"\nopenai_base_url = \"https://r.example/v1\"\n",
    );
    sb.write(
        &sb.env.codex_auth_path(),
        r#"{"OPENAI_API_KEY":"sk-x","tokens":{"refresh_token":"r"}}"#,
    );
    assert!(import_live(&sb.env, Tool::Codex).is_err());
    assert!(store::load(&sb.env)
        .unwrap()
        .providers(Tool::Codex)
        .is_empty());
}

#[test]
fn repeated_codex_capture_ignores_common_preferences_and_unused_tables() {
    let sb = Sandbox::new();
    let config = "model='test'\nmodel_provider='relay'\n[model_providers.relay]\nname='relay'\nbase_url='https://relay.example/v1'\nexperimental_bearer_token='fake'\n";
    sb.write(&sb.env.codex_config_path(), config);
    assert_eq!(import_live(&sb.env, Tool::Codex).unwrap().created.len(), 2);
    let changed = format!("{config}\n[features]\nshell_snapshot=true\n[model_providers.unused]\nbase_url='https://unused.example'\n");
    sb.write(&sb.env.codex_config_path(), &changed);
    let before = sb.read(&sb.env.store_path());
    let report = import_live(&sb.env, Tool::Codex).unwrap();
    assert!(report.created.is_empty());
    assert_eq!(sb.read(&sb.env.store_path()), before);
    sb.write(
        &sb.env.codex_config_path(),
        &changed.replace("model='test'", "model='different'"),
    );
    assert_eq!(
        import_live(&sb.env, Tool::Codex).unwrap().created,
        vec!["default-2"]
    );
}
