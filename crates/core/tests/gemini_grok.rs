//! Gemini CLI（`.env` + `settings.json`）和 Grok Build（`config.toml`）的切换金样测试，
//! 全部在临时 HOME 里。

mod common;

use common::Sandbox;
use leanflip_core::import::import_live;
use leanflip_core::switch;
use leanflip_core::{Provider, Tool};
use serde_json::{json, Value};

const GEMINI_ENV: &str = "# my gemini env\nGOOGLE_GEMINI_BASE_URL=https://a.example\nGEMINI_API_KEY=ka\nMY_PROXY=http://127.0.0.1:7890\n\n# trailing comment\n";

const GEMINI_SETTINGS: &str = r#"{
  "theme": "GitHub",
  "security": {"auth": {"selectedType": "gemini-api-key"}, "folderTrust": {"enabled": true}},
  "model": {"name": "m-a", "maxSessionTurns": 20},
  "mcpServers": {"fs": {"command": "npx", "args": ["fs"]}}
}"#;

fn gemini(id: &str, url: &str, key: &str, model: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({"env": {"GOOGLE_GEMINI_BASE_URL": url, "GEMINI_API_KEY": key, "GEMINI_MODEL": model},
               "config": {"model": {"name": model}}}),
    )
}

fn gemini_sandbox() -> Sandbox {
    let sb = Sandbox::new();
    assert!(sb.env.gemini_dir.starts_with(sb.home()));
    sb.write(&sb.env.gemini_env_path(), GEMINI_ENV);
    sb.write(&sb.env.gemini_settings_path(), GEMINI_SETTINGS);
    sb.add(Tool::Gemini, gemini("a", "https://a.example", "ka", "m-a"));
    sb.add(
        Tool::Gemini,
        gemini("b", "https://b.example/v1", "kb", "m-b"),
    );
    sb.add(
        Tool::Gemini,
        Provider::new("official", "Google", json!({"env": {}, "config": {}})).official(),
    );
    sb
}

fn user_settings_intact(settings: &Value) {
    assert_eq!(settings["theme"], "GitHub");
    assert_eq!(settings["security"]["folderTrust"]["enabled"], true);
    assert_eq!(settings["model"]["maxSessionTurns"], 20);
    assert_eq!(settings["mcpServers"]["fs"]["command"], "npx");
}

#[test]
fn gemini_round_trip_keeps_user_lines_and_settings() {
    let sb = gemini_sandbox();
    let env_path = sb.env.gemini_env_path();
    let settings_path = sb.env.gemini_settings_path();

    let report = switch::switch(&sb.env, Tool::Gemini, "b").unwrap();
    assert!(!report.changed.is_empty());
    let dotenv = sb.read(&env_path);
    assert!(dotenv.contains("# my gemini env"), "{dotenv}");
    assert!(
        dotenv.contains("MY_PROXY=http://127.0.0.1:7890"),
        "{dotenv}"
    );
    assert!(dotenv.contains("# trailing comment"), "{dotenv}");
    assert!(dotenv.contains("GEMINI_API_KEY=kb"), "{dotenv}");
    assert!(dotenv.contains("GOOGLE_GEMINI_BASE_URL=https://b.example/v1"));
    assert!(!dotenv.contains("ka"), "{dotenv}");
    let settings = sb.json(&settings_path);
    user_settings_intact(&settings);
    assert_eq!(settings["model"]["name"], "m-b");
    assert_eq!(
        settings["security"]["auth"]["selectedType"],
        "gemini-api-key"
    );
    assert_eq!(sb.current(Tool::Gemini).as_deref(), Some("b"));

    #[cfg(unix)]
    assert_eq!(common::mode(&env_path), 0o600);
    // 首写备份是原文件的字节拷贝。
    assert_eq!(sb.read(&sb.first_write_backup(&env_path)), GEMINI_ENV);
    assert_eq!(
        sb.read(&sb.first_write_backup(&settings_path)),
        GEMINI_SETTINGS
    );

    switch::switch(&sb.env, Tool::Gemini, "a").unwrap();
    let dotenv = sb.read(&env_path);
    assert!(
        dotenv.contains("GEMINI_API_KEY=ka") && !dotenv.contains("kb"),
        "{dotenv}"
    );
    assert!(dotenv.contains("MY_PROXY=http://127.0.0.1:7890"));
    let settings = sb.json(&settings_path);
    user_settings_intact(&settings);
    assert_eq!(settings["model"]["name"], "m-a");

    // 再切一次同一张卡：字节稳定。
    let env_bytes = sb.read(&env_path);
    let settings_bytes = sb.read(&settings_path);
    switch::switch(&sb.env, Tool::Gemini, "a").unwrap();
    assert_eq!(sb.read(&env_path), env_bytes);
    assert_eq!(sb.read(&settings_path), settings_bytes);
    assert!(sb.temp_files().is_empty(), "{:?}", sb.temp_files());
}

