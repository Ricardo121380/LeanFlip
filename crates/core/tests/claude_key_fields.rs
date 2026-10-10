//! Claude Code 关键字段的切换结果（移植自 cc-switch `tests/golden/claude_key_fields.rs`）。
//!
//! 切换后 live 里的关键字段必须恰好等于目标供应商行里的，上一家的一个都不能留。关键
//! 字段的定义直接引用 `live::floor`，和切换用的是同一份。通用配置片段（common config）
//! 在 ccswitch-lite 里没有，相关的两条未移植。

mod common;

use std::collections::BTreeMap;

use ccsl_core::live::floor::{claude_floor_env, claude_floor_top};
use ccsl_core::{store, switch, Provider, Tool};
use common::Sandbox;
use serde_json::{json, Value};

fn floor_view(settings: &Value) -> BTreeMap<String, Value> {
    let mut view = BTreeMap::new();
    if let Some(env) = settings.get("env").and_then(Value::as_object) {
        for (key, value) in env {
            if claude_floor_env(key) {
                view.insert(format!("env.{key}"), value.clone());
            }
        }
    }
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            if claude_floor_top(key) {
                view.insert(key.clone(), value.clone());
            }
        }
    }
    view
}

fn setup(providers: &[Provider], current: &str) -> Sandbox {
    let sb = Sandbox::new();
    for provider in providers {
        sb.add(Tool::Claude, provider.clone());
    }
    store::set_current(&sb.env, Tool::Claude, current).unwrap();
    let row = providers.iter().find(|p| p.id == current).unwrap();
    sb.write(
        &sb.env.claude_dir.join("settings.json"),
        &serde_json::to_string_pretty(&row.settings_config).unwrap(),
    );
    sb
}

fn live(sb: &Sandbox) -> Value {
    sb.json(&sb.env.claude_settings_path())
}

fn switch_and_check(sb: &Sandbox, target: &str) {
    let row = store::load(&sb.env)
        .unwrap()
        .provider(Tool::Claude, target)
        .unwrap()
        .clone();
    switch::switch(&sb.env, Tool::Claude, target).expect("switch provider");
    let live = live(sb);
    assert_eq!(
        floor_view(&live),
        floor_view(&row.settings_config),
        "after switching to {target}, live key fields must be exactly the target's\nlive: {live:#}"
    );
}

fn relay(id: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({
            "env": {
                "ANTHROPIC_BASE_URL": format!("https://{id}.example"),
                "ANTHROPIC_AUTH_TOKEN": format!("sk-{id}"),
                "ANTHROPIC_MODEL": format!("{id}-model"),
                "ANTHROPIC_DEFAULT_SONNET_MODEL": format!("{id}-sonnet"),
                "CLAUDE_CODE_SUBAGENT_MODEL": format!("{id}-subagent")
            }
        }),
    )
}

fn official() -> Provider {
    Provider::new("claude-official", "Official", json!({ "env": {} })).official()
}

#[test]
fn third_party_to_third_party_replaces_every_key_field() {
    let b = Provider::new(
        "b",
        "b",
        json!({ "env": { "ANTHROPIC_BASE_URL": "https://b.example", "ANTHROPIC_API_KEY": "sk-b" } }),
    );
    let sb = setup(&[relay("a"), b], "a");
    switch_and_check(&sb, "b");
    switch_and_check(&sb, "a");
}

/// `/model` 写在 live 顶层的 `model` 属于切走前的那家，不能带给下一家。
#[test]
fn model_picked_in_client_does_not_follow_the_switch() {
    let sb = setup(&[relay("a"), relay("b")], "a");
    let mut value = live(&sb);
    value["model"] = json!("a-picked-model");
    sb.write(&sb.env.claude_settings_path(), &value.to_string());
    switch_and_check(&sb, "b");
    assert!(live(&sb).get("model").is_none());
}

