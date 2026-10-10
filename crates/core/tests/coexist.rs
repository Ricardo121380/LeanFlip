//! 并存式工具（OpenCode、OpenClaw、Hermes、Pi、MCode）：条目级写入 / 移出、
//! 格式保留、冲突保护、导入，全部在临时 HOME 里。

mod common;

use ccsl_core::coexist;
use ccsl_core::import::import_live;
use ccsl_core::switch;
use ccsl_core::{store, Provider, Tool};
use common::Sandbox;
use serde_json::{json, Value};

fn card(id: &str, settings: Value) -> Provider {
    Provider::new(id, id, settings)
}

fn ids(sb: &Sandbox, tool: Tool) -> Vec<String> {
    coexist::live_entries(&sb.env, tool)
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

// ---------------------------------------------------------------- OpenCode

const OPENCODE: &str = r#"{
  // my OpenCode config
  "$schema": "https://opencode.ai/config.json",
  "theme": "tokyonight", /* keep */
  "provider": {
    "mine": {"npm": "@ai-sdk/openai-compatible", "options": {"baseURL": "https://mine"}, "models": {"m": {}}},
  },
}
"#;

fn opencode_v1(url: &str) -> Value {
    json!({"npm": "@ai-sdk/openai-compatible", "name": "Relay",
           "options": {"baseURL": url, "apiKey": "sk"}, "models": {"gpt-5": {}}})
}

#[test]
fn opencode_add_and_remove_keep_comments_and_other_entries() {
    let sb = Sandbox::new();
    let path = sb.env.opencode_dir.join("opencode.jsonc");
    sb.write(&path, OPENCODE);
    assert_eq!(sb.env.opencode_config_path(), path, "jsonc preferred");

    coexist::add_provider(
        &sb.env,
        Tool::OpenCode,
        card("relay", opencode_v1("https://r")),
        true,
    )
    .unwrap()
    .expect("written");
    let text = sb.read(&path);
    assert!(text.contains("// my OpenCode config"), "{text}");
    assert!(text.contains("/* keep */"), "{text}");
    assert_eq!(ids(&sb, Tool::OpenCode), ["mine", "relay"]);
    assert_eq!(
        sb.current(Tool::OpenCode),
        None,
        "coexist tools have no pointer"
    );

    coexist::remove_from_live(&sb.env, Tool::OpenCode, "relay").unwrap();
    assert_eq!(
        sb.read(&path),
        OPENCODE,
        "removing restores the original bytes"
    );
    // 卡还在，可以再写回去。
    assert!(store::load(&sb.env)
        .unwrap()
        .provider(Tool::OpenCode, "relay")
        .is_some());
    switch::switch(&sb.env, Tool::OpenCode, "relay").unwrap();
    assert_eq!(ids(&sb, Tool::OpenCode), ["mine", "relay"]);
    // 删卡同时移出。
    coexist::delete_provider(&sb.env, Tool::OpenCode, "relay").unwrap();
    assert_eq!(ids(&sb, Tool::OpenCode), ["mine"]);
}

#[test]
fn opencode_native_v2_cards_go_to_providers_and_invalid_ones_are_refused() {
    let sb = Sandbox::new();
    let mut v2 = card(
        "native",
        json!({"package": "@ai-sdk/anthropic", "settings": {"timeout": 1000}}),
    );
    v2.meta
        .extra
        .insert("opencodeConfigFormat".into(), json!("v2"));
    coexist::add_provider(&sb.env, Tool::OpenCode, v2, true).unwrap();
    let live = sb.json(&sb.env.opencode_config_path());
    assert_eq!(live["$schema"], "https://opencode.ai/config.json");
    assert_eq!(live["providers"]["native"]["package"], "@ai-sdk/anthropic");
    assert!(live.get("provider").is_none());

    let bad = card(
        "bad",
        json!({"package": "x", "settings": {"transport": "carrier-pigeon"}}),
    );
    let err = coexist::add_provider(&sb.env, Tool::OpenCode, bad, true).unwrap_err();
    assert!(err.to_string().contains("settings"), "{err}");
    // 新 id 没有 npm / 模型不能沿用内置定义。
    let bare = card("bare", json!({"options": {"apiKey": "k"}}));
    assert!(coexist::add_provider(&sb.env, Tool::OpenCode, bare, true).is_err());
    assert!(store::load(&sb.env)
        .unwrap()
        .provider(Tool::OpenCode, "bare")
        .is_none());
}

