// Regression cases from the 2026-10-14 audit. All writes are confined to tempfile directories.
#[cfg(test)]
mod tests {
    use leanflip_core::{Env, Provider, Tool};
    use leanflip_service::{service::SettingsInput, Service};
    use serde_json::json;
    use std::fs;

    fn setup() -> (tempfile::TempDir, Env, Service) {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        let service = Service::start(env.clone(), None).unwrap();
        (dir, env, service)
    }
    fn card(id: &str) -> Provider {
        Provider::new(
            id,
            id,
            json!({"env":{"ANTHROPIC_BASE_URL":format!("https://{id}.example.invalid"),"ANTHROPIC_AUTH_TOKEN":"audit-fake-key"}}),
        )
    }
    fn live(env: &Env, value: &str) {
        fs::create_dir_all(&env.claude_dir).unwrap();
        fs::write(env.claude_settings_path(), value).unwrap();
    }

    #[test]
    fn import_current_after_existing_provider_should_capture_new_config() {
        let (_dir, env, service) = setup();
        service
            .add_provider(Tool::Claude, card("a"), false)
            .unwrap();
        live(&env, &card("b").settings_config.to_string());
        let result = service.import_live(Tool::Claude).unwrap();
        assert!(!result.created.is_empty(), "BUG: import current returned no created providers even though live contains a new endpoint and credentials");
    }

    #[test]
    fn external_change_should_not_keep_claiming_old_provider_in_use() {
        let (_dir, env, service) = setup();
        service.add_provider(Tool::Claude, card("a"), true).unwrap();
        service
            .add_provider(Tool::Claude, card("b"), false)
            .unwrap();
        live(&env, &card("b").settings_config.to_string());
        let data = service.list_providers(Tool::Claude).unwrap();
        assert!(
            data.current.as_deref() != Some("a") || data.live_error.is_some(),
            "BUG: current=a and live_error=None after live file was replaced by provider b"
        );
    }

    #[test]
    fn invalid_live_config_should_surface_read_error() {
        let (_dir, env, service) = setup();
        service.add_provider(Tool::Claude, card("a"), true).unwrap();
        live(&env, "{broken-json");
        let data = service.list_providers(Tool::Claude).unwrap();
        assert!(
            data.live_error.is_some(),
            "BUG: malformed live JSON is reported without a live_error"
        );
        let report = service.usage_report().unwrap();
        assert_eq!(
            report
                .events
                .iter()
                .rev()
                .find(|e| e.tool == Tool::Claude)
                .unwrap()
                .provider,
            "a",
            "An unreadable file is not evidence that the user deselected the provider"
        );
    }

    #[test]
    fn failed_add_and_activate_should_not_leave_saved_provider() {
        let (_dir, env, service) = setup();
        live(&env, "{broken-json");
        let result = service.add_provider(Tool::Claude, card("a"), true);
        assert!(result.is_err());
        let providers = service.list_providers(Tool::Claude).unwrap().providers;
        assert!(providers.is_empty(), "BUG: save-and-activate reported failure but provider a remains saved, so retry hits duplicate ID");
    }

    #[test]
    fn claude_invalid_env_value_types_should_be_rejected() {
        let (_dir, env, service) = setup();
        let result = service.add_provider(
            Tool::Claude,
            Provider::new(
                "bad",
                "bad",
                json!({"env":{"ANTHROPIC_BASE_URL":123,"ANTHROPIC_AUTH_TOKEN":["not-a-string"]}}),
            ),
            true,
        );
        let wrote_invalid_live = env.claude_settings_path().is_file();
        assert!(
            result.is_err() && !wrote_invalid_live,
            "BUG: numeric URL and array token accepted and written into Claude settings.json"
        );
    }

    #[test]
    fn claude_final_usage_should_replace_earlier_partial_usage() {
        let (_dir, env, service) = setup();
        let folder = env.claude_dir.join("projects/audit");
        fs::create_dir_all(&folder).unwrap();
        let ts = chrono::Utc::now().to_rfc3339();
        let line = |output| {
            json!({"type":"assistant","timestamp":ts,"requestId":"audit-request","message":{"id":"audit-message","model":"audit-model","usage":{"input_tokens":100,"output_tokens":output}}}).to_string()+"\n"
        };
        let path = folder.join("audit.jsonl");
        fs::write(&path, line(1)).unwrap();
        service.usage_report().unwrap();
        fs::write(&path, line(1) + &line(20)).unwrap();
        let report = service.usage_report().unwrap();
        let actual: i64 = report
            .rows
            .iter()
            .filter(|r| r.tool == Tool::Claude)
            .map(|r| r.output)
            .sum();
        assert_eq!(
            actual, 20,
            "BUG: same message with final usage is discarded by first-seen dedup"
        );
    }

