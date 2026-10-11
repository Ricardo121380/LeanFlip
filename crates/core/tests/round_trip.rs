//! A → B → A 往返金样测试：Claude `settings.json`、Codex `config.toml` + `auth.json`。
//!
//! 每个场景都检查：
//! - 非关键字段（用户自己的设置、注释、表、MCP……）原样保留；
//! - 第一次写入前留下了原文件的字节级备份，权限 0600；
//! - 原子写：目标文件被 rename 替换（inode 变了），沙箱里不留任何临时文件，
//!   含凭据的文件权限 0600。

mod common;

use ccsl_core::switch;
use ccsl_core::{Provider, Tool};
use common::Sandbox;
use serde_json::json;

const CLAUDE_LIVE: &str = r#"{
  "env": {
    "ANTHROPIC_BASE_URL": "https://relay-a.example",
    "ANTHROPIC_AUTH_TOKEN": "sk-a",
    "ANTHROPIC_MODEL": "glm-4.7",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
    "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS": "1",
    "MY_OWN_VAR": "keep me",
    "API_TIMEOUT_MS": "600000"
  },
  "permissions": {
    "allow": ["Bash(git status)", "Read(~/notes/**)"],
    "deny": []
  },
  "hooks": {
    "PostToolUse": [{"matcher": "Edit", "hooks": [{"type": "command", "command": "fmt.sh"}]}]
  },
  "enabledPlugins": {"review@official": true},
  "statusLine": {"type": "command", "command": "~/.claude/status.sh"},
  "includeCoAuthoredBy": false,
  "unicode": "中文 \u00e9 ✓"
}"#;

fn claude_a() -> Provider {
    Provider::new(
        "a",
        "Relay A",
        json!({"env": {
            "ANTHROPIC_BASE_URL": "https://relay-a.example",
            "ANTHROPIC_AUTH_TOKEN": "sk-a",
            "ANTHROPIC_MODEL": "glm-4.7",
            "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS": "1"
        }}),
    )
}

fn claude_b() -> Provider {
    Provider::new(
        "b",
        "Relay B",
        json!({"env": {
            "ANTHROPIC_BASE_URL": "https://relay-b.example/anthropic",
            "ANTHROPIC_API_KEY": "sk-b",
            "ANTHROPIC_DEFAULT_SONNET_MODEL": "kimi-k2",
            "CLAUDE_CODE_DISABLE_THINKING": "1"
        }}),
    )
}

