mod common;
use common::Sandbox;
use leanflip_core::provider_import::{self, Selection, Source};
use leanflip_core::{store, Provider, Tool};
use serde_json::json;

#[test]
fn ccswitch_selection_is_readonly_and_does_not_activate_or_overwrite() {
    let sb = Sandbox::new();
    let dir = sb.home().join(".cc-switch");
    let path = dir.join("config.json");
    let input = json!({"claude":{"providers": {
        "one": {"id":"one", "name":"One", "settingsConfig":{"env":{"ANTHROPIC_AUTH_TOKEN":"fixture-key"}}},
        "two": {"id":"two", "name":"Two", "settingsConfig":{"env":{}}}
    }}}).to_string();
    sb.write(&path, &input);
    let scan = provider_import::scan(&sb.env, Source::CcSwitch, None).unwrap();
    assert!(!sb.env.data_dir.exists());
    let preview = serde_json::to_string(&scan.preview(1)).unwrap();
    assert!(!preview.contains("fixture-key"));
    assert!(!preview.contains("settingsConfig"));
    let report = provider_import::apply(
        &sb.env,
        &scan,
        &[Selection {
            index: 0,
            api_key: None,
        }],
    )
    .unwrap();
    assert_eq!(report.created.len(), 1);
    let data = store::load(&sb.env).unwrap();
    assert_eq!(data.providers(Tool::Claude).len(), 1);
    assert!(data.current(Tool::Claude).is_none());
    assert!(!sb.env.claude_settings_path().exists());
    assert_eq!(sb.read(&path), input);
    let retry = provider_import::apply(
        &sb.env,
        &scan,
        &[Selection {
            index: 0,
            api_key: None,
        }],
    )
    .unwrap();
    assert!(retry.created.is_empty());
    assert_eq!(retry.skipped.len(), 1);
}

