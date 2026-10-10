//! Codex 切换红线（移植自 cc-switch `tests/golden/codex_red_lines.rs`）。
//!
//! 只锁结果、不锁机制：「Codex 能加载」「官方登录不会被带到第三方地址」「被拒的切换
//! 没有副作用」。编号沿用原版（CX-xx）；代理、模型目录、Stack 相关的红线（15、20–24）
//! 在 ccswitch-lite 里没有对应功能，未移植。

mod common;

use std::path::PathBuf;
use std::time::SystemTime;

use ccsl_core::switch::{self, codex::WARN_AUTH_UNREADABLE};
use ccsl_core::{store, AppError, Provider, Tool};
use common::Sandbox;
use serde_json::{json, Value};
use toml::Table;

const RESERVED_IDS: &[&str] = &["openai", "ollama", "lmstudio"];
const BEDROCK_IDS: &[&str] = &["amazon-bedrock", "amazon-bedrock-runtime"];

const OAUTH_LOGIN: &str = r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":"id-token","access_token":"access-token","refresh_token":"refresh-token","account_id":"account-1"},"last_refresh":"2026-09-01T00:00:00Z"}"#;

fn codex(id: &str, key: Option<&str>, config: &str) -> Provider {
    let auth = match key {
        Some(key) => json!({ "OPENAI_API_KEY": key }),
        None => json!({}),
    };
    Provider::new(id, id, json!({ "auth": auth, "config": config }))
}

fn relay_config(id: &str, base_url: &str) -> String {
    format!(
        "model_provider = \"{id}\"\nmodel = \"gpt-5\"\n\n[model_providers.{id}]\nname = \"Relay\"\nbase_url = \"{base_url}\"\nwire_api = \"responses\"\n"
    )
}

/// 当前是官方卡（跟随 live 登录），live 里有一份 ChatGPT 登录；再加上要切过去的卡。
fn setup(targets: Vec<Provider>, logged_in: bool, preserve: bool) -> Sandbox {
    let sb = Sandbox::new();
    sb.add(
        Tool::Codex,
        Provider::new(
            "codex-official",
            "Official",
            json!({ "auth": {}, "config": "" }),
        )
        .official(),
    );
    for target in targets {
        sb.add(Tool::Codex, target);
    }
    store::set_current(&sb.env, Tool::Codex, "codex-official").unwrap();
    sb.set_preserve(preserve);
    sb.write(&sb.env.codex_config_path(), "");
    if logged_in {
        sb.write(&sb.env.codex_auth_path(), OAUTH_LOGIN);
    }
    sb
}

fn live_config_text(sb: &Sandbox) -> String {
    sb.read(&sb.env.codex_config_path())
}

fn live_config(sb: &Sandbox) -> Table {
    toml::from_str(&live_config_text(sb)).expect("config.toml parses")
}

fn live_auth(sb: &Sandbox) -> Option<String> {
    let path = sb.env.codex_auth_path();
    path.is_file().then(|| sb.read(&path))
}

fn table<'a>(value: &'a toml::Value, what: &str) -> &'a Table {
    value
        .as_table()
        .unwrap_or_else(|| panic!("{what} is not a table"))
}

fn route(doc: &Table) -> Option<(&str, &Table)> {
    let id = doc.get("model_provider")?.as_str()?;
    let providers = table(doc.get("model_providers")?, "model_providers");
    Some((id, table(providers.get(id)?, id)))
}

fn flag(table: &Table, key: &str) -> Option<bool> {
    table.get(key).and_then(toml::Value::as_bool)
}

fn assert_loadable(doc: &Table) {
    let Some(providers) = doc.get("model_providers") else {
        return;
    };
    for (id, value) in table(providers, "model_providers") {
        assert!(
            !RESERVED_IDS.contains(&id.as_str()),
            "reserved table [model_providers.{id}] makes Codex refuse the config:\n{doc:#?}"
        );
        let t = table(value, id);
        if !BEDROCK_IDS.contains(&id.as_str()) {
            assert!(t.get("aws").is_none(), "aws on a non-bedrock table {id}");
        }
        if t.contains_key("auth") {
            assert!(
                t.get("experimental_bearer_token").is_none()
                    && t.get("env_key").is_none()
                    && flag(t, "requires_openai_auth") != Some(true),
                "table {id} combines `auth` with another credential source:\n{t:#?}"
            );
        }
    }
}

fn assert_key_in_route(doc: &Table, base_url: &str, key: &str) {
    let (id, route) = route(doc).unwrap_or_else(|| panic!("no active route table:\n{doc:#?}"));
    assert!(!RESERVED_IDS.contains(&id), "route uses reserved id {id}");
    assert_eq!(
        route.get("base_url").and_then(toml::Value::as_str),
        Some(base_url),
        "route {id}: {route:#?}"
    );
    assert_eq!(
        route
            .get("experimental_bearer_token")
            .and_then(toml::Value::as_str),
        Some(key),
        "the key must travel in the active route table {id}: {route:#?}"
    );
    assert!(doc.get("experimental_bearer_token").is_none());
    assert!(doc.get("openai_base_url").is_none());
    assert_loadable(doc);
}