    #[test]
    fn codex_copied_history_should_not_count_usage_twice() {
        let (_dir, env, service) = setup();
        let folder = env.codex_dir.join("sessions/2026/10/14");
        fs::create_dir_all(&folder).unwrap();
        let ts = chrono::Utc::now().to_rfc3339();
        let counts = json!({"input_tokens":100,"cached_input_tokens":40,"output_tokens":20,"total_tokens":120});
        let content = json!({"type":"session_meta","payload":{"id":"audit-session","model_provider":"openai"}}).to_string()+"\n"+
            &json!({"type":"turn_context","payload":{"model":"audit-model"}}).to_string()+"\n"+
            &json!({"type":"event_msg","timestamp":ts,"payload":{"type":"token_count","info":{"total_token_usage":counts,"last_token_usage":counts}}}).to_string()+"\n";
        fs::write(folder.join("rollout-a.jsonl"), &content).unwrap();
        fs::write(folder.join("rollout-b.jsonl"), &content).unwrap();
        let actual: i64 = service
            .usage_report()
            .unwrap()
            .rows
            .iter()
            .filter(|r| r.tool == Tool::Codex)
            .map(|r| r.input + r.cache_read + r.output)
            .sum();
        assert_eq!(
            actual, 120,
            "BUG: identical session history copied into another rollout file is counted again"
        );
    }

    #[test]
    fn control_switch_preserves_unrelated_client_settings() {
        let (_dir, env, service) = setup();
        live(
            &env,
            r#"{"env":{"API_TIMEOUT_MS":"999"},"permissions":{"allow":["Read"]}}"#,
        );
        service.add_provider(Tool::Claude, card("a"), true).unwrap();
        let actual: serde_json::Value =
            serde_json::from_slice(&fs::read(env.claude_settings_path()).unwrap()).unwrap();
        assert_eq!(actual["env"]["API_TIMEOUT_MS"], "999");
        assert_eq!(actual["permissions"]["allow"], json!(["Read"]));
    }

    #[test]
    fn relative_config_directory_should_be_rejected_instead_of_saved_and_ignored() {
        let (_dir, env, service) = setup();
        let mut app = service.app_settings();
        app.dir_overrides.claude_dir = Some("relative/config".into());
        let result = service.save_settings(SettingsInput {
            app,
            keep_codex_login: false,
        });
        assert!(result.is_err(), "BUG: settings save returned success for a relative path, but effective Claude directory stayed {}",env.claude_dir.display());
    }

