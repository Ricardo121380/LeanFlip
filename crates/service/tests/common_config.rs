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