#[test]
fn claude_settings_round_trip_a_b_a() {
    let sb = Sandbox::new();
    let settings = sb.env.claude_dir.join("settings.json");
    sb.write(&settings, CLAUDE_LIVE);
    #[cfg(unix)]
    let inode_before = common::inode(&settings);
    sb.add(Tool::Claude, claude_a());
    sb.add(Tool::Claude, claude_b());
    ccsl_core::store::set_current(&sb.env, Tool::Claude, "a").unwrap();

    // A → B
    let report = switch::switch(&sb.env, Tool::Claude, "b").unwrap();
    assert_eq!(report.changed, vec![settings.clone()]);
    let on_b = sb.json(&settings);
    let env = on_b["env"].as_object().unwrap();
    assert_eq!(
        env["ANTHROPIC_BASE_URL"],
        "https://relay-b.example/anthropic"
    );
    assert_eq!(env["ANTHROPIC_API_KEY"], "sk-b");
    assert_eq!(env["ANTHROPIC_DEFAULT_SONNET_MODEL"], "kimi-k2");
    assert_eq!(env["CLAUDE_CODE_DISABLE_THINKING"], "1");
    // A 的关键字段和 A 带进来的独有字段（兼容开关）都走了。
    for gone in [
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_MODEL",
        "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS",
    ] {
        assert!(
            !env.contains_key(gone),
            "A's {gone} must not survive: {on_b:#}"
        );
    }
    // 用户自己的 env（不是关键字段、也不是任何供应商的独有字段）留着。
    assert_eq!(env["API_TIMEOUT_MS"], "600000");
    // 非关键字段原样保留。
    let original: serde_json::Value = serde_json::from_str(CLAUDE_LIVE).unwrap();
    for key in [
        "permissions",
        "hooks",
        "enabledPlugins",
        "statusLine",
        "includeCoAuthoredBy",
        "unicode",
    ] {
        assert_eq!(on_b[key], original[key], "{key} must be preserved");
    }
    assert_eq!(env["MY_OWN_VAR"], "keep me");
    assert_eq!(env["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"], "1");
    assert_eq!(sb.current(Tool::Claude).as_deref(), Some("b"));

    // 首写备份：原文件的字节，0600，旁边有 .source。
    let backup = sb.first_write_backup(&settings);
    assert_eq!(sb.read(&backup), CLAUDE_LIVE);
    let marker = backup.with_file_name(format!(
        "{}.source",
        backup.file_name().unwrap().to_string_lossy()
    ));
    assert_eq!(sb.read(&marker), settings.to_string_lossy());
    #[cfg(unix)]
    {
        assert_eq!(common::mode(&backup), 0o600);
        assert_eq!(common::mode(&marker), 0o600);
        assert_eq!(common::mode(&settings), 0o600, "settings.json holds keys");
        assert_ne!(common::inode(&settings), inode_before, "replaced by rename");
    }

    // B → A：回到原文件（同样的值、同样的键序），备份不再被覆盖。
    switch::switch(&sb.env, Tool::Claude, "a").unwrap();
    let back = sb.read(&settings);
    let back_json: serde_json::Value = serde_json::from_str(&back).unwrap();
    assert_eq!(back_json, original);
    let keys =
        |v: &serde_json::Value| -> Vec<String> { v.as_object().unwrap().keys().cloned().collect() };
    assert_eq!(keys(&back_json), keys(&original), "top-level order kept");
    // 第一次写入会把文件规范化成 serde_json 的 pretty 格式（行内数组展开、`\u00e9` 写成
    // 字符），被清掉再写回的关键字段排到 env 末尾：这是 JSON 编辑器当前的输出行为，值和
    // 用户键的位置不变。规范化之后再往返就是逐字节稳定的。
    assert_ne!(back, CLAUDE_LIVE);
    switch::switch(&sb.env, Tool::Claude, "b").unwrap();
    switch::switch(&sb.env, Tool::Claude, "a").unwrap();
    assert_eq!(
        sb.read(&settings),
        back,
        "second A -> B -> A is byte-identical"
    );
    assert_eq!(sb.read(&backup), CLAUDE_LIVE, "backup is never overwritten");
    assert_eq!(sb.current(Tool::Claude).as_deref(), Some("a"));
    assert!(sb.temp_files().is_empty(), "{:?}", sb.temp_files());
}

const CODEX_LIVE: &str = r#"# my codex config
model = "gpt-5.5" # trailing comment
model_reasoning_effort = "high"
approval_policy = "on-request"
sandbox_mode = "workspace-write"

[mcp_servers.docs]
command = "npx"
args = ["-y", "docs-mcp"]

[projects."/home/me/work"]
trust_level = "trusted"

[model_providers.mine]
name = "Mine"
base_url = "https://mine.example/v1"
wire_api = "responses"
env_key = "MINE_KEY"

[profiles.fast]
model = "gpt-5.5-mini"
"#;

const OAUTH_LOGIN: &str = r#"{
  "OPENAI_API_KEY": null,
  "auth_mode": "chatgpt",
  "last_refresh": "2026-10-01T00:00:00Z",
  "tokens": {
    "access_token": "access-alice",
    "account_id": "acc-plus",
    "id_token": "eyJhbGciOiJub25lIn0.eyJzdWIiOiJhbGljZSJ9.",
    "refresh_token": "refresh-alice"
  }
}"#;

