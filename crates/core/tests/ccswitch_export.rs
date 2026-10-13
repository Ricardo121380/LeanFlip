//! 导入 cc-switch 的导出文件：SQL 导出（新旧两种）、.db 快照、旧版 JSON、坏文件。

mod common;

use std::path::{Path, PathBuf};

use common::Sandbox;
use leanflip_core::ccswitch_export::{self, DuplicatePolicy, ExportFormat, ItemStatus, Preview};
use leanflip_core::{store, Provider, Tool};
use serde_json::json;

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ccswitch-export")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

const V4: &str = "cc-switch-export-20261001_080000.sql";

#[test]
fn preview_does_not_create_a_store_lock() {
    let sb = Sandbox::new();
    let parsed = ccswitch_export::parse(&fixture(V4)).unwrap();
    ccswitch_export::preview(&sb.env, &parsed, DuplicatePolicy::Skip).unwrap();
    assert!(!sb.env.data_dir.exists());
    std::fs::create_dir_all(&sb.env.data_dir).unwrap();
    ccswitch_export::preview(&sb.env, &parsed, DuplicatePolicy::Skip).unwrap();
    assert!(!sb.env.data_dir.join("providers.json.lock").exists());
}

/// HOME 下除 LeanFlip 自己的数据目录外的所有文件。
fn files_outside_data_dir(sb: &Sandbox) -> Vec<PathBuf> {
    fn walk(dir: &Path, skip: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.starts_with(skip) {
                continue;
            }
            if path.is_dir() {
                walk(&path, skip, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(sb.home(), &sb.env.data_dir, &mut out);
    out.sort();
    out
}

fn item<'a>(preview: &'a Preview, app: &str, id: &str) -> &'a ccswitch_export::PreviewItem {
    preview
        .items
        .iter()
        .find(|i| i.app == app && i.source_id == id)
        .unwrap_or_else(|| panic!("{app}/{id} missing"))
}

fn seed(sb: &Sandbox) {
    sb.add(
        Tool::Claude,
        Provider::new(
            "taken",
            "Mine",
            json!({"env": {"ANTHROPIC_BASE_URL": "https://taken.example.com"}}),
        ),
    );
    sb.add(
        Tool::Claude,
        Provider::new(
            "mine",
            "My relay",
            json!({"env": {"ANTHROPIC_BASE_URL": "https://mine.example.com", "ANTHROPIC_AUTH_TOKEN": "sk-test-mine"}}),
        ),
    );
}

#[test]
fn sql_export_preview_classifies_every_row() {
    let sb = Sandbox::new();
    seed(&sb);
    let parsed = ccswitch_export::parse(&fixture(V4)).unwrap();
    assert_eq!(parsed.format, ExportFormat::SqlDump);
    assert_eq!(parsed.schema_version, Some(12));
    let preview = ccswitch_export::preview(&sb.env, &parsed, DuplicatePolicy::Skip).unwrap();
    assert_eq!(preview.items.len(), 21);

    let status = |app: &str, id: &str| item(&preview, app, id).status;
    let code = |app: &str, id: &str| item(&preview, app, id).reason.as_ref().map(|r| r.code);
    for (app, id) in [
        ("claude", "relay"),
        ("claude", "kimi"),
        ("claude", "claude-official"),
        ("claude-desktop", "desk"),
        ("codex", "official"),
        ("codex", "codex-relay"),
        ("gemini", "gem"),
        ("grokbuild", "grok-relay"),
        ("opencode", "kimi-oc"),
        ("openclaw", "kimi-claw"),
        ("hermes", "kimi-hermes"),
        ("pi", "kimi-pi"),
        ("mcode", "kimi-mcode"),
    ] {
        assert_eq!(
            status(app, id),
            ItemStatus::Import,
            "{app}/{id}: {:?}",
            item(&preview, app, id).reason
        );
    }
    assert_eq!(code("claude", "openai-chat"), Some("routing"));
    assert_eq!(code("codex", "copilot"), Some("routing"));
    assert_eq!(code("grokbuild", "grok-oauth"), Some("routing"));
    assert_eq!(code("opencode", "omo"), Some("plugin"));
    assert_eq!(code("copilot-cli", "x"), Some("unsupported_app"));
    assert_eq!(status("claude", "taken"), ItemStatus::Duplicate);
    assert_eq!(code("claude", "taken"), Some("same_id"));
    assert_eq!(status("claude", "same-as-mine"), ItemStatus::Duplicate);
    assert_eq!(code("claude", "same-as-mine"), Some("same_config"));
    let proxy = item(&preview, "claude-desktop", "desk-proxy");
    assert_eq!(proxy.status, ItemStatus::Skip);
    assert_eq!(proxy.reason.as_ref().unwrap().code, "unsupported_config");
    assert!(!proxy.reason.as_ref().unwrap().en.is_empty());
    // 绑定托管账号的卡：能导入，但提示要先登录。
    assert_eq!(
        item(&preview, "codex", "official").notes[0].code,
        "account_binding"
    );

    // 改名策略：同 id 改名导入，配置相同的照样跳过。
    let renamed = ccswitch_export::preview(&sb.env, &parsed, DuplicatePolicy::Rename).unwrap();
    let taken = item(&renamed, "claude", "taken");
    assert_eq!(taken.target_id.as_deref(), Some("taken-2"));
    assert_eq!(taken.reason.as_ref().unwrap().code, "renamed");
    assert_eq!(item(&renamed, "claude", "same-as-mine").target_id, None);
}

#[test]
fn applying_writes_only_the_store_and_never_tokens_or_promo_links() {
    let sb = Sandbox::new();
    seed(&sb);
    let before = files_outside_data_dir(&sb);
    let parsed = ccswitch_export::parse(&fixture(V4)).unwrap();
    let report =
        ccswitch_export::apply(&sb.env, &parsed, &Tool::ALL, DuplicatePolicy::Rename).unwrap();
    assert_eq!(report.created.len(), 14, "{:?}", report.created);
    assert!(report
        .created
        .contains(&(Tool::Claude, "taken".into(), "taken-2".into())));
    assert_eq!(report.skipped, 7);
    assert_eq!(
        files_outside_data_dir(&sb),
        before,
        "no client live config written"
    );

    let data = store::load(&sb.env).unwrap();
    let dump = serde_json::to_string(&data).unwrap();
    for secret in [
        "rt-secret",
        "at-secret",
        "id-secret",
        "oauth-secret",
        "usage-secret",
        "settings-secret",
    ] {
        assert!(!dump.contains(secret), "{secret} leaked into the store");
    }
    for tool in Tool::ALL {
        assert_eq!(
            data.app(tool).and_then(|a| a.current.clone()),
            None,
            "{tool}: pointer untouched"
        );
    }
    let kimi = data.provider(Tool::Claude, "kimi").unwrap();
    assert!(
        kimi.extra.get("websiteUrl").is_none(),
        "affiliate link dropped"
    );
    assert!(kimi.meta.extra.get("isPartner").is_none());
    assert_eq!(
        kimi.settings_config["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-test-kimi",
        "API keys kept"
    );
    let relay = data.provider(Tool::Codex, "codex-relay").unwrap();
    assert!(
        relay.extra.get("websiteUrl").is_none(),
        "path-form referral dropped"
    );
    assert_eq!(
        data.provider(Tool::Claude, "relay").unwrap().extra["websiteUrl"],
        "https://relay.example.com"
    );
    let official = data.provider(Tool::Codex, "official").unwrap();
    assert!(official.is_official_category());
    assert_eq!(
        official
            .meta
            .auth_binding
            .as_ref()
            .unwrap()
            .account_id
            .as_deref(),
        Some("acct-1")
    );
    let desk = data.provider(Tool::ClaudeDesktop, "desk").unwrap();
    assert_eq!(
        desk.meta.extra["claudeDesktopModelRoutes"]["claude-sonnet-4-5"]["labelOverride"],
        "Sonnet"
    );
    assert_eq!(
        data.provider(Tool::Claude, "taken").unwrap().name,
        "Mine",
        "existing card kept"
    );

    // 再导一次：全部是重复，什么都不加。
    let again =
        ccswitch_export::apply(&sb.env, &parsed, &Tool::ALL, DuplicatePolicy::Skip).unwrap();
    assert!(again.created.is_empty(), "{:?}", again.created);
}

#[test]
fn apply_respects_the_tool_selection() {
    let sb = Sandbox::new();
    let parsed = ccswitch_export::parse(&fixture(V4)).unwrap();
    let report =
        ccswitch_export::apply(&sb.env, &parsed, &[Tool::Gemini], DuplicatePolicy::Skip).unwrap();
    assert_eq!(
        report.created,
        [(Tool::Gemini, "gem".to_string(), "gem".to_string())]
    );
}

#[test]
fn reads_old_sql_exports_snapshots_and_legacy_json() {
    let sb = Sandbox::new();
    let old = ccswitch_export::parse(&fixture("cc-switch-export-v3.8.sql")).unwrap();
    assert_eq!(old.schema_version, Some(1));
    let preview = ccswitch_export::preview(&sb.env, &old, DuplicatePolicy::Skip).unwrap();
    assert!(
        preview.items.iter().all(|i| i.status == ItemStatus::Import),
        "{preview:?}"
    );
    assert_eq!(preview.items.len(), 2);

    // .db 快照：把 SQL 导出灌进一个真 SQLite 文件。
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("db_backup_20261001.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(std::str::from_utf8(&fixture(V4)).unwrap())
        .unwrap();
    drop(conn);
    let snap = ccswitch_export::parse(&std::fs::read(&db).unwrap()).unwrap();
    assert_eq!(snap.format, ExportFormat::SqliteDb);
    assert_eq!(snap.row_count(), 21);

    for (name, count) in [("config-v2.json", 2), ("config-v1.json", 1)] {
        let parsed = ccswitch_export::parse(&fixture(name)).unwrap();
        assert_eq!(parsed.format, ExportFormat::LegacyJson);
        let preview = ccswitch_export::preview(&sb.env, &parsed, DuplicatePolicy::Skip).unwrap();
        assert_eq!(preview.items.len(), count, "{name}");
        assert!(
            preview.items.iter().all(|i| i.status == ItemStatus::Import),
            "{name}: {preview:?}"
        );
    }
}

#[test]
fn rejects_malformed_and_hostile_files() {
    let v4 = String::from_utf8(fixture(V4)).unwrap();
    let truncated = &v4[..v4.find("COMMIT;").unwrap()];
    let attach =
        "-- CC Switch SQLite 导出\nATTACH DATABASE '/tmp/leanflip-should-not-exist.db' AS x;\n";
    let pragma = "-- CC Switch SQLite 导出\nPRAGMA writable_schema=ON;\nCREATE TABLE providers(id TEXT, app_type TEXT, name TEXT, settings_config TEXT);\n";
    let no_table = "-- CC Switch SQLite 导出\nBEGIN;\nCREATE TABLE settings (key TEXT, value TEXT);\nCOMMIT;\n";
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("plain text", b"hello".to_vec()),
        ("other sql", b"CREATE TABLE providers(id TEXT);".to_vec()),
        ("binary", vec![0xff, 0xfe, 0x00, 0x01]),
        ("truncated", truncated.as_bytes().to_vec()),
        ("attach", attach.as_bytes().to_vec()),
        ("pragma", pragma.as_bytes().to_vec()),
        ("no providers table", no_table.as_bytes().to_vec()),
        ("broken json", b"{\"claude\": {".to_vec()),
        (
            "json without providers",
            b"{\"version\": 2, \"mcp\": {}}".to_vec(),
        ),
        ("fake sqlite", b"SQLite format 3\0garbage".to_vec()),
    ];
    for (name, bytes) in cases {
        let err = ccswitch_export::parse(&bytes).expect_err(name);
        assert!(!err.to_string().is_empty(), "{name}");
    }
    assert!(
        !Path::new("/tmp/leanflip-should-not-exist.db").exists(),
        "ATTACH must not create files"
    );

    // 有 providers 表但行是坏的：照样能预览，坏行标成跳过。
    let bad_rows = "-- CC Switch SQLite 导出\nBEGIN TRANSACTION;\nCREATE TABLE providers (id TEXT, app_type TEXT, name TEXT, settings_config TEXT);\nINSERT INTO providers VALUES ('', 'claude', 'no id', '{}'),('a', 'claude', 'bad json', 'not json');\nCOMMIT;\n";
    let sb = Sandbox::new();
    let parsed = ccswitch_export::parse(bad_rows.as_bytes()).unwrap();
    let preview = ccswitch_export::preview(&sb.env, &parsed, DuplicatePolicy::Skip).unwrap();
    assert!(
        preview
            .items
            .iter()
            .all(|i| i.reason.as_ref().map(|r| r.code) == Some("malformed")),
        "{preview:?}"
    );
}