#[test]
fn opencode_import_reads_v1_and_valid_v2_and_is_repeatable() {
    let sb = Sandbox::new();
    sb.write(
        &sb.env.opencode_config_path(),
        r#"{"provider": {"a": {"npm": "x", "name": "Alpha", "models": {"m": {}}}},
            "providers": {"b": {"package": "y"}, "broken": {"settings": 3}}}"#,
    );
    let report = import_live(&sb.env, Tool::OpenCode).unwrap();
    assert_eq!(report.created, ["a", "b"]);
    let data = store::load(&sb.env).unwrap();
    assert_eq!(data.provider(Tool::OpenCode, "a").unwrap().name, "Alpha");
    assert_eq!(
        data.provider(Tool::OpenCode, "b").unwrap().meta.extra["opencodeConfigFormat"],
        "v2"
    );
    assert!(import_live(&sb.env, Tool::OpenCode)
        .unwrap()
        .created
        .is_empty());
    // 编辑导入的 V2 卡写回 providers（不会写进 provider 段）。
    let mut b = data.provider(Tool::OpenCode, "b").unwrap().clone();
    b.settings_config = json!({"package": "z"});
    switch::update_provider(&sb.env, Tool::OpenCode, b)
        .unwrap()
        .expect("in live");
    let live = sb.json(&sb.env.opencode_config_path());
    assert_eq!(live["providers"]["b"]["package"], "z");
    assert_eq!(
        live["providers"]["broken"]["settings"], 3,
        "invalid native entry untouched"
    );
}

#[test]
fn adding_a_card_never_overwrites_a_different_live_entry() {
    let sb = Sandbox::new();
    sb.write(&sb.env.opencode_config_path(), OPENCODE);
    let err = coexist::add_provider(
        &sb.env,
        Tool::OpenCode,
        card("mine", opencode_v1("https://other")),
        true,
    )
    .unwrap_err();
    assert!(err.to_string().contains("mine"), "{err}");
    assert_eq!(sb.read(&sb.env.opencode_config_path()), OPENCODE);
    assert!(store::load(&sb.env)
        .unwrap()
        .providers(Tool::OpenCode)
        .is_empty());
    // 不写 live 的加卡照样可以（之后显式写入 = 用户选择覆盖）。
    coexist::add_provider(
        &sb.env,
        Tool::OpenCode,
        card("mine", opencode_v1("https://other")),
        false,
    )
    .unwrap();
    assert_eq!(sb.read(&sb.env.opencode_config_path()), OPENCODE);
}

// ---------------------------------------------------------------- OpenClaw

#[test]
fn openclaw_json5_edits_are_node_level() {
    let sb = Sandbox::new();
    let path = sb.env.openclaw_config_path();
    let original = "// OpenClaw\n{\n  agents: { defaults: { model: 'x/y' } },\n  models: {\n    mode: 'merge',\n    providers: {\n      keep: { baseUrl: 'https://keep', api: 'openai-completions' },\n    },\n  },\n}\n";
    sb.write(&path, original);
    coexist::add_provider(
        &sb.env,
        Tool::OpenClaw,
        card("relay", json!({"baseUrl": "https://relay/v1", "apiKey": "k", "api": "openai-completions", "models": [{"id": "m"}]})),
        true,
    )
    .unwrap();
    let text = sb.read(&path);
    assert!(
        text.starts_with("// OpenClaw\n{\n  agents: { defaults: { model: 'x/y' } },"),
        "{text}"
    );
    assert!(
        text.contains("keep: { baseUrl: 'https://keep', api: 'openai-completions' },"),
        "{text}"
    );
    assert_eq!(ids(&sb, Tool::OpenClaw), ["keep", "relay"]);
    coexist::remove_from_live(&sb.env, Tool::OpenClaw, "relay").unwrap();
    assert_eq!(sb.read(&path), original);

    let invalid = card("x", json!({"foo": 1}));
    assert!(coexist::add_provider(&sb.env, Tool::OpenClaw, invalid, true).is_err());
}