#[test]
fn bedrock_to_official_leaves_no_provider_keys() {
    let bedrock = Provider::new(
        "bedrock",
        "bedrock",
        json!({
            "env": {
                "CLAUDE_CODE_USE_BEDROCK": "1",
                "AWS_REGION": "us-west-2",
                "AWS_BEARER_TOKEN_BEDROCK": "bedrock-key",
                "ANTHROPIC_MODEL": "us.anthropic.claude-sonnet-4-5-v1:0"
            },
            "apiKey": "legacy-bedrock-key"
        }),
    );
    let sb = setup(&[bedrock, official()], "bedrock");
    switch_and_check(&sb, "claude-official");
    let value = live(&sb);
    assert!(floor_view(&value).is_empty(), "official live: {value:#}");

    switch::switch(&sb.env, Tool::Claude, "bedrock").unwrap();
    let value = live(&sb);
    assert_eq!(
        value["env"]["AWS_BEARER_TOKEN_BEDROCK"],
        json!("bedrock-key")
    );
    assert!(value.get("apiKey").is_none(), "live: {value:#}");
}

#[test]
fn legacy_bedrock_api_key_row_is_projected_to_the_bearer_env() {
    let legacy = Provider::new(
        "bedrock",
        "bedrock",
        json!({
            "env": { "CLAUDE_CODE_USE_BEDROCK": "1", "AWS_REGION": "us-west-2" },
            "apiKey": "legacy-bedrock-key"
        }),
    );
    let sb = setup(&[relay("a"), legacy.clone()], "a");
    switch::switch(&sb.env, Tool::Claude, "bedrock").unwrap();
    let value = live(&sb);
    assert_eq!(
        value["env"]["AWS_BEARER_TOKEN_BEDROCK"],
        json!("legacy-bedrock-key")
    );
    assert!(value.get("apiKey").is_none(), "live: {value:#}");
    let row = store::load(&sb.env)
        .unwrap()
        .provider(Tool::Claude, "bedrock")
        .unwrap()
        .settings_config
        .clone();
    assert_eq!(
        row, legacy.settings_config,
        "the row itself is not rewritten"
    );
}

#[test]
fn vertex_to_third_party_leaves_no_vertex_keys() {
    let vertex = Provider::new(
        "vertex",
        "vertex",
        json!({ "env": {
            "CLAUDE_CODE_USE_VERTEX": "1",
            "CLOUD_ML_REGION": "us-east5",
            "ANTHROPIC_VERTEX_PROJECT_ID": "my-project",
            "GOOGLE_APPLICATION_CREDENTIALS": "/Users/me/gcp.json",
            "VERTEX_REGION_CLAUDE_4_0_OPUS": "europe-west1"
        } }),
    );
    let sb = setup(&[vertex, relay("b")], "vertex");
    switch_and_check(&sb, "b");
    switch_and_check(&sb, "vertex");
}

#[test]
fn api_key_helper_does_not_outlive_its_provider() {
    let helper = Provider::new(
        "helper",
        "helper",
        json!({
            "apiKeyHelper": "~/bin/print-key.sh",
            "env": {
                "ANTHROPIC_BASE_URL": "https://helper.example",
                "CLAUDE_CODE_API_KEY_HELPER_TTL_MS": "600000"
            }
        }),
    );
    let sb = setup(&[helper, relay("b")], "helper");
    switch_and_check(&sb, "b");
    switch_and_check(&sb, "helper");
}

#[test]
fn official_roundtrip_through_third_party() {
    let sb = setup(&[official(), relay("b")], "claude-official");
    switch_and_check(&sb, "b");
    switch_and_check(&sb, "claude-official");
    assert!(floor_view(&live(&sb)).is_empty());
}

/// live 不是合法 JSON：拒绝写入，原文件一个字节都不动，指针不动。
#[test]
fn unparsable_live_is_never_overwritten() {
    let sb = setup(&[relay("a"), relay("b")], "a");
    let path = sb.env.claude_settings_path();
    sb.write(&path, "{ \"env\": { broken");
    assert!(switch::switch(&sb.env, Tool::Claude, "b").is_err());
    assert_eq!(sb.read(&path), "{ \"env\": { broken");
    assert_eq!(sb.current(Tool::Claude).as_deref(), Some("a"));
    assert!(sb.temp_files().is_empty());
}

/// 客户端文件还不存在：切换时新建（0600）。
#[test]
fn a_missing_settings_file_is_created() {
    let sb = Sandbox::new();
    sb.add(Tool::Claude, relay("a"));
    switch::switch(&sb.env, Tool::Claude, "a").unwrap();
    let value = live(&sb);
    assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://a.example");
    #[cfg(unix)]
    assert_eq!(common::mode(&sb.env.claude_settings_path()), 0o600);
}
