use leanflip_core::{Env, Provider, Tool};
use leanflip_service::Service;
use serde_json::json;
use std::fs;

fn setup() -> (tempfile::TempDir, Env, Service) {
    let root = tempfile::tempdir().unwrap();
    let env = Env::with_home(root.path());
    let service = Service::start(env.clone(), None).unwrap();
    (root, env, service)
}
#[test]
fn common_settings_roundtrip_for_every_agent_without_touching_provider_records() {
    for tool in Tool::ALL {
        let (_root, env, service) = setup();
        let initial = service.common_config(tool).unwrap();
        let config = match tool {
            Tool::Claude => {
                json!({"env":{"CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS":"1","DISABLE_AUTOUPDATER":"1"},"attribution":{"commit":"","pr":""}})
            }
            Tool::Codex => {
                json!({"features":{"shell_snapshot":false,"multi_agent":true},"tui":{"notifications":true}})
            }
            Tool::Gemini => json!({"general":{"vimMode":true,"enableAutoUpdate":false}}),
            Tool::OpenCode => json!({"autoupdate":false,"share":"disabled"}),
            _ => json!({"test_option":{"enabled":true,"label":"non-secret fixture"}}),
        };
        let before = fs::read(env.store_path()).ok();
        service
            .save_common_config(tool, &initial.revision, config.clone())
            .unwrap();
        let saved = service.common_config(tool).unwrap();
        assert_eq!(saved.config, config, "{tool}");
        assert_eq!(fs::read(env.store_path()).ok(), before, "{tool}");
        assert!(
            service
                .save_common_config(tool, &initial.revision, json!({}))
                .is_err(),
            "stale write for {tool}"
        );
        service
            .save_common_config(tool, &saved.revision, json!({}))
            .unwrap();
        assert_eq!(
            service.common_config(tool).unwrap().config,
            json!({}),
            "reset for {tool}"
        );
    }
}
#[test]
fn claude_shared_preferences_survive_switch_and_keep_three_official_cards() {
    let (_root, env, service) = setup();
    for id in ["official", "uuid-one", "uuid-two"] {
        service
            .add_provider(
                Tool::Claude,
                Provider::new(id, "Official", json!({})).official(),
                false,
            )
            .unwrap();
    }
    let snapshot = service.common_config(Tool::Claude).unwrap();
    let common = json!({"env":{"CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS":"1"},"permissions":{"allow":["Read"]}});
    service
        .save_common_config(Tool::Claude, &snapshot.revision, common.clone())
        .unwrap();
    for id in ["official", "uuid-one", "uuid-two"] {
        service.switch_provider(Tool::Claude, id).unwrap();
    }
    let state = service.list_providers(Tool::Claude).unwrap();
    assert_eq!(state.providers.len(), 3);
    assert_eq!(state.current.as_deref(), Some("uuid-two"));
    assert_eq!(service.common_config(Tool::Claude).unwrap().config, common);
    let captured = service.import_live(Tool::Claude).unwrap();
    assert!(captured.created.is_empty());
    assert_eq!(
        service
            .list_providers(Tool::Claude)
            .unwrap()
            .selected
            .as_deref(),
        Some("uuid-two")
    );
    assert_eq!(
        service
            .list_providers(Tool::Claude)
            .unwrap()
            .providers
            .len(),
        3
    );
    assert!(fs::read(env.claude_settings_path()).is_ok());
}
#[test]
fn protected_credentials_comments_and_external_writes_are_preserved() {
    let (_root, env, service) = setup();
    fs::create_dir_all(&env.codex_dir).unwrap();
    let original = "# keep user comment\nmodel_provider='custom'\n[model_providers.custom]\nbase_url='https://example.invalid'\nexperimental_bearer_token='fake'\n# keep route comment\n[features]\nshell_snapshot = true # preserve if unchanged\n";
    fs::write(env.codex_config_path(), original).unwrap();
    let snapshot = service.common_config(Tool::Codex).unwrap();
    assert!(!snapshot.config.to_string().contains("fake"));
    assert!(service
        .save_common_config(
            Tool::Codex,
            &snapshot.revision,
            json!({"model_providers":{}})
        )
        .is_err());
    service
        .save_common_config(
            Tool::Codex,
            &snapshot.revision,
            json!({"features":{"shell_snapshot":false}}),
        )
        .unwrap();
    let text = fs::read_to_string(env.codex_config_path()).unwrap();
    assert!(text.contains("# keep user comment") && text.contains("# keep route comment"));
    assert!(text.contains("experimental_bearer_token='fake'"));
    let snap = service.common_config(Tool::Codex).unwrap();
    fs::write(
        env.codex_config_path(),
        format!("{text}\n[external]\nchanged=true\n"),
    )
    .unwrap();
    assert!(service
        .save_common_config(Tool::Codex, &snap.revision, json!({}))
        .is_err());
    assert!(fs::read_to_string(env.codex_config_path())
        .unwrap()
        .contains("changed=true"));
}

