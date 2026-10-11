//! Claude Desktop 直连模式：一次切换四个文件（普通配置、3p 配置、profile、`_meta.json`），
//! 全部在临时 HOME 里。

mod common;

use ccsl_core::import::import_live;
use ccsl_core::switch::{self, claude_desktop};
use ccsl_core::{Provider, Tool};
use common::Sandbox;
use serde_json::{json, Value};

const NORMAL: &str = r#"{
  "mcpServers": {"fs": {"command": "npx", "args": ["fs"]}},
  "preferences": {"menuBarEnabled": true}
}"#;

const THREEP: &str = r#"{
  "deploymentMode": "1p",
  "enterpriseConfig": {"inferenceGatewayApiKey": "old-legacy", "inferenceProvider": "gateway", "orgPolicy": "keep"}
}"#;

const META: &str =
    r#"{"appliedId": "user-profile", "entries": [{"id": "user-profile", "name": "Mine"}]}"#;

fn direct(id: &str, url: &str, key: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({"env": {"ANTHROPIC_BASE_URL": url, "ANTHROPIC_AUTH_TOKEN": key}}),
    )
}

fn with_meta(mut provider: Provider, key: &str, value: Value) -> Provider {
    provider.meta.extra.insert(key.into(), value);
    provider
}

fn sandbox() -> Sandbox {
    let sb = Sandbox::new();
    assert!(sb.env.claude_desktop_root.starts_with(sb.home()));
    sb.write(&sb.env.claude_desktop_config_path(), NORMAL);
    sb.write(&sb.env.claude_desktop_3p_config_path(), THREEP);
    sb.write(&claude_desktop::meta_path(&sb.env), META);
    sb.add(
        Tool::ClaudeDesktop,
        with_meta(
            direct("a", "https://a.example", "ka"),
            "claudeDesktopModelRoutes",
            json!({
                "claude-sonnet-4-5": {"model": "", "labelOverride": "Sonnet A"},
                "claude-opus-4-1": {"supports1m": true}
            }),
        ),
    );
    sb.add(Tool::ClaudeDesktop, direct("b", "https://b.example", "kb"));
    sb.add(
        Tool::ClaudeDesktop,
        Provider::new("official", "Claude", json!({"env": {}})).official(),
    );
    sb
}

#[test]
fn direct_switch_writes_four_files_in_one_operation() {
    let sb = sandbox();
    let report = switch::switch(&sb.env, Tool::ClaudeDesktop, "a").unwrap();
    assert_eq!(report.changed.len(), 4, "{:?}", report.changed);

    let normal = sb.json(&sb.env.claude_desktop_config_path());
    assert_eq!(normal["deploymentMode"], "3p");
    assert_eq!(normal["mcpServers"]["fs"]["command"], "npx");
    assert_eq!(normal["preferences"]["menuBarEnabled"], true);
    assert_eq!(
        sb.json(&sb.env.claude_desktop_3p_config_path())["deploymentMode"],
        "3p"
    );

    let profile_path = claude_desktop::profile_path(&sb.env);
    let profile = sb.json(&profile_path);
    assert_eq!(profile["inferenceProvider"], "gateway");
    assert_eq!(profile["inferenceGatewayBaseUrl"], "https://a.example");
    assert_eq!(profile["inferenceGatewayApiKey"], "ka");
    assert_eq!(profile["inferenceGatewayAuthScheme"], "bearer");
    assert_eq!(profile["disableDeploymentModeChooser"], true);
    assert_eq!(profile["coworkEgressAllowedHosts"], json!(["*"]));
    // 排序后的模型列表；带标签 / 1M 的写成对象。
    assert_eq!(
        profile["inferenceModels"],
        json!([
            {"name": "claude-opus-4-1", "supports1m": true},
            {"name": "claude-sonnet-4-5", "labelOverride": "Sonnet A"}
        ])
    );
    #[cfg(unix)]
    assert_eq!(common::mode(&profile_path), 0o600);

    let meta = sb.json(&claude_desktop::meta_path(&sb.env));
    assert_eq!(meta["appliedId"], claude_desktop::PROFILE_ID);
    let entries = meta["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0]["id"], "user-profile",
        "user entry keeps its place"
    );
    assert_eq!(entries[1]["name"], claude_desktop::PROFILE_NAME);
    assert_eq!(sb.current(Tool::ClaudeDesktop).as_deref(), Some("a"));
}