#[test]
fn gemini_official_clears_keys_and_selects_google_login() {
    let sb = gemini_sandbox();
    switch::switch(&sb.env, Tool::Gemini, "official").unwrap();
    let dotenv = sb.read(&sb.env.gemini_env_path());
    assert!(!dotenv.contains("GEMINI_API_KEY"), "{dotenv}");
    assert!(!dotenv.contains("GOOGLE_GEMINI_BASE_URL"), "{dotenv}");
    assert!(
        dotenv.contains("MY_PROXY=http://127.0.0.1:7890"),
        "{dotenv}"
    );
    let settings = sb.json(&sb.env.gemini_settings_path());
    user_settings_intact(&settings);
    assert_eq!(
        settings["security"]["auth"]["selectedType"],
        "oauth-personal"
    );
    assert!(settings["model"].get("name").is_none(), "{settings}");
}

#[test]
fn gemini_google_card_is_detected_by_name_or_partner_key() {
    let by_name = Provider::new("g", "Google Official", json!({"env": {}}));
    assert!(switch::gemini::is_official(&by_name));
    let mut by_key = Provider::new("g2", "My Gemini", json!({"env": {}}));
    by_key
        .meta
        .extra
        .insert("partnerPromotionKey".into(), json!("google-official"));
    assert!(switch::gemini::is_official(&by_key));
    assert!(!switch::gemini::is_official(&Provider::new(
        "x",
        "Googler relay",
        json!({})
    )));
}

#[test]
fn gemini_third_party_without_credentials_is_refused_and_nothing_is_written() {
    let sb = gemini_sandbox();
    sb.add(
        Tool::Gemini,
        Provider::new(
            "bad",
            "Bad",
            json!({"env": {"GOOGLE_GEMINI_BASE_URL": "https://x"}}),
        ),
    );
    let err = switch::switch(&sb.env, Tool::Gemini, "bad").unwrap_err();
    assert!(err.to_string().contains("GEMINI_API_KEY"), "{err}");
    assert_eq!(sb.read(&sb.env.gemini_env_path()), GEMINI_ENV);
    assert_eq!(sb.read(&sb.env.gemini_settings_path()), GEMINI_SETTINGS);
    assert!(sb.current(Tool::Gemini).is_none());
}

#[test]
fn gemini_import_takes_only_key_fields() {
    let sb = Sandbox::new();
    sb.write(&sb.env.gemini_env_path(), GEMINI_ENV);
    sb.write(&sb.env.gemini_settings_path(), GEMINI_SETTINGS);
    let report = import_live(&sb.env, Tool::Gemini).unwrap();
    assert_eq!(report.created, vec!["default", "official"]);
    assert_eq!(report.current.as_deref(), Some("default"));
    let data = leanflip_core::store::load(&sb.env).unwrap();
    let card = &data.providers(Tool::Gemini)[0];
    assert_eq!(
        card.settings_config,
        json!({"env": {"GOOGLE_GEMINI_BASE_URL": "https://a.example", "GEMINI_API_KEY": "ka"},
               "config": {"model": {"name": "m-a"}}})
    );
    // 导入不写 live。
    assert_eq!(sb.read(&sb.env.gemini_env_path()), GEMINI_ENV);

    let empty = Sandbox::new();
    let report = import_live(&empty.env, Tool::Gemini).unwrap();
    assert_eq!(report.created, vec!["official"]);
    assert!(!empty.env.gemini_dir.exists());
}