    #[test]
    fn failed_settings_save_should_not_persist_the_new_app_settings() {
        let (_dir, env, service) = setup();
        let path = env.data_file("app-settings.json");
        let before = fs::read(&path).ok();
        fs::create_dir_all(&env.data_dir).unwrap();
        fs::write(env.store_path(), "{broken-json").unwrap();
        let mut app = service.app_settings();
        app.theme = Some("light".into());
        let result = service.save_settings(SettingsInput {
            app,
            keep_codex_login: false,
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read(&path).ok(),
            before,
            "BUG: failed save changed app-settings.json even though runtime settings stay old"
        );
    }
    #[test]
    fn preferences_and_unused_codex_tables_do_not_change_provider_identity() {
        let (_dir, env, service) = setup();
        for (tool, card) in [
            (
                Tool::Claude,
                Provider::new("official", "Official", json!({})).official(),
            ),
            (
                Tool::Codex,
                Provider::new(
                    "official",
                    "Official",
                    json!({"auth":{}, "config":"model='old'\nmodel_reasoning_effort='high'\n"}),
                )
                .official(),
            ),
        ] {
            service.add_provider(tool, card, true).unwrap();
        }
        live(
            &env,
            r#"{"model":"new","env":{"ANTHROPIC_MODEL":"new","ANTHROPIC_DEFAULT_OPUS_MODEL":"new","CLAUDE_CODE_EFFORT_LEVEL":"max"}}"#,
        );
        fs::write(env.codex_config_path(), "model='new'\nmodel_reasoning_effort='low'\n[model_providers.unused]\nbase_url='https://unused.example'\nexperimental_bearer_token='fake'\n").unwrap();
        for tool in [Tool::Claude, Tool::Codex] {
            let state = service.list_providers(tool).unwrap();
            assert_eq!(
                state.current.as_deref(),
                Some("official"),
                "{tool}: {:?}",
                state.live_error
            );
        }
        // Real route or credential changes must still be detected.
        live(&env, r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"different-key"}}"#);
        assert_eq!(service.list_providers(Tool::Claude).unwrap().current, None);
        live(&env, r#"{"env":[]}"#);
        assert!(service
            .list_providers(Tool::Claude)
            .unwrap()
            .live_error
            .is_some());
        fs::write(env.codex_config_path(), "model_provider='relay'\n[model_providers.relay]\nbase_url='https://relay.example'\nexperimental_bearer_token='fake'\n").unwrap();
        assert_eq!(service.list_providers(Tool::Codex).unwrap().current, None);
    }

    #[test]
    fn duplicate_capture_preserves_all_official_cards_and_the_selected_pointer() {
        let (_dir, env, service) = setup();
        for id in ["official", "uuid-one", "uuid-two"] {
            service
                .add_provider(
                    Tool::Claude,
                    Provider::new(id, "Official", json!({})).official(),
                    false,
                )
                .unwrap();
        }
        service.switch_provider(Tool::Claude, "uuid-two").unwrap();
        let before = fs::read(env.store_path()).unwrap();
        for _ in 0..2 {
            let imported = service.import_live(Tool::Claude).unwrap();
            assert!(imported.created.is_empty());
            assert_eq!(
                service
                    .list_providers(Tool::Claude)
                    .unwrap()
                    .selected
                    .as_deref(),
                Some("uuid-two")
            );
        }
        assert_eq!(
            service
                .list_providers(Tool::Claude)
                .unwrap()
                .providers
                .len(),
            3
        );
        assert_eq!(fs::read(env.store_path()).unwrap(), before);
        // A different stored selection is not changed by finding a matching card.
        service
            .add_provider(Tool::Claude, card("relay"), true)
            .unwrap();
        live(&env, "{}");
        let imported = service.import_live(Tool::Claude).unwrap();
        assert!(imported.created.is_empty());
        assert_eq!(
            service
                .list_providers(Tool::Claude)
                .unwrap()
                .selected
                .as_deref(),
            Some("relay")
        );
    }
    #[test]
    fn missing_login_reference_is_rejected_before_insert_or_update() {
        let (_root, _env, service) = setup();
        let mut card =
            Provider::new("official", "Official", json!({"auth":{},"config":""})).official();
        service
            .add_provider(Tool::Codex, card.clone(), false)
            .unwrap();
        card.set_codex_login_ref(Some("missing-login"));
        assert!(service.update_provider(Tool::Codex, card.clone()).is_err());
        assert!(service.list_providers(Tool::Codex).unwrap().providers[0]
            .codex_login_ref()
            .is_none());
        card.id = "new".into();
        assert!(service.add_provider(Tool::Codex, card, false).is_err());
        assert_eq!(
            service.list_providers(Tool::Codex).unwrap().providers.len(),
            1
        );
    }

    #[test]
    fn every_switch_agent_reports_verified_current_and_detects_external_changes() {
        let cases = [
            (
                Tool::Claude,
                json!({"env":{"ANTHROPIC_BASE_URL":"https://relay.example","ANTHROPIC_AUTH_TOKEN":"fake"}}),
            ),
            (
                Tool::Codex,
                json!({"auth":{"OPENAI_API_KEY":"fake"},"config":"model_provider='relay'\nmodel='test'\n[model_providers.relay]\nname='relay'\nbase_url='https://relay.example/v1'\nwire_api='responses'\nrequires_openai_auth=true\n"}),
            ),
            (
                Tool::Gemini,
                json!({"env":{"GEMINI_API_KEY":"fake","GOOGLE_GEMINI_BASE_URL":"https://relay.example"},"config":{}}),
            ),
            (
                Tool::GrokBuild,
                json!({"config":"[models]\ndefault='test'\n[model.test]\nmodel='test'\nname='relay'\nbase_url='https://relay.example/v1'\napi_key='fake'\napi_backend='responses'\ncontext_window=1000\n"}),
            ),
            (
                Tool::ClaudeDesktop,
                json!({"env":{"ANTHROPIC_BASE_URL":"https://relay.example","ANTHROPIC_AUTH_TOKEN":"fake"}}),
            ),
        ];
        for (tool, config) in cases {
            let (_root, env, service) = setup();
            service
                .add_provider(tool, Provider::new("relay", "Relay", config), true)
                .unwrap();
            let state = service.list_providers(tool).unwrap();
            assert_eq!(
                state.current.as_deref(),
                Some("relay"),
                "{tool}: {:?}",
                state.live_error
            );
            let path = env.live_paths(tool)[0].clone();
            fs::write(path, "invalid syntax").unwrap();
            let state = service.list_providers(tool).unwrap();
            assert_eq!(state.current, None, "{tool}");
            assert_eq!(state.selected.as_deref(), Some("relay"));
        }
    }
}