#[test]
fn switching_between_direct_cards_replaces_the_gateway_but_keeps_user_profile_edits() {
    let sb = sandbox();
    switch::switch(&sb.env, Tool::ClaudeDesktop, "a").unwrap();
    // 用户在 Desktop 里改了 profile 的策略键和别的设置。
    let profile_path = claude_desktop::profile_path(&sb.env);
    let mut profile = sb.json(&profile_path);
    profile["coworkEgressAllowedHosts"] = json!(["api.example"]);
    profile["userTheme"] = json!("dark");
    sb.write(
        &profile_path,
        &serde_json::to_string_pretty(&profile).unwrap(),
    );

    switch::switch(&sb.env, Tool::ClaudeDesktop, "b").unwrap();
    let profile = sb.json(&profile_path);
    assert_eq!(profile["inferenceGatewayBaseUrl"], "https://b.example");
    assert_eq!(profile["inferenceGatewayApiKey"], "kb");
    assert!(profile.get("inferenceModels").is_none(), "{profile}");
    assert_eq!(profile["coworkEgressAllowedHosts"], json!(["api.example"]));
    assert_eq!(profile["userTheme"], "dark");
    let meta = sb.json(&claude_desktop::meta_path(&sb.env));
    assert_eq!(
        meta["entries"].as_array().unwrap().len(),
        2,
        "no duplicate entry"
    );
}

#[test]
fn official_restores_1p_and_scrubs_the_key_from_disk() {
    let sb = sandbox();
    switch::switch(&sb.env, Tool::ClaudeDesktop, "a").unwrap();
    let report = switch::switch(&sb.env, Tool::ClaudeDesktop, "official").unwrap();
    assert_eq!(report.changed.len(), 4);

    let normal = sb.json(&sb.env.claude_desktop_config_path());
    assert_eq!(normal["deploymentMode"], "1p");
    assert_eq!(normal["mcpServers"]["fs"]["command"], "npx");
    let threep = sb.json(&sb.env.claude_desktop_3p_config_path());
    assert_eq!(threep["deploymentMode"], "1p");
    assert_eq!(
        threep["enterpriseConfig"],
        json!({"orgPolicy": "keep"}),
        "legacy gateway keys removed, other enterprise keys kept"
    );
    let profile = sb.read(&claude_desktop::profile_path(&sb.env));
    assert!(!profile.contains("ka"), "{profile}");
    assert!(!profile.contains("inferenceGatewayBaseUrl"), "{profile}");
    let meta = sb.json(&claude_desktop::meta_path(&sb.env));
    assert_eq!(meta["appliedId"], "user-profile");
    assert_eq!(
        meta["entries"],
        json!([{"id": "user-profile", "name": "Mine"}])
    );
}

#[test]
fn official_on_a_fresh_install_creates_no_profile_and_drops_empty_enterprise_config() {
    let sb = Sandbox::new();
    sb.write(
        &sb.env.claude_desktop_3p_config_path(),
        r#"{"enterpriseConfig": {"inferenceProvider": "gateway"}}"#,
    );
    sb.add(
        Tool::ClaudeDesktop,
        Provider::new("official", "Claude", json!({"env": {}})).official(),
    );
    switch::switch(&sb.env, Tool::ClaudeDesktop, "official").unwrap();
    assert!(!claude_desktop::profile_path(&sb.env).exists());
    let threep = sb.json(&sb.env.claude_desktop_3p_config_path());
    assert_eq!(threep, json!({"deploymentMode": "1p"}));
    let meta = sb.json(&claude_desktop::meta_path(&sb.env));
    assert!(meta.get("appliedId").is_none(), "{meta}");
}