const GROK_LIVE: &str = r#"# grok config
[models]
default = "ra"
web_search = "grok-4"

[model.ra]
model = "ma"
base_url = "https://ra.example/v1"
name = "RA"
api_key = "k-ra"
api_backend = "responses"
context_window = 128000
reasoning_summary = "auto"

[model.mine]
model = "local"
base_url = "http://localhost:8080/v1"
name = "Mine"
env_key = "MINE_KEY"
api_backend = "chat"
context_window = 8000

[mcp_servers.fs]
command = "npx"
"#;

fn grok(name: &str, extra: &str) -> Provider {
    Provider::new(
        name,
        name,
        json!({"config": format!(
            "[models]\ndefault = \"{name}\"\n\n[model.{name}]\nmodel = \"m-{name}\"\nbase_url = \"https://{name}.example/v1\"\nname = \"{name}\"\napi_key = \"k-{name}\"\napi_backend = \"responses\"\ncontext_window = 1000\n{extra}"
        )}),
    )
}

fn grok_sandbox() -> Sandbox {
    let sb = Sandbox::new();
    assert!(sb.env.grok_dir.starts_with(sb.home()));
    sb.write(&sb.env.grok_config_path(), GROK_LIVE);
    import_live(&sb.env, Tool::GrokBuild).unwrap();
    sb.add(Tool::GrokBuild, grok("rb", "env_key = \"RB_KEY\"\n"));
    sb
}

fn grok_doc(sb: &Sandbox) -> toml::Table {
    sb.read(&sb.env.grok_config_path()).parse().unwrap()
}

#[test]
fn grok_import_takes_the_default_table_only() {
    let sb = grok_sandbox();
    let data = leanflip_core::store::load(&sb.env).unwrap();
    let cards = data.providers(Tool::GrokBuild);
    assert_eq!(cards[0].id, "default");
    assert_eq!(sb.current(Tool::GrokBuild).as_deref(), Some("default"));
    let config = cards[0].settings_config["config"].as_str().unwrap();
    assert!(config.contains("[model.ra]") && config.contains("reasoning_summary"));
    assert!(
        !config.contains("mine") && !config.contains("mcp_servers"),
        "{config}"
    );
    assert!(cards[1].is_official_category());

    let empty = Sandbox::new();
    empty.write(
        &empty.env.grok_config_path(),
        "[mcp_servers.x]\ncommand = \"y\"\n",
    );
    let report = import_live(&empty.env, Tool::GrokBuild).unwrap();
    assert_eq!(report.created, vec!["official"]);
}

#[test]
fn grok_round_trip_replaces_the_whole_table_and_keeps_user_tables() {
    let sb = grok_sandbox();
    let path = sb.env.grok_config_path();
    switch::switch(&sb.env, Tool::GrokBuild, "rb").unwrap();
    let doc = grok_doc(&sb);
    assert_eq!(doc["models"]["default"].as_str(), Some("rb"));
    assert_eq!(doc["models"]["web_search"].as_str(), Some("grok-4"));
    assert!(doc["model"].get("ra").is_none(), "{doc}");
    assert_eq!(doc["model"]["rb"]["env_key"].as_str(), Some("RB_KEY"));
    assert_eq!(doc["model"]["mine"]["env_key"].as_str(), Some("MINE_KEY"));
    assert_eq!(doc["mcp_servers"]["fs"]["command"].as_str(), Some("npx"));
    assert!(sb.read(&path).starts_with("# grok config\n"));
    #[cfg(unix)]
    assert_eq!(common::mode(&path), 0o600);
    assert_eq!(sb.read(&sb.first_write_backup(&path)), GROK_LIVE);

    switch::switch(&sb.env, Tool::GrokBuild, "default").unwrap();
    let doc = grok_doc(&sb);
    assert_eq!(doc["models"]["default"].as_str(), Some("ra"));
    assert!(doc["model"].get("rb").is_none(), "{doc}");
    assert_eq!(
        doc["model"]["ra"]["reasoning_summary"].as_str(),
        Some("auto")
    );
    assert_eq!(doc["model"]["mine"]["name"].as_str(), Some("Mine"));
    assert!(sb.temp_files().is_empty());
}

