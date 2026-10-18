mod common;
use common::Sandbox;
use leanflip_core::{codex::login::*, import, store, switch, Provider, Tool};
use serde_json::json;

#[test]
fn unknown_leaving_login_keeps_unrelated_stashed_login() {
    let mut stash = LoginStash {
        initialized: true,
        ..Default::default()
    };
    stash.logins.insert(
        "other".into(),
        json!({"tokens":{"access_token":"fictional"}}),
    );
    stash.last = Some("other".into());
    let empty = json!({});
    let plan = plan(AuthInput {
        live: None,
        live_is_managed: false,
        third_party_keys: &[],
        leaving_official: Some(&empty),
        leaving_official_ref: None,
        official_api_keys: &[],
        target: AuthTarget::ThirdParty { preserve: false },
        stash: stash.clone(),
    });
    assert!(plan.stash.is_none() || plan.stash == Some(stash));
}

fn codex(id: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({"auth":{"OPENAI_API_KEY":"fictional-new-key"},"config":format!("model_provider='custom'\nmodel='fixture'\n[model_providers.custom]\nbase_url='https://{id}.invalid/v1'\nwire_api='responses'\n")}),
    )
}
#[test]
fn switching_codex_preserves_foreign_catalog_and_custom_table_without_copying_credentials() {
    let sb = Sandbox::new();
    sb.write(&sb.env.codex_config_path(),"model_catalog_json='/fixture/my-models.json'\n[model_providers.custom]\nbase_url='https://handwritten.invalid/v1'\nhttp_headers={Authorization='fictional-old-auth'}\nquery_params={account='original'}\n[profiles.saved]\nmodel_provider='custom'\n");
    sb.add(Tool::Codex, codex("a"));
    sb.add(Tool::Codex, codex("b"));
    for id in ["a", "b", "a"] {
        switch::switch(&sb.env, Tool::Codex, id).unwrap();
        let doc: toml::Value = sb.read(&sb.env.codex_config_path()).parse().unwrap();
        assert_eq!(
            doc["model_catalog_json"].as_str(),
            Some("/fixture/my-models.json")
        );
        assert_eq!(
            doc["model_providers"]["custom"]["base_url"].as_str(),
            Some(format!("https://{id}.invalid/v1").as_str())
        );
        assert!(doc["model_providers"]["custom"]
            .get("http_headers")
            .is_none());
        assert_eq!(
            doc["model_providers"]["leanflip-legacy-1"]["http_headers"]["Authorization"].as_str(),
            Some("fictional-old-auth")
        );
        assert_eq!(
            doc["profiles"]["saved"]["model_provider"].as_str(),
            Some("leanflip-legacy-1")
        );
        assert_eq!(doc["model_providers"].as_table().unwrap().len(), 2);
    }
}
#[test]
fn explicit_target_catalog_wins_over_foreign_live_catalog() {
    let sb = Sandbox::new();
    sb.write(
        &sb.env.codex_config_path(),
        "model_catalog_json='/fixture/live.json'\n",
    );
    let mut p = codex("a");
    p.settings_config["config"] = json!(format!(
        "model_catalog_json='/fixture/card.json'\n{}",
        p.settings_config["config"].as_str().unwrap()
    ));
    sb.add(Tool::Codex, p);
    switch::switch(&sb.env, Tool::Codex, "a").unwrap();
    let doc: toml::Value = sb.read(&sb.env.codex_config_path()).parse().unwrap();
    assert_eq!(
        doc["model_catalog_json"].as_str(),
        Some("/fixture/card.json")
    );
}
#[test]
fn importing_gemini_preserves_explicit_auth_without_dotenv() {
    for selected in ["vertex-ai", "gemini-api-key", "oauth-personal"] {
        let sb = Sandbox::new();
        sb.write(
            &sb.env.gemini_settings_path(),
            &json!({"security":{"auth":{"selectedType":selected}}}).to_string(),
        );
        import::import_live(&sb.env, Tool::Gemini).unwrap();
        switch::switch(&sb.env, Tool::Gemini, "official").unwrap();
        assert_eq!(
            sb.json(&sb.env.gemini_settings_path())["security"]["auth"]["selectedType"],
            selected
        );
    }
}
#[test]
fn opencode_partial_capabilities_import_and_invalid_entries_have_reasons() {
    let sb = Sandbox::new();
    sb.write(&sb.env.opencode_config_path(),&json!({"providers":{
        "good":{"settings":{"baseURL":"https://fixture.invalid/v1"},"models":{"m":{"capabilities":{"tools":false}}}},
        "bad":{"models":{"m":{"capabilities":{"tools":"wrong-type"}}}}
    }}).to_string());
    let result = import::import_live(&sb.env, Tool::OpenCode).unwrap();
    assert_eq!(result.created, vec!["good"]);
    assert_eq!(result.warnings.len(), 1);
    assert!(result.warnings[0].contains("bad"));
    assert!(result.warnings[0].contains("capabilities.tools"));
    assert_eq!(
        store::load_readonly(&sb.env)
            .unwrap()
            .providers(Tool::OpenCode)
            .len(),
        1
    );
}
#[test]
fn renamed_import_is_counted_as_import_and_cannot_enable_background_quota_requests() {
    use leanflip_core::ccswitch_export::*;
    let sb = Sandbox::new();
    sb.add(Tool::Claude,Provider::new("same","Original",json!({"env":{"ANTHROPIC_BASE_URL":"https://old.invalid","ANTHROPIC_AUTH_TOKEN":"fictional-old"}})));
    let config = json!({"providers":{"same":{"id":"same","name":"Incoming","settingsConfig":{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:1","ANTHROPIC_AUTH_TOKEN":"fictional-new"}},"meta":{"leanflipQuota":{"adapter":"newapi","refreshMinutes":5}}}}});
    let parsed = parse(&serde_json::to_vec(&config).unwrap()).unwrap();
    let preview = preview(&sb.env, &parsed, DuplicatePolicy::Rename).unwrap();
    assert_eq!(preview.items[0].status, ItemStatus::Import);
    let report = apply(&sb.env, &parsed, &[Tool::Claude], DuplicatePolicy::Rename).unwrap();
    assert_eq!(report.created.len(), 1);
    let data = store::load_readonly(&sb.env).unwrap();
    assert_eq!(
        data.provider(Tool::Claude, "same-2").unwrap().meta.extra["leanflipQuota"]
            ["refreshMinutes"],
        0
    );
}