#[test]
fn unsupported_cards_are_refused_before_any_write() {
    let sb = sandbox();
    let cases = [
        with_meta(
            direct("p", "https://p", "k"),
            "claudeDesktopMode",
            json!("proxy"),
        ),
        with_meta(
            direct("f", "https://f", "k"),
            "apiFormat",
            json!("openai_chat"),
        ),
        with_meta(
            direct("t", "https://t", "k"),
            "providerType",
            json!("codex_oauth"),
        ),
        with_meta(direct("u", "https://u", "k"), "isFullUrl", json!(true)),
        with_meta(
            direct("m", "https://m", "k"),
            "claudeDesktopModelRoutes",
            json!({"claude-sonnet-4-5": {"model": "gpt-5"}}),
        ),
        with_meta(
            direct("n", "https://n", "k"),
            "claudeDesktopModelRoutes",
            json!({"gpt-5": {}}),
        ),
        Provider::new(
            "nokey",
            "nokey",
            json!({"env": {"ANTHROPIC_BASE_URL": "https://x"}}),
        ),
        Provider::new(
            "nourl",
            "nourl",
            json!({"env": {"ANTHROPIC_AUTH_TOKEN": "k"}}),
        ),
    ];
    for card in cases {
        let id = card.id.clone();
        sb.add(Tool::ClaudeDesktop, card);
        assert!(
            switch::switch(&sb.env, Tool::ClaudeDesktop, &id).is_err(),
            "{id} should be refused"
        );
        assert_eq!(
            sb.read(&sb.env.claude_desktop_config_path()),
            NORMAL,
            "{id}"
        );
        assert!(!claude_desktop::profile_path(&sb.env).exists(), "{id}");
    }
}

#[test]
fn a_corrupt_file_aborts_the_whole_operation() {
    let sb = sandbox();
    sb.write(&claude_desktop::meta_path(&sb.env), "{not json");
    assert!(switch::switch(&sb.env, Tool::ClaudeDesktop, "a").is_err());
    assert_eq!(sb.read(&sb.env.claude_desktop_config_path()), NORMAL);
    assert_eq!(sb.read(&sb.env.claude_desktop_3p_config_path()), THREEP);
    assert!(!claude_desktop::profile_path(&sb.env).exists());
    assert_eq!(sb.current(Tool::ClaudeDesktop), None);
    assert!(sb.temp_files().is_empty(), "{:?}", sb.temp_files());
}

#[test]
fn model_ids_follow_the_desktop_menu_contract() {
    for ok in [
        "claude-sonnet-4-5",
        "anthropic/claude-opus-4-1",
        "claude-haiku-3",
        "CLAUDE-FABLE-x",
    ] {
        assert!(claude_desktop::is_claude_safe_model_id(ok), "{ok}");
    }
    for bad in [
        "claude-sonnet-",
        "claude-3-5-sonnet",
        "claude-sonnet-4[1m]",
        "gpt-5",
        "anthropic/gpt",
        "",
    ] {
        assert!(!claude_desktop::is_claude_safe_model_id(bad), "{bad}");
    }
}

#[test]
fn import_reads_the_selected_gateway_profile() {
    let sb = Sandbox::new();
    sb.write(
        &claude_desktop::meta_path(&sb.env),
        &json!({"appliedId": claude_desktop::PROFILE_ID,
                "entries": [{"id": claude_desktop::PROFILE_ID, "name": "CC Switch"}]})
        .to_string(),
    );
    sb.write(
        &claude_desktop::profile_path(&sb.env),
        &json!({"inferenceGatewayBaseUrl": "https://gw.example", "inferenceGatewayApiKey": "kg",
                "inferenceProvider": "gateway",
                "inferenceModels": ["claude-sonnet-4-5", {"name": "claude-opus-4-1", "supports1m": true}, "bogus"]})
        .to_string(),
    );
    import_live(&sb.env, Tool::ClaudeDesktop).unwrap();
    assert_eq!(sb.current(Tool::ClaudeDesktop).as_deref(), Some("default"));
    // 原样再写一次：profile 不变（只是格式化），模型列表保留可用的两项。
    switch::switch(&sb.env, Tool::ClaudeDesktop, "default").unwrap();
    let profile = sb.json(&claude_desktop::profile_path(&sb.env));
    assert_eq!(profile["inferenceGatewayApiKey"], "kg");
    assert_eq!(
        profile["inferenceModels"],
        json!([{"name": "claude-opus-4-1", "supports1m": true}, "claude-sonnet-4-5"])
    );
    switch::switch(&sb.env, Tool::ClaudeDesktop, "official").unwrap();
}

#[test]
fn import_without_our_profile_only_creates_the_official_card() {
    let sb = Sandbox::new();
    sb.write(&claude_desktop::meta_path(&sb.env), META);
    import_live(&sb.env, Tool::ClaudeDesktop).unwrap();
    assert_eq!(sb.current(Tool::ClaudeDesktop).as_deref(), Some("official"));
}