#[test]
fn aitoolbox_reads_jsonb_and_text_and_strips_login_tokens() {
    let sb = Sandbox::new();
    let dir = sb.home().join("toolbox");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ai-toolbox.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE codex_provider(id TEXT, data BLOB); CREATE TABLE claude_provider(id TEXT, data TEXT); CREATE TABLE gemini_cli_provider(id TEXT, data BLOB); CREATE TABLE claude_desktop_provider(id TEXT, data BLOB);").unwrap();
    let config = "model_provider = 'relay'\nmodel = 'test-model'\n[model_providers.relay]\nname = 'Relay'\nbase_url = 'https://relay.example/v1'\nwire_api = 'responses'";
    let row = json!({"name":"Codex", "category":"custom", "settings_config":json!({"config":config, "auth":{"OPENAI_API_KEY":"fixture-key", "tokens":{"refresh_token":"fixture-token"}}}).to_string()});
    conn.execute(
        "INSERT INTO codex_provider VALUES ('one', jsonb(?1))",
        [row.to_string()],
    )
    .unwrap();
    conn.execute("INSERT INTO claude_provider VALUES ('two', ?1)", [json!({"name":"Claude", "category":"custom", "settings_config":"{\"env\":{\"ANTHROPIC_AUTH_TOKEN\":\"fixture-key\"}}", "meta":{"apiFormat":"anthropic_messages"}}).to_string()]).unwrap();
    conn.execute("INSERT INTO gemini_cli_provider VALUES ('three', jsonb(?1))", [json!({"name":"Gemini", "category":"custom", "settings_config":"{\"env\":{\"GEMINI_API_KEY\":\"fixture-key\",\"GOOGLE_GEMINI_BASE_URL\":\"https://gemini.example\"}}"}).to_string()]).unwrap();
    conn.execute("INSERT INTO claude_desktop_provider VALUES ('four', jsonb(?1))", [json!({"name":"Desktop", "category":"custom", "settings_config":{"env":{"ANTHROPIC_BASE_URL":"https://desktop.example", "ANTHROPIC_AUTH_TOKEN":"fixture-key"}}, "meta":{"apiFormat":"anthropic_messages"}}).to_string()]).unwrap();
    drop(conn);
    let before = std::fs::read(&path).unwrap();
    let scan = provider_import::scan(&sb.env, Source::AiToolbox, Some(dir)).unwrap();
    assert_eq!(scan.candidates.len(), 4);
    assert!(scan.candidates.iter().all(|c| c.skip_reason.is_none()));
    assert!(!serde_json::to_string(&scan.preview(1))
        .unwrap()
        .contains("fixture"));
    let selected = (0..4)
        .map(|index| Selection {
            index,
            api_key: None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        provider_import::apply(&sb.env, &scan, &selected)
            .unwrap()
            .created
            .len(),
        4
    );
    let saved = store::load(&sb.env).unwrap();
    let card = saved.provider(Tool::Codex, "ai-toolbox:one").unwrap();
    assert!(card.settings_config["auth"]["tokens"].is_null());
    assert_eq!(
        card.settings_config["auth"]["OPENAI_API_KEY"],
        "fixture-key"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    for tool in [Tool::Claude, Tool::Codex, Tool::Gemini, Tool::ClaudeDesktop] {
        assert!(saved.current(tool).is_none());
    }
}

fn superba_profile(key_ref: serde_json::Value) -> serde_json::Value {
    json!({"id":"relay", "name":"Relay", "site_id":"custom", "auth_mode":"apikey", "auth":"apikey", "base_url_override":"https://relay.example/v1", "key_ref":key_ref, "model":{"model":"test-model", "reasoning_effort":"high"}})
}

#[test]
fn superba_keyring_requires_explicit_key_and_ignores_foreign_secret_paths() {
    let sb = Sandbox::new();
    let dir = sb.home().join("superba");
    let profile = superba_profile(
        json!({"kind":"keyring", "service":"superba", "account":"profile:relay:install"}),
    );
    sb.write(
        &dir.join("profiles.json"),
        &json!({"schema":1,"profiles":[profile]}).to_string(),
    );
    let scan = provider_import::scan(&sb.env, Source::Superba, Some(dir.clone())).unwrap();
    assert!(scan.candidates[0].needs_api_key);
    assert!(
        scan.candidates[0].skip_reason.is_none(),
        "{:?}",
        scan.candidates[0].skip_reason
    );
    assert!(provider_import::apply(
        &sb.env,
        &scan,
        &[Selection {
            index: 0,
            api_key: None
        }]
    )
    .is_err());
    assert!(!sb.env.store_path().exists());
    let report = provider_import::apply(
        &sb.env,
        &scan,
        &[Selection {
            index: 0,
            api_key: Some("fixture-user-key".into()),
        }],
    )
    .unwrap();
    assert_eq!(report.created.len(), 1);
    assert_eq!(
        store::load(&sb.env).unwrap().providers(Tool::Codex)[0].settings_config["auth"]
            ["OPENAI_API_KEY"],
        "fixture-user-key"
    );
    let outside = sb.home().join("outside.secret");
    sb.write(&outside, "do-not-import");
    std::fs::create_dir_all(dir.join("secrets")).unwrap();
    let profile = superba_profile(json!({"kind":"file", "path":outside}));
    sb.write(
        &dir.join("profiles.json"),
        &json!({"schema":1,"profiles":[profile]}).to_string(),
    );
    let scan = provider_import::scan(&sb.env, Source::Superba, Some(dir)).unwrap();
    assert!(scan.candidates[0].needs_api_key);
    assert!(!scan.candidates[0]
        .provider
        .settings_config
        .to_string()
        .contains("do-not-import"));
}

#[test]
fn superba_private_file_key_and_incompatible_profiles() {
    let sb = Sandbox::new();
    let dir = sb.home().join("superba");
    let secret = dir.join("secrets/profile-relay.secret");
    sb.write(&secret, "fixture-private-key");
    let profile = superba_profile(json!({"kind":"file", "path":secret}));
    let mut pat = superba_profile(json!(null));
    pat["id"] = "pat".into();
    pat["auth"] = "pat".into();
    let mut official = superba_profile(json!(null));
    official["id"] = "official".into();
    official["builtin"] = "official".into();
    sb.write(
        &dir.join("profiles.json"),
        &json!({"schema":1,"profiles":[profile,pat,official]}).to_string(),
    );
    let scan = provider_import::scan(&sb.env, Source::Superba, Some(dir)).unwrap();
    assert!(!scan.candidates[0].needs_api_key);
    assert_eq!(
        scan.candidates[0].provider.settings_config["auth"]["OPENAI_API_KEY"],
        "fixture-private-key"
    );
    assert!(scan.candidates[1].skip_reason.is_some());
    assert!(scan.candidates[2].skip_reason.is_some());
    assert!(provider_import::apply(
        &sb.env,
        &scan,
        &[Selection {
            index: 1,
            api_key: None
        }]
    )
    .is_err());
}

#[test]
fn malformed_inputs_and_invalid_selections_never_mutate_the_store() {
    let sb = Sandbox::new();
    let dir = sb.home().join("superba");
    sb.write(&dir.join("profiles.json"), r#"{"schema":99,"profiles":[]}"#);
    assert!(provider_import::scan(&sb.env, Source::Superba, Some(dir.clone())).is_err());
    sb.write(
        &dir.join("profiles.json"),
        r#"{"profiles": "fixture-secret"}"#,
    );
    let error = provider_import::scan(&sb.env, Source::Superba, Some(dir.clone()))
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("fixture-secret"));
    sb.write(
        &dir.join("profiles.json"),
        &json!({"profiles":[superba_profile(json!(null))]}).to_string(),
    );
    let scan = provider_import::scan(&sb.env, Source::Superba, Some(dir)).unwrap();
    assert!(provider_import::apply(
        &sb.env,
        &scan,
        &[Selection {
            index: 99,
            api_key: None
        }]
    )
    .is_err());
    assert!(!sb.env.store_path().exists());
    sb.add(
        Tool::Codex,
        Provider::new("superba:relay", "Existing", json!({"auth":{},"config":""})).official(),
    );
    let before = sb.read(&sb.env.store_path());
    let report = provider_import::apply(
        &sb.env,
        &scan,
        &[Selection {
            index: 0,
            api_key: Some("fixture-key".into()),
        }],
    )
    .unwrap();
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(sb.read(&sb.env.store_path()), before);
}

#[test]
fn ccswitch_provider_only_scan_ignores_a_broken_account_store() {
    let sb = Sandbox::new();
    let dir = sb.home().join(".cc-switch");
    sb.write(&dir.join("config.json"), r#"{"claude":{"providers":{}}}"#);
    sb.write(&dir.join("codex_oauth_auth.json"), "invalid-account-store");
    assert!(provider_import::scan(&sb.env, Source::CcSwitch, None).is_ok());
}

#[cfg(unix)]
#[test]
fn superba_never_follows_a_secret_directory_symlink_outside_the_source() {
    let sb = Sandbox::new();
    let dir = sb.home().join("superba");
    let outside = sb.home().join("private");
    let secret = outside.join("profile-other.secret");
    sb.write(&secret, "unrelated-secret");
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("secrets")).unwrap();
    sb.write(
        &dir.join("profiles.json"),
        &json!({"profiles":[superba_profile(json!({"kind":"file","path":secret}))]}).to_string(),
    );
    let scan = provider_import::scan(&sb.env, Source::Superba, Some(dir)).unwrap();
    assert!(scan.candidates[0].needs_api_key);
}

#[test]
fn aitoolbox_rejects_malformed_auth_and_marks_protocol_conversions() {
    let sb = Sandbox::new();
    let dir = sb.home().join("toolbox");
    std::fs::create_dir_all(&dir).unwrap();
    let conn = rusqlite::Connection::open(dir.join("ai-toolbox.db")).unwrap();
    conn.execute_batch("CREATE TABLE codex_provider(id TEXT, data TEXT);")
        .unwrap();
    conn.execute(
        "INSERT INTO codex_provider VALUES ('one', ?1)",
        [
            json!({"name":"Invalid", "settings_config":{"auth":"fixture-secret","config":""}})
                .to_string(),
        ],
    )
    .unwrap();
    let error = provider_import::scan(&sb.env, Source::AiToolbox, Some(dir.clone()))
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("fixture-secret"));
    conn.execute("UPDATE codex_provider SET data=?1", [json!({"name":"Gateway", "settings_config":{"auth":{},"config":""}, "meta":{"api_format":"openai_chat"}}).to_string()]).unwrap();
    let scan = provider_import::scan(&sb.env, Source::AiToolbox, Some(dir)).unwrap();
    assert_eq!(
        scan.candidates[0].skip_reason.as_ref().unwrap().code,
        "gateway"
    );
}

#[test]
fn aitoolbox_bootstrap_override_is_resolved_under_the_injected_home() {
    let sb = Sandbox::new();
    let default = provider_import::default_dir(&sb.env, Source::AiToolbox).unwrap();
    assert!(default.starts_with(sb.home()));
    let custom = sb.home().join("custom-ai-toolbox");
    sb.write(
        &default.join("app_paths.json"),
        &json!({"data_dir_override":custom}).to_string(),
    );
    assert_eq!(
        provider_import::default_dir(&sb.env, Source::AiToolbox).unwrap(),
        custom
    );
}
