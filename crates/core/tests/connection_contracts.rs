//! LeanFlip behavior contracts, built from client configuration interfaces.
mod common;
use common::Sandbox;
use leanflip_core::{store, switch, Provider, Tool};
use serde_json::{json, Value};

#[test]
fn switching_claude_connections_replaces_authentication_but_preserves_user_policy() {
    let sandbox = Sandbox::new();
    let personal = json!({"permissions":{"defaultMode":"plan"},"hooks":{"Stop":[]},"env":{"API_TIMEOUT_MS":"30000","CLAUDE_CODE_MAX_CONTEXT_TOKENS":"262144","AWS_ACCESS_KEY_ID":"fictional-old","CLAUDE_CODE_USE_BEDROCK":"1"},"apiKeyHelper":"old-helper"});
    sandbox.write(&sandbox.env.claude_settings_path(), &personal.to_string());
    sandbox.add(Tool::Claude,Provider::new("gateway","Gateway",json!({"env":{"ANTHROPIC_BASE_URL":"https://example.invalid","ANTHROPIC_AUTH_TOKEN":"fictional-next"}})));
    sandbox.add(
        Tool::Claude,
        Provider::new("official", "Official", json!({"env":{}})).official(),
    );
    for id in ["gateway", "official", "gateway"] {
        switch::switch(&sandbox.env, Tool::Claude, id).unwrap();
        let current = sandbox.json(&sandbox.env.claude_settings_path());
        assert_eq!(current["permissions"], personal["permissions"]);
        assert_eq!(current["hooks"], personal["hooks"]);
        assert_eq!(current["env"]["API_TIMEOUT_MS"], "30000");
        assert_eq!(current["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "262144");
        assert!(current.get("apiKeyHelper").is_none());
        assert!(current["env"].get("AWS_ACCESS_KEY_ID").is_none());
        assert_eq!(
            current["env"].get("ANTHROPIC_AUTH_TOKEN").is_some(),
            id == "gateway"
        );
    }
}

#[test]
fn codex_auth_sources_remain_separate_from_native_login() {
    for preserve in [false, true] {
        for credential in [
            "experimental_bearer_token = 'fictional-key'",
            "env_key = 'LEANFLIP_TEST_TOKEN'",
            "http_headers = { Authorization = 'Bearer fictional-header' }",
            "auth = { command = 'fictional-helper' }",
        ] {
            let sandbox = Sandbox::new();
            sandbox.set_preserve(preserve);
            let native = json!({"auth_mode":"chatgpt","tokens":{"access_token":"native-fixture","refresh_token":"native-refresh","id_token":leanflip_core::codex::auth::test_codex_id_token("local-user"),"account_id":"workspace-fixture"}});
            sandbox.write(&sandbox.env.codex_auth_path(), &native.to_string());
            sandbox.write(&sandbox.env.codex_config_path(),"# personal\napproval_policy = 'on-request'\n[mcp_servers.local]\ncommand = 'fixture'\n");
            sandbox.add(
                Tool::Codex,
                Provider::new("native", "Native", json!({"auth":{},"config":""})).official(),
            );
            sandbox.add(Tool::Codex,Provider::new("gateway","Gateway",json!({"auth":{},"config":format!("model_provider='test-route'\n[model_providers.test-route]\nbase_url='https://example.invalid/v1'\nwire_api='responses'\n{credential}\n")})));
            store::set_current(&sandbox.env, Tool::Codex, "native").unwrap();
            switch::switch(&sandbox.env, Tool::Codex, "gateway").unwrap();
            let config = sandbox.read(&sandbox.env.codex_config_path());
            let doc: toml::Value = config.parse().unwrap();
            assert!(!config.contains("native-fixture"));
            assert!(!config.contains("native-refresh"));
            assert_eq!(
                doc["mcp_servers"]["local"]["command"].as_str(),
                Some("fixture")
            );
            let route = &doc["model_providers"]["custom"];
            assert_eq!(
                route["base_url"].as_str(),
                Some("https://example.invalid/v1")
            );
            if credential.starts_with("auth") || credential.starts_with("http_headers") {
                assert_ne!(
                    route
                        .get("requires_openai_auth")
                        .and_then(toml::Value::as_bool),
                    Some(true)
                );
            }
            if preserve {
                assert_eq!(sandbox.json(&sandbox.env.codex_auth_path()), native);
            } else {
                assert!(!sandbox.env.codex_auth_path().exists());
            }
            switch::switch(&sandbox.env, Tool::Codex, "native").unwrap();
            assert_eq!(sandbox.json(&sandbox.env.codex_auth_path()), native);
        }
    }
}

#[test]
fn a_profile_override_or_invalid_later_file_cannot_publish_credentials() {
    for config in [
        "profile='work'\n[profiles.work]\nmodel_provider='different'\n",
        "model_providers = 9\n",
        "[broken\n",
    ] {
        let sandbox = Sandbox::new();
        sandbox.write(&sandbox.env.codex_config_path(), config);
        let original =
            "{\"auth_mode\":\"chatgpt\",\"tokens\":{\"refresh_token\":\"fictional-native\"}}";
        sandbox.write(&sandbox.env.codex_auth_path(), original);
        sandbox.add(Tool::Codex,Provider::new("gateway","Gateway",json!({"auth":{"OPENAI_API_KEY":"fictional-new"},"config":"model_provider='r'\n[model_providers.r]\nbase_url='https://example.invalid/v1'\n"})));
        assert!(switch::switch(&sandbox.env, Tool::Codex, "gateway").is_err());
        assert_eq!(sandbox.read(&sandbox.env.codex_config_path()), config);
        assert_eq!(sandbox.read(&sandbox.env.codex_auth_path()), original);
        assert!(sandbox.current(Tool::Codex).is_none());
    }
}

#[test]
fn gemini_does_not_accept_blank_credentials_or_select_key_auth_for_vertex() {
    for credential in [Value::String("".into()), Value::String("  ".into())] {
        let card = Provider::new(
            "empty",
            "Empty",
            json!({"env":{"GEMINI_API_KEY":credential,"GOOGLE_GEMINI_BASE_URL":"https://example.invalid"}}),
        );
        assert!(switch::gemini::projection(&card).is_err());
    }
    let card = Provider::new(
        "vertex",
        "Vertex",
        json!({"env":{"GOOGLE_GENAI_USE_VERTEXAI":"true","GOOGLE_CLOUD_PROJECT":"project-fixture"}}),
    );
    assert_eq!(
        switch::gemini::projection(&card).unwrap().selected_type,
        Some("vertex-ai")
    );
}

#[test]
fn listing_native_logins_does_not_create_a_profile_directory() {
    let sandbox = Sandbox::new();
    assert!(switch::codex::available_logins(&sandbox.env)
        .unwrap()
        .is_empty());
    assert!(!sandbox.env.data_dir.exists());
    std::fs::create_dir_all(&sandbox.env.data_dir).unwrap();
    assert!(switch::codex::available_logins(&sandbox.env)
        .unwrap()
        .is_empty());
    assert!(!sandbox.env.data_dir.join("providers.json.lock").exists());
}

#[test]
fn desktop_imports_foreign_profiles_without_adopting_their_identifier() {
    let sandbox = Sandbox::new();
    let id = "00000000-0000-4000-8000-000000157210";
    sandbox.write(
        &switch::claude_desktop::meta_path(&sandbox.env),
        &json!({"appliedId": id, "entries": [{"id": id, "name": "Previous tool"}]}).to_string(),
    );
    let old_path = sandbox
        .env
        .claude_desktop_library_dir()
        .join(format!("{id}.json"));
    let old = json!({"inferenceProvider":"gateway", "inferenceGatewayBaseUrl":"https://example.invalid", "inferenceGatewayApiKey":"fictional-token"}).to_string();
    sandbox.write(&old_path, &old);
    let imported = switch::claude_desktop::imported_card(&sandbox.env)
        .unwrap()
        .unwrap();
    assert_eq!(
        imported.settings_config["env"]["ANTHROPIC_BASE_URL"],
        "https://example.invalid"
    );
    sandbox.add(Tool::ClaudeDesktop, imported.clone());
    switch::switch(&sandbox.env, Tool::ClaudeDesktop, &imported.id).unwrap();
    assert_ne!(switch::claude_desktop::PROFILE_ID, id);
    assert_eq!(sandbox.read(&old_path), old);
    assert_eq!(
        sandbox.json(&switch::claude_desktop::meta_path(&sandbox.env))["appliedId"],
        switch::claude_desktop::PROFILE_ID
    );
}