fn switch(sb: &Sandbox, id: &str) -> Result<Vec<String>, AppError> {
    switch::switch(&sb.env, Tool::Codex, id).map(|report| report.warnings)
}

type FileState = Option<(Vec<u8>, SystemTime)>;

#[derive(Debug, PartialEq)]
struct LiveSnapshot {
    files: Vec<(PathBuf, FileState)>,
    current: Option<String>,
    target_row: Value,
}

fn snapshot(sb: &Sandbox, target: &str) -> LiveSnapshot {
    let files = [
        sb.env.codex_config_path(),
        sb.env.codex_auth_path(),
        sb.env.data_file("codex-login-stash.json"),
    ]
    .into_iter()
    .map(|path| {
        let content = std::fs::read(&path).ok().map(|bytes| {
            let mtime = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .expect("mtime");
            (bytes, mtime)
        });
        (path, content)
    })
    .collect();
    let data = store::load(&sb.env).unwrap();
    LiveSnapshot {
        files,
        current: sb.current(Tool::Codex),
        target_row: data
            .provider(Tool::Codex, target)
            .unwrap()
            .settings_config
            .clone(),
    }
}

/// CX-01 / 02 / 03
#[test]
fn cx01_03_third_party_key_lands_in_the_active_route_table() {
    for preserve in [false, true] {
        let sb = setup(
            vec![codex(
                "relay",
                Some("sk-relay"),
                &relay_config("relay", "https://relay.example/v1"),
            )],
            true,
            preserve,
        );
        switch(&sb, "relay").expect("switch to relay");
        assert_key_in_route(&live_config(&sb), "https://relay.example/v1", "sk-relay");
        match live_auth(&sb) {
            Some(auth) => {
                assert!(preserve, "auth.json must be deleted, not rewritten: {auth}");
                assert_eq!(
                    auth, OAUTH_LOGIN,
                    "the preserved login must stay byte-identical"
                );
            }
            None => assert!(!preserve, "the preserved login was deleted"),
        }
    }
}

/// CX-04
#[test]
fn cx04_tables_declaring_their_own_auth_get_no_injected_key() {
    for preserve in [false, true] {
        let own_auth = |id: &str, extra: &str| {
            codex(
                id,
                Some("sk-stored"),
                &format!(
                    "model_provider = \"{id}\"\n\n[model_providers.{id}]\nname = \"Own\"\nbase_url = \"https://{id}.example/v1\"\nwire_api = \"responses\"\n{extra}"
                ),
            )
        };
        let sb = setup(
            vec![
                own_auth("envkey", "env_key = \"RELAY_API_KEY\"\n"),
                own_auth(
                    "header",
                    "http_headers = { authorization = \"Bearer header-token\" }\n",
                ),
                own_auth(
                    "helper",
                    "\n[model_providers.helper.auth]\ncommand = \"print-token\"\n",
                ),
            ],
            false,
            preserve,
        );
        for id in ["envkey", "header", "helper"] {
            switch(&sb, id).unwrap_or_else(|e| panic!("switch to {id}: {e:?}"));
            let text = live_config_text(&sb);
            assert!(
                !text.contains("sk-stored"),
                "{id} declares its own auth; the stored key must not be injected:\n{text}"
            );
            let doc = live_config(&sb);
            assert_loadable(&doc);
            let (_, route) = route(&doc).expect("route");
            assert_eq!(
                route.get("base_url").and_then(toml::Value::as_str),
                Some(format!("https://{id}.example/v1").as_str())
            );
            if id == "header" {
                assert_eq!(
                    route["http_headers"]["authorization"].as_str(),
                    Some("Bearer header-token")
                );
            }
        }
    }
}

/// CX-05
#[test]
fn cx05_requires_openai_auth_true_still_gets_the_key() {
    let sb = setup(
        vec![codex(
            "bridge",
            Some("sk-bridge"),
            "model_provider = \"bridge\"\n\n[model_providers.bridge]\nname = \"Bridge\"\nbase_url = \"https://bridge.example/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nhttp_headers = { Authorization = \"Bearer header-token\" }\n",
        )],
        true,
        true,
    );
    switch(&sb, "bridge").expect("switch to bridge");
    assert_key_in_route(&live_config(&sb), "https://bridge.example/v1", "sk-bridge");
    assert_eq!(live_auth(&sb).as_deref(), Some(OAUTH_LOGIN));
}