fn relay(id: &str, key: &str, url: &str, model: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({
            "auth": {"OPENAI_API_KEY": key},
            "config": format!(
                "model_provider = \"{id}\"\nmodel = \"{model}\"\nmodel_context_window = 128000\n\n[model_providers.{id}]\nname = \"{id}\"\nbase_url = \"{url}\"\nwire_api = \"responses\"\n"
            )
        }),
    )
}

/// 用户自己的内容（注释、MCP、projects、用户的 provider 表、profile）都还在。
fn assert_user_content(text: &str) {
    for needle in [
        "# my codex config",
        "# trailing comment",
        "approval_policy = \"on-request\"",
        "sandbox_mode = \"workspace-write\"",
        "[mcp_servers.docs]\ncommand = \"npx\"\nargs = [\"-y\", \"docs-mcp\"]",
        "[projects.\"/home/me/work\"]\ntrust_level = \"trusted\"",
        "[model_providers.mine]\nname = \"Mine\"\nbase_url = \"https://mine.example/v1\"\nwire_api = \"responses\"\nenv_key = \"MINE_KEY\"",
        "[profiles.fast]\nmodel = \"gpt-5.5-mini\"",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
}

/// 官方（ChatGPT 登录） → 第三方 A → 官方：登录被暂存再原样还回，config 的用户内容不动。
#[test]
fn codex_official_to_third_party_and_back_keeps_the_chatgpt_login() {
    let sb = Sandbox::new();
    let config = sb.env.codex_config_path();
    let auth = sb.env.codex_auth_path();
    sb.write(&config, CODEX_LIVE);
    sb.write(&auth, OAUTH_LOGIN);
    let report = ccsl_core::import::import_live(&sb.env, Tool::Codex).unwrap();
    assert_eq!(report.current.as_deref(), Some("official"));
    sb.add(
        Tool::Codex,
        relay("relay-a", "sk-a", "https://a.example/v1", "a-model"),
    );

    // 官方 → A
    switch::switch(&sb.env, Tool::Codex, "relay-a").unwrap();
    let on_a = sb.read(&config);
    assert_user_content(&on_a);
    let doc: toml::Table = toml::from_str(&on_a).unwrap();
    assert_eq!(doc["model_provider"].as_str(), Some("custom"));
    assert_eq!(doc["model"].as_str(), Some("a-model"));
    assert_eq!(doc["model_context_window"].as_integer(), Some(128000));
    assert!(
        doc.get("model_reasoning_effort").is_none(),
        "official's key field left"
    );
    let custom = doc["model_providers"]["custom"].as_table().unwrap();
    assert_eq!(custom["experimental_bearer_token"].as_str(), Some("sk-a"));
    assert_eq!(custom["base_url"].as_str(), Some("https://a.example/v1"));
    assert_eq!(
        custom["requires_openai_auth"].as_bool(),
        Some(false),
        "login was removed from disk; true would trap Codex on the login screen"
    );
    // auth.json 被删（不是写成 {}），登录进了暂存（0600）。
    assert!(
        !auth.exists(),
        "auth.json must be deleted, never rewritten as {{}}"
    );
    let stash_path = sb.env.data_file("codex-login-stash.json");
    let stash = sb.json(&stash_path);
    let stashed = &stash["logins"]["sub:alice|account:acc-plus"];
    assert_eq!(stashed["tokens"]["refresh_token"], "refresh-alice");
    assert_eq!(stash["last"], "sub:alice|account:acc-plus");
    // 首写备份。
    let config_backup = sb.first_write_backup(&config);
    let auth_backup = sb.first_write_backup(&auth);
    assert_eq!(sb.read(&config_backup), CODEX_LIVE);
    assert_eq!(sb.read(&auth_backup), OAUTH_LOGIN);
    #[cfg(unix)]
    {
        assert_eq!(common::mode(&stash_path), 0o600);
        assert_eq!(common::mode(&config_backup), 0o600);
        assert_eq!(common::mode(&auth_backup), 0o600);
        assert_eq!(common::mode(&config), 0o600);
    }
    // 第三方 Key 绝不进库里的官方卡、不进 auth.json。
    let store_text = sb.read(&sb.env.store_path());
    assert!(
        !store_text.contains("refresh-alice"),
        "OAuth tokens never enter the store"
    );

    // A → 官方
    #[cfg(unix)]
    let inode_on_a = common::inode(&config);
    switch::switch(&sb.env, Tool::Codex, "official").unwrap();
    let back = sb.read(&config);
    assert_user_content(&back);
    let doc: toml::Table = toml::from_str(&back).unwrap();
    assert!(doc.get("model_provider").is_none());
    assert_eq!(doc["model"].as_str(), Some("gpt-5.5"));
    assert_eq!(doc["model_reasoning_effort"].as_str(), Some("high"));
    assert!(
        doc.get("model_context_window").is_none(),
        "A's exclusive field left"
    );
    // custom 表留着（会话分桶），但不带真实 Key。
    let dormant = doc["model_providers"]["custom"].as_table().unwrap();
    assert!(dormant.get("experimental_bearer_token").is_none());
    assert!(!back.contains("sk-a"));
    // 登录原样回来，暂存清空。
    let restored = sb.json(&auth);
    assert_eq!(
        restored,
        serde_json::from_str::<serde_json::Value>(OAUTH_LOGIN).unwrap()
    );
    let stash = sb.json(&stash_path);
    assert!(stash["logins"].as_object().unwrap().is_empty());
    #[cfg(unix)]
    {
        assert_eq!(common::mode(&auth), 0o600);
        assert_ne!(common::inode(&config), inode_on_a, "replaced by rename");
    }
    // 备份只在第一次写时留，不被覆盖。
    assert_eq!(sb.read(&config_backup), CODEX_LIVE);
    assert_eq!(sb.read(&auth_backup), OAUTH_LOGIN);
    assert!(sb.temp_files().is_empty(), "{:?}", sb.temp_files());
}

/// 第三方 A → 第三方 B → A：config.toml 回到和第一次切到 A 之后逐字节相同；
/// 保留登录打开时 auth.json 一个字节都不动。
#[test]
fn codex_third_party_a_b_a_is_byte_stable() {
    for preserve in [false, true] {
        let sb = Sandbox::new();
        let config = sb.env.codex_config_path();
        let auth = sb.env.codex_auth_path();
        sb.write(&config, CODEX_LIVE);
        sb.write(&auth, OAUTH_LOGIN);
        ccsl_core::import::import_live(&sb.env, Tool::Codex).unwrap();
        sb.set_preserve(preserve);
        sb.add(
            Tool::Codex,
            relay("relay-a", "sk-a", "https://a.example/v1", "a-model"),
        );
        sb.add(
            Tool::Codex,
            relay("relay-b", "sk-b", "https://b.example/v1", "b-model"),
        );

        switch::switch(&sb.env, Tool::Codex, "relay-a").unwrap();
        let on_a = sb.read(&config);
        switch::switch(&sb.env, Tool::Codex, "relay-b").unwrap();
        let on_b = sb.read(&config);
        assert_user_content(&on_b);
        assert!(on_b.contains("sk-b") && !on_b.contains("sk-a"), "{on_b}");
        let doc: toml::Table = toml::from_str(&on_b).unwrap();
        assert_eq!(
            doc["model_providers"]["custom"]["requires_openai_auth"].as_bool(),
            Some(preserve),
            "preserve={preserve}"
        );
        switch::switch(&sb.env, Tool::Codex, "relay-a").unwrap();
        assert_eq!(sb.read(&config), on_a, "preserve={preserve}: A -> B -> A");

        if preserve {
            assert_eq!(
                sb.read(&auth),
                OAUTH_LOGIN,
                "preserved login is byte-identical"
            );
        } else {
            assert!(!auth.exists());
        }
        assert!(sb.temp_files().is_empty());
    }
}
