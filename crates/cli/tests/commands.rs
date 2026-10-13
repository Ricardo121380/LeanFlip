use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use leanflip_core::{store, Env, Provider, Tool};
use serde_json::{json, Value};

struct Sandbox {
    dir: tempfile::TempDir,
    env: Env,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path().join("home"));
        fs::create_dir(&env.home).unwrap();
        Self { dir, env }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_leanflip"));
        command.arg("--home").arg(&self.env.home).args(args);
        // --home must isolate this process even if its parent uses another tool home.
        command.env("CODEX_HOME", self.dir.path().join("unrelated-codex"));
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> Value {
        success(self.run(args))
    }

    fn input(&self, args: &[&str], provider: &Value) -> Value {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(provider.to_string().as_bytes())
            .unwrap();
        success(child.wait_with_output().unwrap())
    }

    fn write(&self, path: &Path, bytes: impl AsRef<[u8]>) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["ok"], true);
    body["data"].clone()
}

fn human(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn renamed_entrypoint_shows_its_version_and_read_only_quick_start() {
    let sb = Sandbox::new();
    let version = human(sb.run(&["--version"]));
    assert_eq!(
        version.trim(),
        format!("leanflip {}", env!("CARGO_PKG_VERSION"))
    );
    let help = human(sb.run(&[]));
    assert!(help.contains("LeanFlip · 快速入门") && help.contains("leanflip status"));
    sb.ok(&["status", "--json"]);
    assert!(snapshot(&sb.env.home).is_empty());
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(root: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.insert(path.clone(), None);
                walk(&path, out);
            } else {
                out.insert(path.clone(), Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    out
}

#[test]
fn queries_leave_empty_home_completely_untouched() {
    let sb = Sandbox::new();
    for args in [
        vec!["status", "--json"],
        vec!["--json", "list", "codex"],
        vec!["paths", "--json"],
        vec!["accounts", "list", "--json"],
    ] {
        sb.ok(&args);
        assert!(
            snapshot(&sb.env.home).is_empty(),
            "{args:?} wrote into the home"
        );
    }
    assert!(!sb.dir.path().join("unrelated-codex").exists());
}

#[test]
fn read_commands_do_not_recover_pending_operations() {
    let sb = Sandbox::new();
    let state = sb.env.device_store().state_path();
    sb.write(&state, json!({"version": 1, "apps": {"claude": {"pending": {"op": "switch", "files": [], "target": {}, "published": false}}}}).to_string());
    let before = snapshot(&sb.env.home);
    for args in [
        vec!["status", "--json"],
        vec!["list", "claude", "--json"],
        vec!["paths", "--json"],
        vec!["accounts", "list", "--json"],
    ] {
        sb.ok(&args);
        assert_eq!(
            snapshot(&sb.env.home),
            before,
            "{args:?} recovered or wrote"
        );
    }
    let report = sb.ok(&["recover", "claude", "--json"]);
    assert_eq!(report["noticeCount"], 1);
    assert!(
        leanflip_core::state::pending(&sb.env.device_store(), "claude")
            .unwrap()
            .is_none()
    );
}

#[test]
fn mutating_commands_report_automatic_recovery() {
    let sb = Sandbox::new();
    sb.write(
        &sb.env.device_store().state_path(),
        json!({
            "version": 1, "apps": {"claude": {"pending": {
                "op": "switch", "files": [], "target": {}, "published": false
            }}}
        })
        .to_string(),
    );
    let result = sb.input(&["provider", "add", "claude", "--json"], &json!({
        "id": "test", "name": "Test", "settingsConfig": {"env": {"ANTHROPIC_BASE_URL": "https://example.test", "ANTHROPIC_AUTH_TOKEN": "test-only"}}
    }));
    assert_eq!(result["recoveryNoticeCount"], 1);
}

#[test]
fn safe_listing_excludes_configuration_unknown_fields_and_metadata() {
    let sb = Sandbox::new();
    let mut card = Provider::new(
        "relay",
        "Example",
        json!({"env":{"ANTHROPIC_AUTH_TOKEN":"secret-key-must-not-leak"}}),
    );
    card.meta
        .extra
        .insert("token".into(), json!("secret-meta-must-not-leak"));
    card.extra
        .insert("private".into(), json!("secret-extra-must-not-leak"));
    store::add_provider(&sb.env, Tool::Claude, card).unwrap();
    let before = snapshot(&sb.env.home);
    for args in [
        vec!["list", "claude", "--json"],
        vec!["list", "claude"],
        vec!["status", "--json"],
    ] {
        let result = sb.run(&args);
        assert!(result.status.success());
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        for forbidden in [
            "secret-key",
            "secret-meta",
            "secret-extra",
            "settingsConfig",
        ] {
            assert!(!output.contains(forbidden), "{forbidden} leaked");
        }
    }
    assert_eq!(snapshot(&sb.env.home), before);
}

#[test]
fn provider_input_switch_and_coexist_commands_share_the_store() {
    let sb = Sandbox::new();
    let claude = json!({"id":"relay", "name":"Relay", "settingsConfig":{"env":{"ANTHROPIC_BASE_URL":"https://example.com", "ANTHROPIC_AUTH_TOKEN":"sk-private"}}});
    sb.input(&["provider", "add", "claude", "--json"], &claude);
    assert!(!sb.env.claude_settings_path().exists());
    sb.ok(&["use", "claude", "relay", "--json"]);
    let live: Value =
        serde_json::from_slice(&fs::read(sb.env.claude_settings_path()).unwrap()).unwrap();
    assert_eq!(live["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-private");
    assert_eq!(
        store::load(&sb.env)
            .unwrap()
            .current(Tool::Claude)
            .unwrap()
            .id,
        "relay"
    );

    let open = json!({"id":"relay", "name":"Relay", "settingsConfig":{"npm":"@ai-sdk/openai-compatible", "options":{"baseURL":"https://example.com", "apiKey":"sk-private-open"}, "models":{"model":{}}}});
    sb.input(
        &["provider", "add", "opencode", "--file", "-", "--json"],
        &open,
    );
    sb.ok(&["enable", "opencode", "relay", "--json"]);
    assert_eq!(
        sb.ok(&["list", "opencode", "--json"])["providers"][0]["enabled"],
        true
    );
    sb.ok(&["disable", "opencode", "relay", "--json"]);
    assert_eq!(
        sb.ok(&["list", "opencode", "--json"])["providers"][0]["enabled"],
        false
    );
    sb.ok(&["provider", "delete", "opencode", "relay", "--json"]);
    assert_eq!(
        sb.ok(&["list", "opencode", "--json"])["providers"],
        json!([])
    );
}

#[test]
fn invalid_arguments_modes_and_parse_errors_have_safe_stable_errors() {
    let sb = Sandbox::new();
    for (args, code) in [
        (vec!["list", "unknown", "--json"], "invalid_arguments"),
        (vec!["use", "opencode", "p", "--json"], "invalid_tool_mode"),
        (vec!["enable", "codex", "p", "--json"], "invalid_tool_mode"),
    ] {
        let output = sb.run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], code);
    }
    assert!(snapshot(&sb.env.home).is_empty());
    sb.write(
        &sb.env.store_path(),
        br#"{"version":"secret-in-parse-error"}"#,
    );
    let output = sb.run(&["list", "codex", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(!text.contains("secret-in-parse-error"));
    let error: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(error["error"]["code"], "json");
}

#[test]
fn export_preview_is_readonly_and_apply_only_saves_selected_cards() {
    let sb = Sandbox::new();
    let export = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/tests/fixtures/ccswitch-export/cc-switch-export-20261001_080000.sql");
    let file = export.to_str().unwrap();
    let preview = sb.ok(&["import", "file", file, "--tool", "claude", "--json"]);
    assert!(!preview["items"].as_array().unwrap().is_empty());
    assert!(preview["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["tool"] == "claude"));
    assert!(
        snapshot(&sb.env.home).is_empty(),
        "preview must not create locks or migrate a store"
    );
    let output = sb.run(&["import", "file", file, "--preview", "--json"]);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("sk-test") && !text.contains("rt-secret"));
    let applied = sb.ok(&[
        "import",
        "file",
        file,
        "--apply",
        "--tool",
        "claude",
        "--duplicate",
        "skip",
        "--json",
    ]);
    assert!(!applied["created"].as_array().unwrap().is_empty());
    let data = store::load(&sb.env).unwrap();
    assert!(!data.providers(Tool::Claude).is_empty());
    assert!(data.providers(Tool::Codex).is_empty());
    assert!(!sb.env.claude_settings_path().exists());
    let again = sb.ok(&[
        "import", "file", file, "--apply", "--tool", "claude", "--json",
    ]);
    assert_eq!(again["created"], json!([]));
}

#[test]
fn paths_honor_app_overrides_without_saving_settings() {
    let sb = Sandbox::new();
    let custom = sb.dir.path().join("custom-codex");
    let settings = sb.env.data_dir.join("app-settings.json");
    sb.write(
        &settings,
        json!({"dirOverrides":{"codexDir":custom}}).to_string(),
    );
    let before = snapshot(&sb.env.home);
    let result = sb.ok(&["paths", "codex", "--json"]);
    let paths = result["tools"]["codex"].as_array().unwrap();
    assert!(paths
        .iter()
        .all(|p| Path::new(p.as_str().unwrap()).starts_with(&custom)));
    assert_eq!(snapshot(&sb.env.home), before);
    assert!(!custom.exists());
}

#[test]
fn unreadable_live_config_is_reported_without_parser_contents() {
    let sb = Sandbox::new();
    sb.write(&sb.env.opencode_config_path(), b"{secret-from-live-parser");
    let result = sb.ok(&["list", "opencode", "--json"]);
    assert_eq!(result["liveReadable"], false);
    assert!(!result.to_string().contains("secret-from-live-parser"));
}

#[test]
fn account_listing_is_readonly_and_account_updates_never_print_tokens() {
    let sb = Sandbox::new();
    let account_path = sb.env.data_file("codex_oauth_auth.json");
    sb.write(&account_path, json!({"version":2,"accounts":{
        "one":{"account_id":"one","email":"one@example.com","refresh_token":"private-refresh-token-one","id_token":"private-id-token-one","authenticated_at":1},
        "two":{"account_id":"two","email":"two@example.com","refresh_token":"private-refresh-token-two","id_token":"private-id-token-two","authenticated_at":2}
    }, "default_account_id":"one"}).to_string());
    let before = snapshot(&sb.env.home);
    let listed = sb.ok(&["accounts", "list", "--json"]);
    assert_eq!(listed["accounts"].as_array().unwrap().len(), 2);
    assert!(!listed.to_string().contains("private-"));
    assert_eq!(snapshot(&sb.env.home), before);
    sb.ok(&["accounts", "default", "two", "--json"]);
    let listed = sb.ok(&["accounts", "list", "--json"]);
    let default = listed["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|account| account["isDefault"] == true)
        .unwrap();
    assert_eq!(default["id"], "two");
    sb.ok(&["accounts", "remove", "one", "--json"]);
    let listed = sb.ok(&["accounts", "list", "--json"]);
    assert_eq!(listed["accounts"].as_array().unwrap().len(), 1);
    assert!(!listed.to_string().contains("private-"));
}

#[test]
fn human_empty_states_provide_next_steps_in_both_languages_without_writes() {
    let sb = Sandbox::new();
    let zh = human(sb.run(&["list", "codex"]));
    assert!(zh.contains("暂无供应商"));
    assert!(zh.contains("leanflip import live codex"));
    assert!(
        zh.contains("--home"),
        "follow-up hints must retain the selected environment"
    );
    let en = human(sb.run(&["list", "codex", "--lang", "en"]));
    assert!(en.contains("No providers yet"));
    assert!(en.contains("leanflip provider add codex --file provider.json"));
    let accounts = human(sb.run(&["accounts", "list"]));
    assert!(accounts.contains("暂无托管账号"));
    assert!(accounts.contains("leanflip accounts login"));
    let status = human(sb.run(&["status"]));
    assert!(status.contains("工具概览") && status.contains("尚未选择"));
    assert!(!status.contains("false") && !status.contains("null"));
    assert!(snapshot(&sb.env.home).is_empty());
}

#[test]
fn human_list_uses_tool_specific_state_and_keeps_full_identifiers() {
    let sb = Sandbox::new();
    let id = "provider-with-a-long-stable-id-that-must-remain-copyable";
    store::add_provider(
        &sb.env,
        Tool::Claude,
        Provider::new(id, "中文供应商", json!({"env": {}})),
    )
    .unwrap();
    let narrow = human(
        sb.command(&["list", "claude"])
            .env("COLUMNS", "40")
            .output()
            .unwrap(),
    );
    assert!(narrow.contains(id));
    assert!(narrow.contains("中文供应商"));
    assert!(narrow.contains("未选中"));
    assert!(!narrow.contains("null") && !narrow.contains("false"));
    assert!(narrow.contains("leanflip use claude"));
    store::add_provider(
        &sb.env,
        Tool::OpenCode,
        Provider::new(
            "p",
            "并存卡片",
            json!({"npm":"@ai-sdk/openai-compatible","options":{},"models":{}}),
        ),
    )
    .unwrap();
    sb.write(&sb.env.opencode_config_path(), b"{secret-config-parser");
    let unreadable = human(sb.run(&["list", "opencode"]));
    assert!(unreadable.contains("无法读取"));
    assert!(unreadable.contains("leanflip paths opencode"));
    assert!(!unreadable.contains("secret-config-parser"));
}

#[test]
fn help_is_localized_and_includes_actionable_examples_without_loading_config() {
    let sb = Sandbox::new();
    sb.write(&sb.env.store_path(), b"invalid-secret-provider-store");
    let before = snapshot(&sb.env.home);
    let zh = human(sb.run(&["--help"]));
    assert!(zh.contains("用法：") && zh.contains("示例："));
    assert!(zh.contains("leanflip list codex") && zh.contains("--lang"));
    assert!(!zh.contains("Options:") && !zh.contains("Print help"));
    let en = human(sb.run(&["--lang", "en", "import", "file", "--help"]));
    assert!(en.contains("Examples:"));
    assert!(en.contains("--apply --tool codex"));
    let zh_sub = human(sb.run(&["import", "file", "--lang=zh", "--help"]));
    assert!(zh_sub.contains("仅预览，不保存") && zh_sub.contains("确认写入所选数据"));
    assert_eq!(snapshot(&sb.env.home), before);
}

#[test]
fn language_never_changes_json_bytes_error_messages_or_exit_codes() {
    let sb = Sandbox::new();
    for args in [
        vec!["status", "--json"],
        vec!["list", "codex", "--json"],
        vec!["accounts", "list", "--json"],
        vec!["use", "opencode", "p", "--json"],
        vec!["list", "invalid-private-tool", "--json"],
    ] {
        let mut zh_args = args.clone();
        zh_args.extend(["--lang", "zh"]);
        let mut en_args = args;
        en_args.extend(["--lang", "en"]);
        let zh = sb.run(&zh_args);
        let en = sb.run(&en_args);
        assert_eq!(zh.status.code(), en.status.code());
        assert_eq!(zh.stdout, en.stdout);
        assert_eq!(zh.stderr, en.stderr);
        assert!(!String::from_utf8_lossy(&zh.stderr).contains("invalid-private-tool"));
    }
    assert!(snapshot(&sb.env.home).is_empty());
}

#[test]
fn human_preview_explains_fixed_reason_codes_and_preserves_json() {
    let sb = Sandbox::new();
    let export = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/tests/fixtures/ccswitch-export/cc-switch-export-20261001_080000.sql");
    let path = export.to_str().unwrap();
    let text = human(sb.run(&["import", "file", path, "--tool", "claude"]));
    assert!(text.contains("导入预览") && text.contains("尚未保存"));
    assert!(text.contains("需要代理或协议转换"));
    assert!(text.contains("--apply"));
    assert!(!text.contains("sk-test") && !text.contains("rt-secret"));
    let zh = sb.run(&["import", "file", path, "--json", "--lang", "zh"]);
    let en = sb.run(&["import", "file", path, "--json", "--lang", "en"]);
    assert_eq!(zh.stdout, en.stdout);
    assert!(snapshot(&sb.env.home).is_empty());
}

#[test]
fn missing_commands_show_help_without_opening_configuration_and_json_stays_an_error() {
    let sb = Sandbox::new();
    sb.write(
        &sb.env.store_path(),
        b"private-invalid-config-that-must-not-be-read",
    );
    let before = snapshot(&sb.env.home);
    let quick_start = human(sb.run(&[]));
    assert!(quick_start.contains("快速入门") && quick_start.contains("leanflip status"));
    assert!(!quick_start.contains("错误 [") && !quick_start.contains("private-invalid"));
    let english = human(sb.run(&["--lang", "en"]));
    assert!(english.contains("Quick start") && english.contains("Examples:"));
    let group = human(sb.run(&["provider"]));
    assert!(group.contains("leanflip provider") && group.contains("新增、更新或删除"));
    for args in [
        vec!["--json"],
        vec!["provider", "--json"],
        vec!["--lang", "en", "--json"],
    ] {
        let result = sb.run(&args);
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        let error: Value = serde_json::from_slice(&result.stderr).unwrap();
        assert_eq!(
            error,
            json!({"ok":false,"error":{"code":"invalid_arguments","message":"Invalid arguments. Run leanflip --help or leanflip <command> --help for usage."}})
        );
    }
    assert_eq!(snapshot(&sb.env.home), before);
}

#[test]
fn argument_errors_show_only_recognized_command_help_and_never_raw_arguments() {
    let sb = Sandbox::new();
    for (args, usage) in [
        (
            vec!["list", "private-invalid-tool"],
            "leanflip list [OPTIONS] <TOOL>",
        ),
        (
            vec!["provider", "add", "private-invalid-tool"],
            "leanflip provider add [OPTIONS] <TOOL>",
        ),
        (
            vec![
                "providers",
                "update",
                "codex",
                "--file",
                "private-provider-path",
                "--private-unknown-flag",
            ],
            "leanflip provider update [OPTIONS] <TOOL>",
        ),
        (
            vec!["accounts", "login", "--timeout", "private-invalid-timeout"],
            "leanflip accounts login [OPTIONS]",
        ),
        (
            vec![
                "import",
                "file",
                "private-export-path",
                "--preview",
                "--apply",
            ],
            "leanflip import file [OPTIONS] <PATH>",
        ),
        (
            vec!["provider", "private-invalid-action"],
            "leanflip provider",
        ),
    ] {
        let result = sb.run(&args);
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        let text = String::from_utf8(result.stderr).unwrap();
        assert!(text.contains(usage), "{args:?}: {text}");
        assert!(!text.contains("private-"), "raw argument leaked: {text}");
        assert!(text.contains("invalid_arguments"));
        if args[0] == "list" || args[1] == "add" {
            assert!(
                text.contains("codex") && text.contains("opencode"),
                "valid tools must remain discoverable"
            );
        }
    }
    let en = sb.run(&["--lang", "en", "list"]);
    let text = String::from_utf8(en.stderr).unwrap();
    assert!(text.contains("Usage: leanflip list") && text.contains("Examples:"));
    assert!(snapshot(&sb.env.home).is_empty());
}

#[test]
fn closing_a_human_output_pipe_is_quiet_success_while_json_retains_exit_one() {
    let sb = Sandbox::new();
    // Larger than a pipe's capacity, so the producer must observe the reader
    // closing even if the subprocess starts before we drop its stdout handle.
    store::add_provider(
        &sb.env,
        Tool::Claude,
        Provider::new("large", "display-text".repeat(100_000), json!({"env":{}})),
    )
    .unwrap();
    for (args, expected_exit) in [
        (vec!["list", "claude"], 0),
        (vec!["list", "claude", "--json"], 1),
    ] {
        let mut child = sb
            .command(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let result = child.wait_with_output().unwrap();
        assert_eq!(result.status.code(), Some(expected_exit));
        assert!(
            result.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn interactive_menu_requires_a_terminal_and_never_consumes_piped_input() {
    let sb = Sandbox::new();
    for args in [
        vec!["menu"],
        vec!["menu", "codex"],
        vec!["ui", "--lang", "en"],
        vec!["tui"],
        vec!["tui", "codex", "--lang", "en"],
    ] {
        let output = sb.run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8(output.stderr)
            .unwrap()
            .contains("interactive_terminal"));
        assert!(snapshot(&sb.env.home).is_empty());
    }
    let output = sb.run(&["menu", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "invalid_arguments");
    assert!(snapshot(&sb.env.home).is_empty());
    let output = sb.run(&["tui", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "invalid_arguments");
    assert!(snapshot(&sb.env.home).is_empty());
}

#[test]
fn interactive_help_is_bilingual_and_plain_no_argument_help_stays_noninteractive() {
    let sb = Sandbox::new();
    assert!(human(sb.run(&["menu", "--help"])).contains("交互菜单"));
    assert!(human(sb.run(&["menu", "--help", "--lang", "en"])).contains("searchable menu"));
    assert!(human(sb.run(&["tui", "--help"])).contains("全屏终端工作台"));
    assert!(human(sb.run(&["tui", "--help", "--lang", "en"])).contains("full-screen terminal"));
    assert!(human(sb.run(&[])).contains("快速入门"));
    assert!(snapshot(&sb.env.home).is_empty());
}

#[test]
fn local_ccswitch_migration_previews_without_writes_and_preserves_source() {
    let sb = Sandbox::new();
    let source = sb.env.home.join(".cc-switch/config.json");
    sb.write(&source, json!({"claude":{"providers":{"imported":{"name":"Local fixture","settingsConfig":{"env":{"ANTHROPIC_AUTH_TOKEN":"private-migration-fixture"}}}}}}).to_string());
    let before = snapshot(&sb.env.home);
    let output = sb.run(&["import", "local", "--json"]);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-migration-fixture"));
    let preview = success(output);
    assert_eq!(preview["items"][0]["id"], "imported");
    assert_eq!(snapshot(&sb.env.home), before);
    let applied = sb.ok(&["import", "local", "--apply", "--tool", "claude", "--json"]);
    assert_eq!(applied["created"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read(&source).unwrap(), before[&source].clone().unwrap());
    assert!(!sb.env.claude_settings_path().exists());
    assert_eq!(
        sb.ok(&["import", "local", "--apply", "--json"])["created"],
        json!([])
    );
}

#[test]
fn encrypted_cli_backup_roundtrip_and_scope_preview_are_secret_safe() {
    let source = Sandbox::new();
    let target = Sandbox::new();
    let mut data = store::StoreData::default();
    data.app_mut(Tool::Claude).providers.push(serde_json::from_value(json!({"id":"fixture", "name":"Work", "settingsConfig":{"env":{"ANTHROPIC_AUTH_TOKEN":"fixture-encrypted-key"}}})).unwrap());
    store::update(&source.env, |target| {
        *target = data;
        Ok(())
    })
    .unwrap();
    let pass = source.dir.path().join("passphrase");
    source.write(&pass, "fixture-private-passphrase\n");
    let backup = source.dir.path().join("backup.leanflip");
    source.ok(&[
        "export",
        backup.to_str().unwrap(),
        "--password-file",
        pass.to_str().unwrap(),
        "--json",
    ]);
    let before = snapshot(&target.env.home);
    let preview = target.ok(&[
        "import",
        "backup",
        backup.to_str().unwrap(),
        "--password-file",
        pass.to_str().unwrap(),
        "--no-accounts",
        "--no-settings",
        "--tool",
        "claude",
        "--json",
    ]);
    assert_eq!(preview["selection"]["tools"], json!(["claude"]));
    assert_eq!(preview["selection"]["paths"], false);
    assert!(!preview.to_string().contains("fixture-encrypted-key"));
    assert_eq!(snapshot(&target.env.home), before);
    let report = target.ok(&[
        "import",
        "backup",
        backup.to_str().unwrap(),
        "--password-file",
        pass.to_str().unwrap(),
        "--apply",
        "--json",
    ]);
    assert_eq!(report["providersAdded"], 1);
    assert!(!target.env.claude_settings_path().exists());
    assert_eq!(
        store::load_readonly(&target.env)
            .unwrap()
            .providers(Tool::Claude)[0]
            .settings_config["env"]["ANTHROPIC_AUTH_TOKEN"],
        "fixture-encrypted-key"
    );
    let missing_password = target.run(&["import", "backup", backup.to_str().unwrap(), "--json"]);
    assert!(!missing_password.status.success());
    assert!(String::from_utf8_lossy(&missing_password.stderr).contains("backup.password_required"));
}