/// CX-06 / 07
#[test]
fn cx06_07_refused_switches_have_no_side_effects() {
    let cases: [(&str, bool, Option<&str>, &str); 6] = [
        ("empty-config-with-key", false, Some("sk-x"), ""),
        ("empty-config-with-key", true, Some("sk-x"), ""),
        (
            "keyless-reroute",
            false,
            None,
            "model = \"gpt-5\"\nopenai_base_url = \"https://relay.example/v1\"\n",
        ),
        (
            "keyless-reroute",
            true,
            None,
            "model = \"gpt-5\"\nopenai_base_url = \"https://relay.example/v1\"\n",
        ),
        (
            "keyless-requires",
            true,
            None,
            "model_provider = \"relay\"\n\n[model_providers.relay]\nname = \"Relay\"\nbase_url = \"https://relay.example/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nhttp_headers = { Authorization = \"Bearer h\" }\n",
        ),
        // 带 Key 但只选了内置 openai：Key 没有落点。
        (
            "key-without-slot",
            false,
            Some("sk-x"),
            "model_provider = \"openai\"\nmodel = \"m\"\n",
        ),
    ];
    for (id, preserve, key, config) in cases {
        let sb = setup(
            vec![
                codex(
                    "good",
                    Some("sk-good"),
                    &relay_config("good", "https://good.example/v1"),
                ),
                codex(id, key, config),
            ],
            true,
            preserve,
        );
        switch(&sb, "good").expect("switch to good");
        let before = snapshot(&sb, id);
        let result = switch(&sb, id);
        assert!(
            result.is_err(),
            "{id} (preserve={preserve}) must be refused"
        );
        assert_eq!(
            before,
            snapshot(&sb, id),
            "{id} (preserve={preserve}): a refused switch must not touch live, current or the row"
        );
    }
}

/// CX-09
#[test]
fn cx09_legacy_openai_base_url_reroute_never_reaches_live() {
    for preserve in [false, true] {
        let sb = setup(
            vec![codex(
                "legacy",
                Some("sk-legacy"),
                "model = \"gpt-5\"\nopenai_base_url = \"https://relay.example/v1\"\n",
            )],
            true,
            preserve,
        );
        switch(&sb, "legacy").expect("switch to legacy reroute card");
        let doc = live_config(&sb);
        assert_key_in_route(&doc, "https://relay.example/v1", "sk-legacy");
        assert_ne!(
            route(&doc)
                .unwrap()
                .1
                .get("name")
                .and_then(toml::Value::as_str),
            Some("OpenAI"),
            "a third-party table named OpenAI turns on Codex's OpenAI-only features"
        );
    }
}

/// CX-10
#[test]
fn cx10_normalization_never_overwrites_a_user_table() {
    let user_table = "[model_providers.cc-switch]\nname = \"Mine\"\nbase_url = \"https://mine.example/v1\"\nwire_api = \"responses\"\nhttp_headers = { X-Tenant = \"t1\" }\n";
    let sb = setup(
        vec![codex(
            "legacy",
            Some("sk-legacy"),
            &format!(
                "model = \"gpt-5\"\nopenai_base_url = \"https://relay.example/v1\"\n\n{user_table}"
            ),
        )],
        false,
        false,
    );
    sb.write(&sb.env.codex_config_path(), user_table);
    switch(&sb, "legacy").expect("switch to legacy reroute card");
    let doc = live_config(&sb);
    assert_key_in_route(&doc, "https://relay.example/v1", "sk-legacy");
    let mine = table(&doc["model_providers"]["cc-switch"], "cc-switch");
    let expected: Table = toml::from_str::<Table>(user_table).unwrap()["model_providers"]
        ["cc-switch"]
        .as_table()
        .unwrap()
        .clone();
    assert_eq!(
        mine, &expected,
        "the user's own table must survive untouched"
    );
}

/// CX-11
#[test]
fn cx11_inline_model_providers_keep_the_key_inside() {
    let sb = setup(
        vec![codex(
            "inline",
            Some("sk-inline"),
            "model_provider = \"custom\"\nmodel = \"gpt-5\"\nmodel_providers = { custom = { name = \"Inline\", base_url = \"https://inline.example/v1\", wire_api = \"responses\" } }\n",
        )],
        false,
        false,
    );
    switch(&sb, "inline").expect("switch to inline card");
    assert_key_in_route(&live_config(&sb), "https://inline.example/v1", "sk-inline");
}

/// CX-12
#[test]
fn cx12_reserved_ids_match_case_sensitively() {
    for id in ["OpenAI", "Ollama", "oss"] {
        let sb = setup(
            vec![codex(
                "variant",
                Some("sk-variant"),
                &relay_config(id, "https://variant.example/v1"),
            )],
            false,
            false,
        );
        switch(&sb, "variant").unwrap_or_else(|e| panic!("switch to {id}: {e:?}"));
        assert_key_in_route(
            &live_config(&sb),
            "https://variant.example/v1",
            "sk-variant",
        );
    }
}