#[test]
fn common_edits_can_clear_last_preference_without_erasing_nested_credentials() {
    let (_root, env, service) = setup();
    fs::create_dir_all(&env.claude_dir).unwrap();
    fs::write(
        env.claude_settings_path(),
        r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"fake","DISABLE_AUTOUPDATER":"1"}}"#,
    )
    .unwrap();
    let before = service.common_config(Tool::Claude).unwrap();
    service
        .save_common_config(Tool::Claude, &before.revision, json!({}))
        .unwrap();
    let actual: serde_json::Value =
        serde_json::from_slice(&fs::read(env.claude_settings_path()).unwrap()).unwrap();
    assert_eq!(actual, json!({"env":{"ANTHROPIC_AUTH_TOKEN":"fake"}}));
    let before = service.common_config(Tool::Claude).unwrap();
    service
        .save_common_config(Tool::Claude, &before.revision, json!({"permissions":{}}))
        .unwrap();
    assert_eq!(
        service.common_config(Tool::Claude).unwrap().config,
        json!({"permissions":{}})
    );
}

#[test]
fn invalid_quick_option_types_are_rejected_before_writing() {
    let (_root, env, service) = setup();
    let before = service.common_config(Tool::Codex).unwrap();
    assert!(service
        .save_common_config(
            Tool::Codex,
            &before.revision,
            json!({"features":{"multi_agent":"true"}})
        )
        .is_err());
    assert!(!env.codex_config_path().exists());
}

#[test]
fn codex_capacity_follows_provider_while_shared_execution_preferences_survive() {
    let (_root, env, service) = setup();
    fs::create_dir_all(&env.codex_dir).unwrap();
    fs::write(
        env.codex_config_path(),
        "# personal comment\n[features]\ncustom_flag = true\n",
    )
    .unwrap();
    let snap = service.common_config(Tool::Codex).unwrap();
    let common = json!({"approval_policy":"on-request","sandbox_mode":"workspace-write","tool_output_token_limit":12000,"features":{"custom_flag":true}});
    service
        .save_common_config(Tool::Codex, &snap.revision, common.clone())
        .unwrap();
    for (id, window, limit) in [("one", 1000000, 900000), ("two", 200000, 150000)] {
        service.add_provider(Tool::Codex, Provider::new(id, id, json!({"auth":{},"config":format!("model='test-model'\nmodel_context_window={window}\nmodel_auto_compact_token_limit={limit}\nweb_search='cached'\nmodel_verbosity='low'\n")})).official(), false).unwrap();
    }
    for (id, window, limit) in [
        ("one", 1000000, 900000),
        ("two", 200000, 150000),
        ("one", 1000000, 900000),
    ] {
        service.switch_provider(Tool::Codex, id).unwrap();
        let text = fs::read_to_string(env.codex_config_path()).unwrap();
        let parsed: toml_edit::DocumentMut = text.parse().unwrap();
        assert_eq!(parsed["model_context_window"].as_integer(), Some(window));
        assert_eq!(
            parsed["model_auto_compact_token_limit"].as_integer(),
            Some(limit)
        );
        assert_eq!(parsed["web_search"].as_str(), Some("cached"));
        assert_eq!(parsed["model_verbosity"].as_str(), Some("low"));
        assert!(text.contains("# personal comment"));
        assert_eq!(service.common_config(Tool::Codex).unwrap().config, common);
    }
    // Client-side capacity edits are preferences, not a change of provider identity.
    let mut edited = fs::read_to_string(env.codex_config_path())
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    edited["model_context_window"] = toml_edit::value(200000);
    edited["model_auto_compact_token_limit"] = toml_edit::value(150000);
    fs::write(env.codex_config_path(), edited.to_string()).unwrap();
    assert_eq!(
        service
            .list_providers(Tool::Codex)
            .unwrap()
            .current
            .as_deref(),
        Some("one")
    );
}