#[test]
fn openclaw_creates_a_default_file() {
    let sb = Sandbox::new();
    sb.add(Tool::OpenClaw, card("a", json!({"baseUrl": "https://a"})));
    coexist::write_to_live(&sb.env, Tool::OpenClaw, "a").unwrap();
    let live: Value = json5::from_str(&sb.read(&sb.env.openclaw_config_path())).unwrap();
    assert_eq!(live["models"]["mode"], "merge");
    assert_eq!(live["models"]["providers"]["a"]["baseUrl"], "https://a");
}

// ---------------------------------------------------------------- Hermes

const HERMES: &str = "# Hermes config\nmodel:\n  default: old-model\n  provider: openrouter\n  context_length: 200000 # mine\n\nterminal:\n  backend: local   # keep this comment\n\ncustom_providers:\n- name: existing\n  base_url: https://existing\n  request_timeout_seconds: 30\n\nproviders:\n  dict-only:\n    base_url: https://dict\n";

fn hermes_card(id: &str) -> Provider {
    card(
        id,
        json!({"baseUrl": "https://relay/v1", "api_key": "k", "api": "openai-completions",
               "models": [{"id": "m-1", "context_length": 128000}, {"id": "m-2"}]}),
    )
}

#[test]
fn hermes_only_rewrites_its_sections_and_activate_points_model_at_the_provider() {
    let sb = Sandbox::new();
    let path = sb.env.hermes_config_path();
    sb.write(&path, HERMES);
    sb.add(Tool::Hermes, hermes_card("relay"));
    switch::switch(&sb.env, Tool::Hermes, "relay").unwrap();
    let text = sb.read(&path);
    assert!(text.starts_with("# Hermes config\n"), "{text}");
    assert!(
        text.contains("terminal:\n  backend: local   # keep this comment\n"),
        "{text}"
    );
    assert!(
        text.contains("providers:\n  dict-only:\n    base_url: https://dict\n"),
        "{text}"
    );
    let live: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    let relay = &live["custom_providers"][1];
    assert_eq!(relay["name"].as_str(), Some("relay"));
    assert_eq!(
        relay["base_url"].as_str(),
        Some("https://relay/v1"),
        "camelCase sanitised"
    );
    assert!(relay.get("baseUrl").is_none() && relay.get("api").is_none());
    assert_eq!(relay["model"].as_str(), Some("m-1"));
    assert_eq!(
        relay["models"]["m-1"]["context_length"].as_u64(),
        Some(128000)
    );
    assert_eq!(live["model"]["provider"].as_str(), Some("relay"));
    assert_eq!(live["model"]["default"].as_str(), Some("m-1"));
    assert_eq!(live["model"]["context_length"].as_u64(), Some(200000));
    assert_eq!(sb.current(Tool::Hermes), None);

    // 移出：只动 custom_providers，model 段不回滚（和 cc-switch 一致）。
    coexist::remove_from_live(&sb.env, Tool::Hermes, "relay").unwrap();
    assert_eq!(ids(&sb, Tool::Hermes), ["existing"]);
    assert!(sb.read(&path).contains("# keep this comment"));
}

#[test]
fn hermes_keeps_on_disk_fields_and_refuses_dict_only_providers() {
    let sb = Sandbox::new();
    let path = sb.env.hermes_config_path();
    sb.write(&path, HERMES);
    let report = import_live(&sb.env, Tool::Hermes).unwrap();
    assert_eq!(
        report.created,
        ["existing"],
        "dict-only entries are not imported"
    );
    let mut existing = store::load(&sb.env)
        .unwrap()
        .provider(Tool::Hermes, "existing")
        .unwrap()
        .clone();
    existing.settings_config["base_url"] = json!("https://changed");
    existing
        .settings_config
        .as_object_mut()
        .unwrap()
        .remove("request_timeout_seconds");
    switch::update_provider(&sb.env, Tool::Hermes, existing)
        .unwrap()
        .expect("in live");
    let live: serde_yaml::Value = serde_yaml::from_str(&sb.read(&path)).unwrap();
    assert_eq!(
        live["custom_providers"][0]["base_url"].as_str(),
        Some("https://changed")
    );
    assert_eq!(
        live["custom_providers"][0]["request_timeout_seconds"].as_u64(),
        Some(30)
    );

    let err =
        coexist::add_provider(&sb.env, Tool::Hermes, hermes_card("dict-only"), true).unwrap_err();
    assert!(err.to_string().contains("dict-only"), "{err}");
}