/// CX-13
#[test]
fn cx13_stale_reserved_tables_never_reach_live() {
    for preserve in [false, true] {
        let sb = setup(
            vec![codex(
                "stale",
                Some("sk-stale"),
                "model_provider = \"openai\"\nmodel = \"gpt-5\"\n\n[model_providers.openai]\nname = \"Stale\"\nbase_url = \"https://stale.example/v1\"\nwire_api = \"chat\"\n",
            )],
            true,
            preserve,
        );
        switch(&sb, "stale").expect("switch to a card carrying a stale reserved table");
        assert_key_in_route(&live_config(&sb), "https://stale.example/v1", "sk-stale");
        if preserve {
            assert_eq!(live_auth(&sb).as_deref(), Some(OAUTH_LOGIN));
        }
    }
}

/// CX-14
#[test]
fn cx14_requires_openai_auth_follows_login_preservation() {
    let header_card = codex(
        "header",
        None,
        "model_provider = \"header\"\n\n[model_providers.header]\nname = \"Header\"\nbase_url = \"https://header.example/v1\"\nwire_api = \"responses\"\nhttp_headers = { Authorization = \"Bearer h\" }\n",
    );

    // 开关关：登录被删，活跃带 Key 的表不能再要求官方登录（否则 TUI 卡在登录屏）。
    let sb = setup(
        vec![
            codex(
                "claims-login",
                Some("sk-a"),
                "model_provider = \"relay\"\n\n[model_providers.relay]\nname = \"Relay\"\nbase_url = \"https://relay.example/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n",
            ),
            header_card.clone(),
        ],
        true,
        false,
    );
    switch(&sb, "claims-login").expect("switch");
    let doc = live_config(&sb);
    assert_eq!(
        flag(route(&doc).unwrap().1, "requires_openai_auth"),
        Some(false)
    );
    assert!(live_auth(&sb).is_none());
    switch(&sb, "header").expect("switch to header card");
    let doc = live_config(&sb);
    assert_ne!(
        flag(route(&doc).unwrap().1, "requires_openai_auth"),
        Some(true)
    );

    // 开关开且已登录：带 Key 的表要显示账号状态、让 Codex 继续刷新 token。
    let sb = setup(
        vec![
            codex(
                "silent",
                Some("sk-b"),
                &relay_config("relay", "https://relay.example/v1"),
            ),
            header_card,
        ],
        true,
        true,
    );
    switch(&sb, "silent").expect("switch");
    let doc = live_config(&sb);
    assert_eq!(
        flag(route(&doc).unwrap().1, "requires_openai_auth"),
        Some(true)
    );
    switch(&sb, "header").expect("switch to header card");
    let doc = live_config(&sb);
    assert_ne!(
        flag(route(&doc).unwrap().1, "requires_openai_auth"),
        Some(true)
    );
}

/// CX-25：auth.json 读不了（是个目录）时切换照常成功，带警告，不碰它。
#[test]
fn cx25_unreadable_auth_json_is_a_warning() {
    let sb = setup(
        vec![codex(
            "relay",
            Some("sk-relay"),
            &relay_config("relay", "https://relay.example/v1"),
        )],
        false,
        false,
    );
    std::fs::create_dir_all(sb.env.codex_auth_path()).unwrap();
    let warnings = switch(&sb, "relay").expect("the switch itself succeeds");
    assert!(
        warnings.iter().any(|w| w.starts_with(WARN_AUTH_UNREADABLE)),
        "warnings: {warnings:?}"
    );
    assert!(sb.env.codex_auth_path().is_dir(), "left untouched");
    assert_key_in_route(&live_config(&sb), "https://relay.example/v1", "sk-relay");
    assert_eq!(sb.current(Tool::Codex).as_deref(), Some("relay"));
}

/// 生效的 profile 覆盖了选路：Codex 实际不会走目标供应商，拒绝写入（原版 crate 内测试）。
#[test]
fn an_active_profile_overriding_the_route_is_refused() {
    let sb = setup(
        vec![codex(
            "relay",
            Some("sk-relay"),
            &relay_config("relay", "https://relay.example/v1"),
        )],
        true,
        false,
    );
    let live = "profile = \"work\"\n\n[profiles.work]\nmodel_provider = \"azure\"\n";
    sb.write(&sb.env.codex_config_path(), live);
    let before = snapshot(&sb, "relay");
    let err = switch(&sb, "relay").expect_err("must be refused");
    assert!(err.to_string().contains("work"), "{err}");
    assert_eq!(before, snapshot(&sb, "relay"));
}
