use super::*;
use serde_json::json;

fn card(config: Value) -> Provider {
    Provider::new("work", "Work", config)
}
fn update() -> ConnectionUpdate {
    ConnectionUpdate {
        name: "Renamed".into(),
        base_url: Some("https://new.example/v1".into()),
        api_key: ApiKeyChange::Set("replacement-test-key".into()),
    }
}
#[test]
fn all_json_tool_shapes_display_endpoint_and_edit_without_losing_models_or_metadata() {
    for (tool, config, url, key) in [
        (
            Tool::Claude,
            json!({"env":{"ANTHROPIC_BASE_URL":"https://old.example","ANTHROPIC_API_KEY":"old-test-key","MODEL":"keep"}}),
            "/env/ANTHROPIC_BASE_URL",
            "/env/ANTHROPIC_API_KEY",
        ),
        (
            Tool::ClaudeDesktop,
            json!({"env":{"ANTHROPIC_BASE_URL":"https://old.example","ANTHROPIC_AUTH_TOKEN":"old-test-key"}}),
            "/env/ANTHROPIC_BASE_URL",
            "/env/ANTHROPIC_AUTH_TOKEN",
        ),
        (
            Tool::Gemini,
            json!({"env":{"GOOGLE_GEMINI_BASE_URL":"https://old.example","GEMINI_API_KEY":"old-test-key"}}),
            "/env/GOOGLE_GEMINI_BASE_URL",
            "/env/GEMINI_API_KEY",
        ),
        (
            Tool::OpenCode,
            json!({"options":{"baseURL":"https://old.example","apiKey":"old-test-key"},"models":{"fixture":{}}}),
            "/options/baseURL",
            "/options/apiKey",
        ),
        (
            Tool::Mcode,
            json!({"options":{"baseURL":"https://old.example","apiKey":"old-test-key"}}),
            "/options/baseURL",
            "/options/apiKey",
        ),
        (
            Tool::OpenClaw,
            json!({"baseUrl":"https://old.example","apiKey":"old-test-key"}),
            "/baseUrl",
            "/apiKey",
        ),
        (
            Tool::Pi,
            json!({"baseUrl":"https://old.example","apiKey":"old-test-key"}),
            "/baseUrl",
            "/apiKey",
        ),
        (
            Tool::Hermes,
            json!({"base_url":"https://old.example","api_key":"old-test-key"}),
            "/base_url",
            "/api_key",
        ),
    ] {
        let mut provider = card(config);
        provider.settings_config["untouched"] = json!({"model":"custom","nested":[1,2]});
        let info = connection_info(tool, &provider);
        assert!(info.editable && info.api_key_set);
        assert_eq!(info.base_url.as_deref(), Some("https://old.example/"));
        assert!(!serde_json::to_string(&info)
            .unwrap()
            .contains("old-test-key"));
        patch(tool, &mut provider, update()).unwrap();
        assert_eq!(
            provider.settings_config.pointer(url).unwrap(),
            "https://new.example/v1"
        );
        assert_eq!(
            provider.settings_config.pointer(key).unwrap(),
            "replacement-test-key"
        );
        assert_eq!(
            provider.settings_config["untouched"],
            json!({"model":"custom","nested":[1,2]})
        );
        patch(
            tool,
            &mut provider,
            ConnectionUpdate {
                name: "Work".into(),
                base_url: Some(String::new()),
                api_key: ApiKeyChange::Keep,
            },
        )
        .unwrap();
        assert!(provider.settings_config.pointer(url).is_none());
        assert_eq!(
            provider.settings_config.pointer(key).unwrap(),
            "replacement-test-key"
        );
    }
}
#[test]
fn toml_edits_target_the_selected_provider_or_model_and_keep_comments() {
    for (tool,config,selected) in [
        (Tool::Codex,"# preserve comment\nmodel_provider = 'active'\nmodel = 'fixture-model'\n[model_providers.decoy]\nbase_url = 'https://decoy.example'\n[model_providers.active]\nbase_url = 'https://active.example'\nwire_api = 'responses'\n","[model_providers.decoy]"),
        (Tool::GrokBuild,"# preserve comment\n[models]\ndefault = 'active'\n[model.decoy]\nbase_url = 'https://decoy.example'\n[model.active]\nbase_url = 'https://active.example'\napi_key = 'old-test-key'\nmodel = 'fixture-model'\n","[model.decoy]"),
    ] {
        let mut provider=card(json!({"auth":{"OPENAI_API_KEY":"old-test-key"},"config":config}));
        assert_eq!(connection_info(tool,&provider).base_url.as_deref(),Some("https://active.example/"));
        patch(tool,&mut provider,update()).unwrap();
        let changed=provider.settings_config["config"].as_str().unwrap();assert!(changed.contains("# preserve comment"));assert!(changed.contains("https://decoy.example"));assert!(changed.contains(selected));assert!(changed.contains("https://new.example/v1"));
        assert!(connection_info(tool,&provider).api_key_set);
        patch(tool, &mut provider, ConnectionUpdate { name:"Work".into(), base_url:Some(String::new()), api_key:ApiKeyChange::Keep }).unwrap();
        let cleared = config_document(&provider.settings_config).unwrap();
        let (group, name) = table_path(tool, &cleared).unwrap();
        assert!(cleared[group][&name].get("base_url").is_none());
    }
}
#[test]
fn codex_without_model_provider_ignores_a_leftover_provider_table() {
    // Codex uses its built-in openai provider here, so the table's URL is not the endpoint.
    let mut provider = card(
        json!({"auth":{},"config":"model = 'm'\n[model_providers.custom]\nbase_url = 'https://unused.example/v1'\n"}),
    );
    let info = connection_info(Tool::Codex, &provider);
    assert_eq!(info.base_url, None);
    assert!(!info.editable);
    assert!(patch(Tool::Codex, &mut provider, update()).is_err());
    // Grok Build keeps using a lone model table when no default is named.
    let grok = card(json!({"config":"[model.only]\nbase_url = 'https://grok.example/v1'\n"}));
    assert_eq!(
        connection_info(Tool::GrokBuild, &grok).base_url.as_deref(),
        Some("https://grok.example/v1")
    );
}
#[test]
fn summaries_redact_url_credentials_and_official_login_is_not_replaced() {
    let mut provider = card(
        json!({"env":{"ANTHROPIC_BASE_URL":"https://user:secret@api.example/v1?token=private#fragment","ANTHROPIC_AUTH_TOKEN":"private-key"}}),
    );
    let info = connection_info(Tool::Claude, &provider);
    assert!(info.url_hidden);
    assert_eq!(info.base_url.as_deref(), Some("https://api.example/v1"));
    assert!(!serde_json::to_string(&info).unwrap().contains("private"));
    provider.category = Some("official".into());
    assert!(!connection_info(Tool::Claude, &provider).editable);
    assert!(patch(Tool::Claude, &mut provider, update()).is_err());
}
#[test]
fn stale_edits_fail_and_active_card_edits_follow_the_existing_live_write_contract() {
    let dir = tempfile::tempdir().unwrap();
    let service = Service::open(Env::with_home(dir.path()), None).unwrap();
    let provider = card(
        json!({"env":{"ANTHROPIC_BASE_URL":"https://old.example","ANTHROPIC_AUTH_TOKEN":"old-key","CUSTOM":"keep"}}),
    );
    service
        .add_provider(Tool::Claude, provider.clone(), false)
        .unwrap();
    service
        .update_connection(Tool::Claude, &provider, update())
        .unwrap();
    assert!(!service.env().claude_settings_path().exists());
    assert_eq!(
        service
            .update_connection(Tool::Claude, &provider, update())
            .unwrap_err()
            .code(),
        "conflict"
    );
    let before = service.list_providers(Tool::Claude).unwrap().providers[0].clone();
    let live_path = service.env().claude_settings_path();
    std::fs::create_dir_all(live_path.parent().unwrap()).unwrap();
    std::fs::write(&live_path, r#"{"env":{"CUSTOM":"keep"}}"#).unwrap();
    service.switch_provider(Tool::Claude, "work").unwrap();
    service
        .update_connection(
            Tool::Claude,
            &before,
            ConnectionUpdate {
                name: before.name.clone(),
                base_url: None,
                api_key: ApiKeyChange::Set("latest-test-key".into()),
            },
        )
        .unwrap();
    let live: Value =
        serde_json::from_slice(&std::fs::read(service.env().claude_settings_path()).unwrap())
            .unwrap();
    assert_eq!(live["env"]["ANTHROPIC_AUTH_TOKEN"], "latest-test-key");
    assert_eq!(live["env"]["CUSTOM"], "keep");
}

#[test]
fn key_references_are_configured_and_clearing_legacy_aliases_leaves_no_fallback() {
    let reference =
        card(json!({"baseUrl":"https://example.com", "apiKey":{"source":"env","id":"API_KEY"}}));
    assert!(connection_info(Tool::OpenClaw, &reference).api_key_set);
    let mut provider = card(
        json!({"base_url":"https://example.com", "baseUrl":"https://legacy.example", "api_key":"primary", "apiKey":"legacy"}),
    );
    patch(
        Tool::Hermes,
        &mut provider,
        ConnectionUpdate {
            name: "Work".into(),
            base_url: Some(String::new()),
            api_key: ApiKeyChange::Clear,
        },
    )
    .unwrap();
    assert!(!connection_info(Tool::Hermes, &provider).api_key_set);
    assert!(connection_info(Tool::Hermes, &provider).base_url.is_none());
    let cloud = card(json!({"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}));
    assert!(!connection_info(Tool::Claude, &cloud).editable);
}

#[test]
fn clearing_codex_key_clears_imported_bearer_sources_and_live_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    let service = Service::open(env.clone(), None).unwrap();
    let provider = card(
        json!({"auth":{},"config":"model_provider='custom'\nexperimental_bearer_token='fixture-root'\n[model_providers.custom]\nbase_url='https://fixture.invalid/v1'\nexperimental_bearer_token='fixture-original'\n"}),
    );
    assert!(connection_info(Tool::Codex, &provider).api_key_set);
    service.add_provider(Tool::Codex, provider, true).unwrap();
    for key in [
        ApiKeyChange::Set("fixture-replacement".into()),
        ApiKeyChange::Clear,
    ] {
        let data = store::load_readonly(&env).unwrap();
        let expected = redact::redact(Tool::Codex, data.provider(Tool::Codex, "work").unwrap());
        service
            .update_connection(
                Tool::Codex,
                &expected,
                ConnectionUpdate {
                    name: "Work".into(),
                    base_url: None,
                    api_key: key,
                },
            )
            .unwrap();
    }
    let data = store::load_readonly(&env).unwrap();
    let saved = data.provider(Tool::Codex, "work").unwrap();
    assert!(!connection_info(Tool::Codex, saved).api_key_set);
    for text in [
        saved.settings_config["config"]
            .as_str()
            .unwrap()
            .to_string(),
        std::fs::read_to_string(env.codex_config_path()).unwrap(),
    ] {
        assert!(!text.contains("experimental_bearer_token"));
        assert!(!text.contains("fixture-original"));
        assert!(!text.contains("fixture-root"));
        assert!(!text.contains("fixture-replacement"));
    }
}

#[test]
fn opencode_v2_connection_edits_update_the_native_live_fields() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    std::fs::create_dir_all(&env.opencode_dir).unwrap();
    std::fs::write(env.opencode_config_path(),json!({"providers":{"work":{"settings":{"baseURL":"https://old.example/v1","apiKey":"fixture-old"},"models":{"m":{"capabilities":{"tools":false}}}}}}).to_string()).unwrap();
    let service = Service::open(env.clone(), None).unwrap();
    service.import_live(Tool::OpenCode).unwrap();
    let data = store::load_readonly(&env).unwrap();
    let provider = data.provider(Tool::OpenCode, "work").unwrap();
    assert_eq!(
        connection_info(Tool::OpenCode, provider)
            .base_url
            .as_deref(),
        Some("https://old.example/v1")
    );
    service
        .update_connection(
            Tool::OpenCode,
            &redact::redact(Tool::OpenCode, provider),
            update(),
        )
        .unwrap();
    let live: Value =
        serde_json::from_slice(&std::fs::read(env.opencode_config_path()).unwrap()).unwrap();
    assert_eq!(
        live["providers"]["work"]["settings"]["baseURL"],
        "https://new.example/v1"
    );
    assert_eq!(
        live["providers"]["work"]["settings"]["apiKey"],
        "replacement-test-key"
    );
    assert!(live["providers"]["work"].get("options").is_none());
    assert_eq!(
        live["providers"]["work"]["models"]["m"]["capabilities"]["tools"],
        false
    );
}

#[test]
fn sync_link_checks_source_snapshot_and_never_rewrites_live_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    let service = Service::open(env.clone(), None).unwrap();
    let provider = card(
        json!({"env":{"ANTHROPIC_BASE_URL":"https://old.example","ANTHROPIC_AUTH_TOKEN":"fixture-old"}}),
    );
    service
        .add_provider(Tool::Claude, provider.clone(), true)
        .unwrap();
    service
        .update_connection(
            Tool::Claude,
            &redact::redact(Tool::Claude, &provider),
            update(),
        )
        .unwrap();
    let before = std::fs::read(env.claude_settings_path()).unwrap();
    assert_eq!(
        service
            .link_provider(
                Tool::Claude,
                redact::redact(Tool::Claude, &provider),
                "link-fixture".into()
            )
            .unwrap_err()
            .code(),
        "conflict"
    );
    assert_eq!(std::fs::read(env.claude_settings_path()).unwrap(), before);
    let data = store::load_readonly(&env).unwrap();
    let current = redact::redact(Tool::Claude, data.provider(Tool::Claude, "work").unwrap());
    for _ in 0..2 {
        service
            .link_provider(Tool::Claude, current.clone(), "link-fixture".into())
            .unwrap();
    }
    assert_eq!(std::fs::read(env.claude_settings_path()).unwrap(), before);
    let data = store::load_readonly(&env).unwrap();
    assert_eq!(
        data.provider(Tool::Claude, "work").unwrap().meta.extra["leanflipLink"],
        "link-fixture"
    );
    assert!(service
        .link_provider(Tool::Claude, current, "different-link".into())
        .is_err());
}