#[test]
fn hermes_heals_duplicate_sections_keeping_the_last() {
    let sb = Sandbox::new();
    let path = sb.env.hermes_config_path();
    sb.write(
        &path,
        "custom_providers:\n- name: stale\ncustom_providers:\n- name: fresh\n",
    );
    assert_eq!(ids(&sb, Tool::Hermes), ["fresh"]);
    sb.add(Tool::Hermes, hermes_card("relay"));
    coexist::write_to_live(&sb.env, Tool::Hermes, "relay").unwrap();
    let text = sb.read(&path);
    assert_eq!(text.matches("custom_providers:").count(), 1, "{text}");
    assert_eq!(ids(&sb, Tool::Hermes), ["fresh", "relay"]);
}

// ---------------------------------------------------------------- Pi

#[test]
fn pi_models_entries_are_private_and_protected() {
    let sb = Sandbox::new();
    let path = sb.env.pi_models_path();
    sb.write(
        &path,
        "{\n  // mine\n  \"providers\": {\n    \"own\": {\"baseUrl\": \"https://own\"}\n  }\n}\n",
    );
    coexist::add_provider(
        &sb.env,
        Tool::Pi,
        card("relay", json!({"baseUrl": "https://relay", "apiKey": "k"})),
        true,
    )
    .unwrap();
    assert!(sb.read(&path).contains("// mine"));
    assert_eq!(ids(&sb, Tool::Pi), ["own", "relay"]);
    #[cfg(unix)]
    assert_eq!(common::mode(&path), 0o600);
    let err = coexist::add_provider(
        &sb.env,
        Tool::Pi,
        card("own", json!({"baseUrl": "https://x"})),
        true,
    )
    .unwrap_err();
    assert!(err.to_string().contains("own"), "{err}");
    // Pi 自己的 settings.json / auth.json 不碰。
    assert!(!sb.env.pi_agent_dir.join("settings.json").exists());
    assert!(!sb.env.pi_agent_dir.join("auth.json").exists());

    sb.write(
        &path,
        &format!(
            "{{\"providers\": {{}}, \"pad\": \"{}\"}}",
            "x".repeat(1024 * 1024)
        ),
    );
    assert!(coexist::write_to_live(&sb.env, Tool::Pi, "relay").is_err());
}

// ---------------------------------------------------------------- MCode

fn mcode_card(id: &str) -> Provider {
    card(
        id,
        json!({"api": "openai-completions", "options": {"baseURL": "https://relay/v1", "apiKey": "k"},
               "models": {"m-1": {}, "m-2": {"enabled": true}}}),
    )
}