/// Grok 的 `/settings` 把 `models.default` 改成了内置模型：切走时按写入记录删表，
/// 上一家的表（含 Key）不会留在 live。
#[test]
fn grok_switch_away_after_client_changed_default_still_removes_our_table() {
    let sb = grok_sandbox();
    switch::switch(&sb.env, Tool::GrokBuild, "rb").unwrap();
    let path = sb.env.grok_config_path();
    let edited = sb
        .read(&path)
        .replace("default = \"rb\"", "default = \"grok-4\"");
    sb.write(&path, &edited);
    switch::switch(&sb.env, Tool::GrokBuild, "default").unwrap();
    let text = sb.read(&path);
    assert!(!text.contains("k-rb"), "{text}");
    assert!(text.contains("[model.mine]"));
}

#[test]
fn grok_official_removes_our_table_and_default() {
    let sb = grok_sandbox();
    switch::switch(&sb.env, Tool::GrokBuild, "official").unwrap();
    let doc = grok_doc(&sb);
    assert!(doc["models"].get("default").is_none(), "{doc}");
    assert_eq!(doc["models"]["web_search"].as_str(), Some("grok-4"));
    assert!(doc["model"].get("ra").is_none());
    assert!(doc["model"].get("mine").is_some());

    // 没有用户表时，空的 [model] / [models] 一并去掉，Grok 认出官方状态。
    let clean = Sandbox::new();
    clean.add(Tool::GrokBuild, grok("rx", ""));
    clean.add(
        Tool::GrokBuild,
        Provider::new("official", "xAI", json!({"config": ""})).official(),
    );
    switch::switch(&clean.env, Tool::GrokBuild, "rx").unwrap();
    switch::switch(&clean.env, Tool::GrokBuild, "official").unwrap();
    let text = clean.read(&clean.env.grok_config_path());
    assert!(
        leanflip_core::grok_config::is_official_live_config(&text),
        "{text:?}"
    );
}

#[test]
fn grok_invalid_card_is_refused_and_nothing_is_written() {
    let sb = grok_sandbox();
    sb.add(
        Tool::GrokBuild,
        Provider::new(
            "bad",
            "Bad",
            json!({"config": "[models]\ndefault = \"bad\"\n\n[model.bad]\nmodel = \"m\"\n"}),
        ),
    );
    assert!(switch::switch(&sb.env, Tool::GrokBuild, "bad").is_err());
    assert_eq!(sb.read(&sb.env.grok_config_path()), GROK_LIVE);
    assert_eq!(sb.current(Tool::GrokBuild).as_deref(), Some("default"));
    // 编辑时也一样拦下：预检不过，库里的行不变。
    let data = leanflip_core::store::load(&sb.env).unwrap();
    let bad = data
        .providers(Tool::GrokBuild)
        .iter()
        .find(|p| p.id == "bad")
        .unwrap()
        .clone();
    assert!(switch::preflight(&data, Tool::GrokBuild, &bad).is_err());
}

#[test]
fn gemini_imported_official_login_restores_its_model() {
    let sb = Sandbox::new();
    sb.write(&sb.env.gemini_settings_path(), r#"{"model":{"name":"official-model","maxSessionTurns":20},"security":{"auth":{"selectedType":"oauth-personal"}}}"#);
    import_live(&sb.env, Tool::Gemini).unwrap();
    let official = sb.current(Tool::Gemini).unwrap();
    sb.add(
        Tool::Gemini,
        gemini(
            "relay",
            "https://example.invalid",
            "fake-key",
            "relay-model",
        ),
    );
    switch::switch(&sb.env, Tool::Gemini, "relay").unwrap();
    switch::switch(&sb.env, Tool::Gemini, &official).unwrap();
    let settings = sb.json(&sb.env.gemini_settings_path());
    assert_eq!(settings["model"]["name"], "official-model");
    assert_eq!(settings["model"]["maxSessionTurns"], 20);
    assert_eq!(
        settings["security"]["auth"]["selectedType"],
        "oauth-personal"
    );
}
