use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use ccsl_core::{store, Env, Provider, Tool};
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
        let mut command = Command::new(env!("CARGO_BIN_EXE_ccsl"));
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
    assert!(ccsl_core::state::pending(&sb.env.device_store(), "claude")
        .unwrap()
        .is_none());
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