#[test]
fn claude_shortcuts_write_the_right_scope_and_reset_without_losing_rules() {
    let (_root, env, service) = setup();
    let before = service.common_config(Tool::Claude).unwrap();
    let common = json!({"permissions":{"defaultMode":"bypassPermissions","allow":["Read"],"deny":["Read(.env)"]},"promptCacheTtl":"5m","subagentPromptCacheTtl":"1h"});
    service
        .save_common_config(Tool::Claude, &before.revision, common.clone())
        .unwrap();
    let capacity = json!({"env":{"ANTHROPIC_BASE_URL":"https://qa.invalid","ANTHROPIC_AUTH_TOKEN":"fake","CLAUDE_CODE_MAX_CONTEXT_TOKENS":"262144","CLAUDE_CODE_AUTO_COMPACT_WINDOW":"200000","CLAUDE_CODE_MAX_OUTPUT_TOKENS":"64000"}});
    service
        .add_provider(Tool::Claude, Provider::new("qa", "QA", capacity), true)
        .unwrap();
    let actual: serde_json::Value =
        serde_json::from_slice(&fs::read(env.claude_settings_path()).unwrap()).unwrap();
    assert_eq!(actual["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "262144");
    assert_eq!(actual["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "200000");
    assert_eq!(actual["env"]["CLAUDE_CODE_MAX_OUTPUT_TOKENS"], "64000");
    let snap = service.common_config(Tool::Claude).unwrap();
    assert_eq!(snap.config, common);
    let mut reset = common;
    reset["permissions"]
        .as_object_mut()
        .unwrap()
        .remove("defaultMode");
    service
        .save_common_config(Tool::Claude, &snap.revision, reset.clone())
        .unwrap();
    let actual: serde_json::Value =
        serde_json::from_slice(&fs::read(env.claude_settings_path()).unwrap()).unwrap();
    assert_eq!(actual["permissions"], reset["permissions"]);
    assert_eq!(actual["env"]["ANTHROPIC_AUTH_TOKEN"], "fake");
    assert_eq!(actual["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "262144");
    service
        .add_provider(
            Tool::Claude,
            Provider::new("official", "Official", json!({})).official(),
            true,
        )
        .unwrap();
    let actual: serde_json::Value =
        serde_json::from_slice(&fs::read(env.claude_settings_path()).unwrap()).unwrap();
    assert!(actual["env"]
        .get("CLAUDE_CODE_MAX_CONTEXT_TOKENS")
        .is_none());
    let shared = service.common_config(Tool::Claude).unwrap().config;
    for key in ["permissions", "promptCacheTtl", "subagentPromptCacheTtl"] {
        assert_eq!(shared[key], reset[key]);
    }
    // A malformed client-side preference must not hide the selected connection.
    let mut edited = actual;
    edited["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"] = json!("user-entered-value");
    fs::write(env.claude_settings_path(), edited.to_string()).unwrap();
    assert_eq!(
        service
            .list_providers(Tool::Claude)
            .unwrap()
            .current
            .as_deref(),
        Some("official")
    );
}

#[test]
fn invalid_capacity_and_budget_values_never_reach_client_files() {
    let (_root, env, service) = setup();
    for config in [
        "model_context_window='1000000'",
        "model_context_window=0",
        "model_auto_compact_token_limit=-1",
        "model_context_window=200000\nmodel_auto_compact_token_limit=900000",
    ] {
        let bad =
            Provider::new("invalid", "Invalid", json!({"auth":{},"config":config})).official();
        assert!(service.add_provider(Tool::Codex, bad, true).is_err());
        assert!(!env.codex_config_path().exists());
    }
    let snap = service.common_config(Tool::Codex).unwrap();
    for value in [json!(-1), json!(1.5), json!("1000")] {
        assert!(service
            .save_common_config(
                Tool::Codex,
                &snap.revision,
                json!({"tool_output_token_limit":value})
            )
            .is_err());
        assert!(!env.codex_config_path().exists());
    }
    for value in ["0", "64000oops", "128001"] {
        assert!(service
            .add_provider(
                Tool::Claude,
                Provider::new(
                    "invalid",
                    "Invalid",
                    json!({"env":{"CLAUDE_CODE_MAX_OUTPUT_TOKENS":value}})
                ),
                true
            )
            .is_err());
        assert!(!env.claude_settings_path().exists());
    }
}
