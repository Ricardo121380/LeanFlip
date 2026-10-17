mod common;
use common::Sandbox;
use leanflip_core::{command_trust, switch, Provider, Tool};
use serde_json::json;

#[test]
fn imported_claims_cannot_authorize_claude_helpers_and_changed_commands_require_review() {
    let sb = Sandbox::new();
    let mut card = Provider::new(
        "helper",
        "Helper",
        json!({"apiKeyHelper":"printf fixture-key", "env":{"ANTHROPIC_BASE_URL":"https://example.com"}}),
    );
    card.meta = serde_json::from_value(json!({"trusted":true,"commandsApproved":true})).unwrap();
    sb.add(Tool::Claude, card.clone());
    let error = switch::switch(&sb.env, Tool::Claude, &card.id).unwrap_err();
    assert!(error.to_string().contains("凭据命令"));
    assert!(!sb.env.claude_settings_path().exists());
    let preview = command_trust::preview(&sb.env, Tool::Claude, &card.settings_config).unwrap();
    assert!(!preview.approved);
    assert!(
        command_trust::approve(&sb.env, Tool::Claude, &card.settings_config, "unreviewed").is_err()
    );
    command_trust::approve(
        &sb.env,
        Tool::Claude,
        &card.settings_config,
        &preview.fingerprint,
    )
    .unwrap();
    switch::switch(&sb.env, Tool::Claude, &card.id).unwrap();
    assert_eq!(
        sb.json(&sb.env.claude_settings_path())["apiKeyHelper"],
        "printf fixture-key"
    );
    let mut changed_environment = card.settings_config.clone();
    changed_environment["env"]["PATH"] = json!("/untrusted/bin");
    assert!(
        !command_trust::preview(&sb.env, Tool::Claude, &changed_environment)
            .unwrap()
            .approved
    );
    card.settings_config["apiKeyHelper"] = json!("printf changed");
    assert!(
        !command_trust::preview(&sb.env, Tool::Claude, &card.settings_config)
            .unwrap()
            .approved
    );
    let other = Sandbox::new();
    assert!(!command_trust::preview(&other.env, Tool::Claude, &json!({"apiKeyHelper":"printf fixture-key", "env":{"ANTHROPIC_BASE_URL":"https://example.com"}})).unwrap().approved);
}

#[test]
fn codex_command_arguments_and_working_directory_are_part_of_consent() {
    let sb = Sandbox::new();
    let config = |arg: &str, cwd: &str| json!({"auth":{},"config":format!("model_provider = \"relay\"\n[model_providers.relay]\nname=\"relay\"\nbase_url=\"https://example.com\"\n[model_providers.relay.auth]\ncommand=\"credential-helper\"\nargs=[\"{arg}\"]\ncwd=\"{cwd}\"\n")});
    let settings = config("get", "/tmp");
    let card = Provider::new("helper", "Helper", settings.clone());
    sb.add(Tool::Codex, card.clone());
    assert!(switch::switch(&sb.env, Tool::Codex, &card.id).is_err());
    assert!(!sb.env.codex_config_path().exists());
    let review = command_trust::preview(&sb.env, Tool::Codex, &settings).unwrap();
    assert_eq!(
        review.commands["model_providers.relay.auth"]["args"],
        json!(["get"])
    );
    command_trust::approve(&sb.env, Tool::Codex, &settings, &review.fingerprint).unwrap();
    switch::switch(&sb.env, Tool::Codex, &card.id).unwrap();
    for changed in [config("delete", "/tmp"), config("get", "/elsewhere")] {
        assert!(
            !command_trust::preview(&sb.env, Tool::Codex, &changed)
                .unwrap()
                .approved
        );
        assert!(
            command_trust::approve(&sb.env, Tool::Codex, &changed, &review.fingerprint).is_err()
        );
    }
    sb.write(
        &sb.env.data_file("trusted-credential-commands.json"),
        "{broken",
    );
    assert!(command_trust::preview(&sb.env, Tool::Codex, &settings).is_err());
}

#[test]
fn claude_onboarding_is_explicit_preserves_preferences_and_backs_up_original() {
    let sb = Sandbox::new();
    let path = sb.home().join(".claude.json");
    let original =
        r#"{"hasCompletedOnboarding":false,"projects":{"/work":{"trusted":false}},"theme":"dark"}"#;
    sb.write(&path, original);
    sb.add(
        Tool::Claude,
        Provider::new(
            "relay",
            "Relay",
            json!({"env":{"ANTHROPIC_AUTH_TOKEN":"fixture"}}),
        ),
    );
    switch::switch(&sb.env, Tool::Claude, "relay").unwrap();
    assert_eq!(
        sb.read(&path),
        original,
        "switching must never skip onboarding implicitly"
    );
    switch::claude::complete_onboarding(&sb.env).unwrap();
    assert_eq!(
        sb.json(&path),
        json!({"hasCompletedOnboarding":true,"projects":{"/work":{"trusted":false}},"theme":"dark"})
    );
    assert_eq!(sb.read(&sb.first_write_backup(&path)), original);
    assert!(switch::claude::complete_onboarding(&sb.env)
        .unwrap()
        .changed
        .is_empty());
    sb.write(&path, "{malformed");
    assert!(switch::claude::complete_onboarding(&sb.env).is_err());
    assert_eq!(sb.read(&path), "{malformed");
}