#[test]
fn mcode_custom_providers_respect_account_providers_and_default_models() {
    let sb = Sandbox::new();
    let path = sb.env.mcode_config_path();
    sb.write(
        &path,
        "defaultModel: custom_provider:relay/m-1\ncustom_provider:\n  minimax:\n    kind: account\n    token: secret\n",
    );
    assert!(
        ids(&sb, Tool::Mcode).is_empty(),
        "account providers are not listed"
    );
    coexist::add_provider(&sb.env, Tool::Mcode, mcode_card("relay"), true).unwrap();
    assert_eq!(ids(&sb, Tool::Mcode), ["relay"]);
    #[cfg(unix)]
    assert_eq!(common::mode(&path), 0o600);
    let live: serde_yaml::Value = serde_yaml::from_str(&sb.read(&path)).unwrap();
    assert_eq!(
        live["custom_provider"]["minimax"]["token"].as_str(),
        Some("secret")
    );
    assert!(
        !std::path::PathBuf::from(format!("{}.lock", path.display())).exists(),
        "lock released"
    );

    // relay/m-1 是默认模型：不能移出，也不能把 m-1 删掉。
    assert!(coexist::remove_from_live(&sb.env, Tool::Mcode, "relay").is_err());
    let mut edited = mcode_card("relay");
    edited.settings_config["models"] = json!({"m-2": {}});
    assert!(switch::update_provider(&sb.env, Tool::Mcode, edited).is_err());
    assert!(store::load(&sb.env)
        .unwrap()
        .provider(Tool::Mcode, "relay")
        .unwrap()
        .settings_config["models"]
        .get("m-1")
        .is_some());
    // 账号供应商不能改。
    let err = coexist::add_provider(&sb.env, Tool::Mcode, mcode_card("minimax"), true).unwrap_err();
    assert!(err.to_string().contains("账号"), "{err}");
    // 校验。
    for bad in [
        card("bad key", mcode_card("x").settings_config),
        card(
            "y",
            json!({"options": {"baseURL": "ftp://x", "apiKey": "k"}, "models": {"m": {}}}),
        ),
        card(
            "z",
            json!({"options": {"baseURL": "https://x"}, "models": {"m": {}}}),
        ),
        card(
            "w",
            json!({"options": {"baseURL": "https://x", "apiKey": "k"}, "models": {}}),
        ),
    ] {
        let id = bad.id.clone();
        assert!(
            coexist::add_provider(&sb.env, Tool::Mcode, bad, true).is_err(),
            "{id}"
        );
    }
}

#[test]
fn mcode_waits_for_its_own_lock() {
    let sb = Sandbox::new();
    let path = sb.env.mcode_config_path();
    sb.write(&path, "custom_provider: {}\n");
    let lock = std::path::PathBuf::from(format!(
        "{}.lock",
        std::fs::canonicalize(&path).unwrap().display()
    ));
    std::fs::create_dir(&lock).unwrap();
    sb.add(Tool::Mcode, mcode_card("relay"));
    let err = coexist::write_to_live(&sb.env, Tool::Mcode, "relay").unwrap_err();
    assert!(err.to_string().contains("MCode"), "{err}");
    assert_eq!(sb.read(&path), "custom_provider: {}\n");
    std::fs::remove_dir(&lock).unwrap();
    coexist::write_to_live(&sb.env, Tool::Mcode, "relay").unwrap();
    assert_eq!(ids(&sb, Tool::Mcode), ["relay"]);
}

// ---------------------------------------------------------------- 通用

#[test]
fn a_corrupt_file_is_never_overwritten() {
    let sb = Sandbox::new();
    for (tool, path, text) in [
        (Tool::OpenCode, sb.env.opencode_config_path(), "{ broken"),
        (Tool::OpenClaw, sb.env.openclaw_config_path(), "{ broken"),
        (
            Tool::Hermes,
            sb.env.hermes_config_path(),
            "custom_providers: [\n",
        ),
        (Tool::Pi, sb.env.pi_models_path(), "[1, 2]"),
        (Tool::Mcode, sb.env.mcode_config_path(), "- a\n- b\n"),
    ] {
        sb.write(&path, text);
        let settings = match tool {
            Tool::OpenCode => opencode_v1("https://x"),
            Tool::Mcode => mcode_card("x").settings_config,
            Tool::OpenClaw => json!({"baseUrl": "https://x"}),
            _ => json!({"baseUrl": "https://x"}),
        };
        let result = coexist::add_provider(&sb.env, tool, card("x", settings), true);
        assert!(result.is_err(), "{tool}");
        assert_eq!(sb.read(&path), text, "{tool}");
    }
    assert!(sb.temp_files().is_empty(), "{:?}", sb.temp_files());
}

#[test]
fn first_write_backups_are_taken_for_coexist_files() {
    let sb = Sandbox::new();
    sb.write(&sb.env.hermes_config_path(), HERMES);
    sb.add(Tool::Hermes, hermes_card("relay"));
    coexist::write_to_live(&sb.env, Tool::Hermes, "relay").unwrap();
    assert_eq!(
        sb.read(&sb.first_write_backup(&sb.env.hermes_config_path())),
        HERMES
    );
}
