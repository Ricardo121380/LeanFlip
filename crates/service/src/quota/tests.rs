use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, SystemTime};

use leanflip_core::codex::auth::test_id_token_with_claims;
use leanflip_core::codex::oauth::transport::TokenResponse;
use leanflip_core::provider::{AuthBinding, AUTH_PROVIDER_CODEX_OAUTH};
use leanflip_core::{store, Env, Provider};
use serde_json::{json, Value};

use super::*;
use crate::Service;

/// 2026-10-15T04:00:00Z
const NOW: i64 = 1_792_036_800_000;

fn limits(primary: Value, secondary: Value) -> Value {
    json!({
        "limit_id": "codex",
        "primary": primary,
        "secondary": secondary,
        "credits": {"has_credits": true, "unlimited": false, "balance": "60951.5625675000"},
        "plan_type": "pro",
        "rate_limit_reached_type": null
    })
}

fn weekly(used: f64, resets_at: i64) -> Value {
    json!({"used_percent": used, "window_minutes": 10080, "resets_at": resets_at})
}

fn token_count(ts: &str, rate_limits: Value) -> String {
    json!({
        "timestamp": ts,
        "type": "event_msg",
        "payload": {"type": "token_count", "info": null, "rate_limits": rate_limits}
    })
    .to_string()
}

fn session_meta(workspace: &str, user: &str) -> String {
    json!({
        "timestamp": "2026-10-15T01:00:00.000Z",
        "type": "session_meta",
        "payload": {
            "id": "s", "model_provider": "openai", "base_instructions": {"text": "long"},
            "creator_account_id": workspace, "creator_user_id": user
        }
    })
    .to_string()
}

fn write_session(dir: &Path, name: &str, lines: &[String], age: Duration) {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    let modified = SystemTime::UNIX_EPOCH + Duration::from_millis(NOW as u64) - age;
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
}

fn identity(workspace: &str, user: Option<&str>) -> CodexIdentity {
    CodexIdentity {
        workspace: workspace.into(),
        user: user.map(str::to_string),
    }
}

// ---- 本机会话记录 ----

#[test]
fn weekly_only_plan_keeps_one_window_and_credits() {
    let resets = NOW / 1000 + 3 * 86_400;
    let snap = codex_log::parse(
        &limits(weekly(51.0, resets), Value::Null),
        &json!("2026-10-15T03:18:00.000Z"),
        NOW,
    )
    .unwrap();
    assert_eq!(snap.windows.len(), 1);
    assert_eq!(snap.windows[0].minutes, 10080);
    assert!(!snap.windows[0].reset);
    assert_eq!(snap.level, QuotaLevel::Ok);
    assert_eq!(snap.used_fraction, Some(0.51));
    assert_eq!(snap.plan.as_deref(), Some("pro"));
    assert_eq!(snap.origin, QuotaOrigin::Local);
    assert_eq!(snap.credits.as_ref().unwrap().balance, Some(60951.5625675));
    assert_eq!(snap.fetched_at, NOW - 42 * 60_000);
}

#[test]
fn windows_sort_short_first_and_set_the_level() {
    let later = NOW / 1000 + 3600;
    let five_hours = json!({"used_percent": 92.0, "window_minutes": 300, "resets_at": later});
    let snap = codex_log::parse(
        &limits(weekly(40.0, later), five_hours),
        &json!("2026-10-15T03:59:00Z"),
        NOW,
    )
    .unwrap();
    assert_eq!(
        snap.windows.iter().map(|w| w.minutes).collect::<Vec<_>>(),
        [300, 10080]
    );
    assert_eq!(snap.level, QuotaLevel::Warn);

    let mut reached = limits(weekly(40.0, later), Value::Null);
    reached["rate_limit_reached_type"] = json!("primary");
    let snap = codex_log::parse(&reached, &json!("2026-10-15T03:59:00Z"), NOW).unwrap();
    assert_eq!(snap.level, QuotaLevel::Out);
}

#[test]
fn passed_reset_time_marks_the_window_and_ignores_its_usage() {
    let earlier = NOW / 1000 - 60;
    let snap = codex_log::parse(
        &limits(weekly(100.0, earlier), Value::Null),
        &json!("2026-10-08T03:59:00Z"),
        NOW,
    )
    .unwrap();
    assert!(snap.windows[0].reset);
    assert_eq!(snap.used_fraction, None);
    assert_eq!(snap.level, QuotaLevel::Ok);
}

#[test]
fn records_without_windows_or_credits_or_time_are_skipped() {
    let empty = json!({"limit_id": "codex", "primary": null, "secondary": null, "credits": null});
    assert!(codex_log::parse(&empty, &json!("2026-10-15T03:59:00Z"), NOW).is_none());
    let ok = limits(weekly(10.0, NOW / 1000 + 60), Value::Null);
    assert!(codex_log::parse(&ok, &json!("not a time"), NOW).is_none());
}

#[test]
fn reader_keeps_the_newest_record_per_identity() {
    let dir = tempfile::tempdir().unwrap();
    let codex = dir.path();
    let day = codex.join("sessions/2026/10/15");
    let resets = NOW / 1000 + 86_400;
    write_session(
        &day,
        "rollout-a-new.jsonl",
        &[
            session_meta("ws-a", "user-a"),
            token_count(
                "2026-10-15T03:00:00Z",
                limits(weekly(50.0, resets), Value::Null),
            ),
            "{\"type\":\"response_item\",\"payload\":{}}".into(),
            token_count(
                "2026-10-15T03:30:00Z",
                limits(weekly(55.0, resets), Value::Null),
            ),
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\"}}".into(),
        ],
        Duration::from_secs(60),
    );
    write_session(
        &day,
        "rollout-a-old.jsonl",
        &[
            session_meta("ws-a", "user-a"),
            token_count(
                "2026-10-15T01:00:00Z",
                limits(weekly(20.0, resets), Value::Null),
            ),
        ],
        Duration::from_secs(3600),
    );
    write_session(
        &codex.join("archived_sessions"),
        "rollout-b.jsonl",
        &[
            session_meta("ws-b", "user-b"),
            token_count(
                "2026-10-14T03:00:00Z",
                limits(weekly(12.0, resets), Value::Null),
            ),
        ],
        Duration::from_secs(86_400),
    );
    // 超过 8 天没动过的文件不看。
    write_session(
        &codex.join("sessions/2026/10/01"),
        "rollout-c.jsonl",
        &[
            session_meta("ws-c", "user-c"),
            token_count(
                "2026-10-01T03:00:00Z",
                limits(weekly(1.0, resets), Value::Null),
            ),
        ],
        Duration::from_secs(9 * 86_400),
    );
    // 没有身份字段的旧版本记录认不出是谁的。
    write_session(
        &day,
        "rollout-legacy.jsonl",
        &[
            json!({"type": "session_meta", "payload": {"id": "x"}}).to_string(),
            token_count(
                "2026-10-15T03:50:00Z",
                limits(weekly(99.0, resets), Value::Null),
            ),
        ],
        Duration::from_secs(10),
    );

    let mut found = codex_log::read(codex, NOW);
    found.sort_by(|a, b| a.identity.workspace.cmp(&b.identity.workspace));
    let summary: Vec<_> = found
        .iter()
        .map(|f| {
            (
                f.identity.workspace.as_str(),
                f.identity.user.as_deref(),
                f.snapshot.windows[0].used_percent,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("ws-a", Some("user-a"), 55.0),
            ("ws-b", Some("user-b"), 12.0)
        ]
    );
}

#[test]
fn reader_prefers_the_main_limit_and_skips_a_cut_first_line() {
    let dir = tempfile::tempdir().unwrap();
    let day = dir.path().join("sessions/2026/10/15");
    let resets = NOW / 1000 + 86_400;
    let mut other = limits(weekly(80.0, resets), Value::Null);
    other["limit_id"] = json!("codex_other");
    let filler = format!(
        "{{\"type\":\"response_item\",\"payload\":\"{}\"}}",
        "x".repeat(400)
    );
    write_session(
        &day,
        "rollout.jsonl",
        &[
            session_meta("ws", "user"),
            token_count(
                "2026-10-15T02:00:00Z",
                limits(weekly(1.0, resets), Value::Null),
            ),
            filler,
            token_count(
                "2026-10-15T03:00:00Z",
                limits(weekly(30.0, resets), Value::Null),
            ),
            token_count("2026-10-15T03:10:00Z", other),
        ],
        Duration::from_secs(60),
    );
    // 末尾 1200 字节从填充行中间开始：那半行不能当成记录。
    let found = codex_log::read_with(dir.path(), NOW, 1200);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].snapshot.windows[0].used_percent, 30.0);
}

#[test]
fn reader_steps_over_a_trailing_record_longer_than_the_tail() {
    let dir = tempfile::tempdir().unwrap();
    let day = dir.path().join("sessions/2026/10/15");
    let resets = NOW / 1000 + 86_400;
    // 最后一条是很长的工具输出，末尾一块里没有一行完整的记录。
    let output = format!(
        "{{\"type\":\"response_item\",\"payload\":\"{}\"}}",
        "x".repeat(5000)
    );
    write_session(
        &day,
        "rollout.jsonl",
        &[
            session_meta("ws", "user"),
            token_count(
                "2026-10-15T03:00:00Z",
                limits(weekly(30.0, resets), Value::Null),
            ),
            output,
        ],
        Duration::from_secs(60),
    );
    let found = codex_log::read_with(dir.path(), NOW, 1200);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].snapshot.windows[0].used_percent, 30.0);
}

// ---- 联网查询的返回 ----

#[test]
fn usage_response_uses_window_lengths_not_positions() {
    let body = json!({
        "plan_type": "pro",
        "rate_limit": {
            "allowed": true,
            "limit_reached": false,
            "primary_window": {
                "used_percent": 51, "limit_window_seconds": 604800,
                "reset_after_seconds": 3600, "reset_at": NOW / 1000 + 3600
            },
            "secondary_window": null
        },
        "credits": {"has_credits": true, "unlimited": false, "balance": "18.7907035000"}
    });
    let snap = codex_remote::parse(&body, NOW).unwrap();
    assert_eq!(snap.windows.len(), 1);
    assert_eq!(snap.windows[0].minutes, 10080);
    assert_eq!(snap.windows[0].used_percent, 51.0);
    assert_eq!(snap.windows[0].resets_at, Some(NOW / 1000 + 3600));
    assert_eq!(snap.origin, QuotaOrigin::Network);
    assert_eq!(snap.fetched_at, NOW);
    assert_eq!(snap.credits.unwrap().balance, Some(18.7907035));
}

#[test]
fn usage_response_accepts_camel_case_and_relative_resets() {
    let body = json!({
        "planType": "plus",
        "rateLimit": {
            "allowed": false,
            "primaryWindow": {"usedPercent": 92.5, "limitWindowSeconds": 18000, "resetAfterSeconds": 600},
            "secondaryWindow": {"usedPercent": 40, "limitWindowSeconds": 604800, "resetAfterSeconds": 86400}
        }
    });
    let snap = codex_remote::parse(&body, NOW).unwrap();
    assert_eq!(
        snap.windows
            .iter()
            .map(|w| (w.minutes, w.resets_at))
            .collect::<Vec<_>>(),
        [
            (300, Some(NOW / 1000 + 600)),
            (10080, Some(NOW / 1000 + 86_400))
        ]
    );
    assert!(snap.limit_reached);
    assert_eq!(snap.level, QuotaLevel::Out);
    assert_eq!(snap.plan.as_deref(), Some("plus"));
    assert!(codex_remote::parse(&json!({"rate_limit": null}), NOW).is_none());
}

fn serve(replies: Vec<(u16, &'static str, String)>) -> (String, test_server::Seen) {
    let (origin, seen) = test_server::serve(replies);
    (format!("{origin}/backend-api/wham/usage"), seen)
}

fn usage_body(used: f64) -> String {
    json!({
        "plan_type": "team",
        "rate_limit": {"primary_window": {"used_percent": used, "limit_window_seconds": 18000, "reset_after_seconds": 900}},
        "credits": {"has_credits": false, "unlimited": false, "balance": "0"}
    })
    .to_string()
}

#[test]
fn fetch_sends_the_workspace_header_and_maps_failures() {
    let (url, seen) = serve(vec![
        (200, "", usage_body(12.0)),
        (401, "", "{\"detail\":\"secret-detail\"}".into()),
        (
            302,
            "Location: https://elsewhere.example/\r\n",
            String::new(),
        ),
        (200, "", "<html>sign in</html>".into()),
        (429, "", String::new()),
    ]);
    let client = codex_remote::client().unwrap();
    let get = || codex_remote::fetch(&client, &url, "at-secret", "ws-team", NOW);

    let snap = get().unwrap();
    assert_eq!(snap.windows[0].minutes, 300);
    let request = seen.lock().unwrap()[0].clone();
    assert!(request[0].starts_with("GET /backend-api/wham/usage "));
    let header = |name: &str| test_server::header(&request, name);
    assert_eq!(header("authorization").as_deref(), Some("Bearer at-secret"));
    assert_eq!(header("chatgpt-account-id").as_deref(), Some("ws-team"));

    let codes: Vec<_> = (0..4).map(|_| get().unwrap_err()).collect();
    assert_eq!(
        codes,
        [
            codex_remote::Failure {
                code: "unauthorized",
                status: Some(401)
            },
            // 不跟随重定向，免得把 token 带到别的主机。
            codex_remote::Failure {
                code: "http",
                status: Some(302)
            },
            codex_remote::Failure {
                code: "format",
                status: None
            },
            codex_remote::Failure {
                code: "rateLimited",
                status: Some(429)
            },
        ]
    );
    assert_eq!(seen.lock().unwrap().len(), 5);
}

// ---- 缓存 ----

#[test]
fn cache_replaces_the_same_identity_and_drops_old_entries() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    let snap = |used: f64, at: i64| {
        let mut s =
            codex_remote::parse(&serde_json::from_str(&usage_body(used)).unwrap(), at).unwrap();
        s.fetched_at = at;
        s
    };
    let a = identity("ws-a", Some("user-a"));
    let b = identity("ws-b", None);
    assert!(cache::load(&env, NOW).is_empty());
    cache::save(
        &env,
        &[
            Observation {
                identity: a.clone(),
                snapshot: snap(10.0, NOW - 40 * 86_400_000),
            },
            Observation {
                identity: b.clone(),
                snapshot: snap(20.0, NOW - 1000),
            },
        ],
        &[],
        NOW - 40 * 86_400_000,
    )
    .unwrap();
    cache::save(
        &env,
        &[Observation {
            identity: b.clone(),
            snapshot: snap(30.0, NOW),
        }],
        &[],
        NOW,
    )
    .unwrap();
    let loaded = cache::load(&env, NOW);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].identity, b);
    assert_eq!(loaded[0].snapshot.windows[0].used_percent, 30.0);

    fs::write(env.data_file("quota-cache.json"), "{broken").unwrap();
    assert!(cache::load(&env, NOW).is_empty());
}

fn relay_entry(key: &str, at: i64) -> cache::RelayEntry {
    let balance = QuotaBalance {
        unit: "USD".into(),
        remaining: Some(5.0),
        limit: None,
        used: None,
        unlimited: false,
        expires_at: None,
        account_remaining: None,
    };
    cache::RelayEntry {
        key: key.into(),
        snapshot: QuotaSnapshot::new(Vec::new(), None, None, at, QuotaOrigin::Network, false)
            .with_balance(Some(balance)),
    }
}

#[test]
fn cache_hides_month_old_entries_even_before_the_next_save() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    let old = NOW - 31 * 86_400_000;
    let mut snapshot =
        codex_remote::parse(&serde_json::from_str(&usage_body(10.0)).unwrap(), old).unwrap();
    snapshot.fetched_at = old;
    cache::save(
        &env,
        &[Observation {
            identity: identity("ws-a", Some("user-a")),
            snapshot,
        }],
        &[relay_entry("relay:a", old)],
        old,
    )
    .unwrap();
    assert_eq!(cache::load(&env, old).len(), 1);
    assert_eq!(cache::load_relays(&env, old).len(), 1);
    // 之后一直没有新结果写进来，读的时候也不再显示。
    assert!(cache::load(&env, NOW).is_empty());
    assert!(cache::load_relays(&env, NOW).is_empty());
}

#[test]
fn cached_snapshots_count_resets_that_passed_since_they_were_saved() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    let reset = NOW / 1000 - 60;
    let later = NOW / 1000 + 86_400;
    let five_hours =
        |used: f64| json!({"used_percent": used, "window_minutes": 300, "resets_at": reset});
    let mut full = limits(weekly(40.0, later), five_hours(100.0));
    full["rate_limit_reached_type"] = json!("primary");
    let before = NOW - 30 * 60_000;
    let snapshot = codex_log::parse(&full, &json!("2026-10-15T03:00:00Z"), before).unwrap();
    assert!(snapshot.limit_reached);
    assert_eq!(snapshot.level, QuotaLevel::Out);
    cache::save(
        &env,
        &[Observation {
            identity: identity("ws-a", Some("user-a")),
            snapshot,
        }],
        &[],
        before,
    )
    .unwrap();

    // 读的时候 5 小时窗口已经重置：不再算用完，只剩周窗口的 40%。
    let loaded = cache::load(&env, NOW);
    let snapshot = &loaded[0].snapshot;
    assert!(snapshot.windows[0].reset);
    assert!(!snapshot.limit_reached);
    assert_eq!(snapshot.level, QuotaLevel::Ok);
    assert_eq!(snapshot.used_fraction, Some(0.4));
    // 本机记录也一样。
    let local = codex_log::parse(&full, &json!("2026-10-15T03:00:00Z"), NOW).unwrap();
    assert_eq!(local.level, QuotaLevel::Ok);

    // 周窗口还满着：限额仍然有效。
    let mut both = limits(weekly(100.0, later), five_hours(100.0));
    both["rate_limit_reached_type"] = json!("secondary");
    let local = codex_log::parse(&both, &json!("2026-10-15T03:00:00Z"), NOW).unwrap();
    assert!(local.limit_reached);
    assert_eq!(local.level, QuotaLevel::Out);
}

#[test]
fn concurrent_cache_saves_keep_every_source() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    let writers: Vec<_> = (0..8)
        .map(|i| {
            let env = env.clone();
            std::thread::spawn(move || {
                cache::save(&env, &[], &[relay_entry(&format!("relay:{i}"), NOW)], NOW).unwrap();
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!(cache::load_relays(&env, NOW).len(), 8);
}

// ---- 身份、卡片和总览 ----

fn observed(workspace: &str, user: &str, used: f64, at: i64) -> Observation {
    let mut snapshot = codex_log::parse(
        &limits(weekly(used, NOW / 1000 + 86_400), Value::Null),
        &json!("2026-10-15T03:00:00Z"),
        NOW,
    )
    .unwrap();
    snapshot.fetched_at = at;
    Observation {
        identity: identity(workspace, Some(user)),
        snapshot,
    }
}

fn card(id: &str, active: bool, who: CodexIdentity) -> CodexCard {
    CodexCard {
        provider_id: id.into(),
        provider_name: id.into(),
        active,
        identity: who,
    }
}

fn account_holder(id: &str, who: CodexIdentity) -> Holder {
    Holder {
        identity: who,
        label: Some(format!("{id}@example.com")),
        account: Some(id.into()),
        login: None,
        live: false,
    }
}

fn login_holder(login: &str, who: CodexIdentity, live: bool) -> Holder {
    Holder {
        identity: who,
        label: Some("me@example.com".into()),
        account: None,
        login: Some(login.into()),
        live,
    }
}

#[test]
fn cards_sharing_an_identity_become_one_source() {
    let view = build_view(
        vec![
            account_holder("acc-a", identity("ws-a", Some("user-a"))),
            // 本机原生登录和托管账号是同一个人：合成一个来源。
            login_holder("sub:a|account:ws-a", identity("ws-a", Some("user-a")), true),
            login_holder("sub:b|account:ws-b", identity("ws-b", None), false),
            account_holder("acc-z", identity("ws-z", Some("user-z"))),
        ],
        vec![
            card("pro", true, identity("ws-a", Some("user-a"))),
            card("pro-copy", false, identity("ws-a", Some("user-a"))),
            card("plus", false, identity("ws-b", None)),
        ],
        vec![
            observed("ws-a", "user-a", 51.0, NOW - 1000),
            observed("ws-b", "user-b", 92.0, NOW - 5000),
            observed("ws-b", "user-c", 10.0, NOW - 9000),
        ],
        &HashMap::new(),
        NOW,
    );
    let keys: Vec<_> = view.sources.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(
        keys,
        ["codex:ws-a:user-a", "codex:ws-b:*", "codex:ws-z:user-z"]
    );
    let first = &view.sources[0];
    assert_eq!(first.users.len(), 2);
    assert_eq!(first.account_id.as_deref(), Some("acc-a"));
    assert_eq!(first.login.as_deref(), Some("sub:a|account:ws-a"));
    // 只知道工作区时，取这个工作区最新的记录。
    let plus = view.sources[1].snapshot.as_ref().unwrap();
    assert_eq!(plus.windows[0].used_percent, 92.0);
    // 没有卡片的托管账号也列出来，账号页要用。
    assert!(view.sources[2].users.is_empty() && view.sources[2].snapshot.is_none());
    // 只在正在用的来源里挑：plus 更满，但它不是当前供应商。
    assert_eq!(view.busiest.as_deref(), Some("codex:ws-a:user-a"));
}

#[test]
fn newest_data_wins_and_only_newer_errors_show() {
    let a = identity("ws-a", Some("user-a"));
    let errors = HashMap::from([(
        a.key(),
        QuotaError {
            code: "unauthorized".into(),
            status: Some(401),
            at: NOW - 500,
        },
    )]);
    let view = |seen: Vec<Observation>| {
        build_view(
            vec![account_holder("acc-a", a.clone())],
            vec![],
            seen,
            &errors,
            NOW,
        )
    };
    let older = view(vec![observed("ws-a", "user-a", 10.0, NOW - 9000)]);
    assert_eq!(
        older.sources[0].error.as_ref().unwrap().code,
        "unauthorized"
    );
    let newer = view(vec![
        observed("ws-a", "user-a", 10.0, NOW - 9000),
        observed("ws-a", "user-a", 33.0, NOW - 100),
    ]);
    assert_eq!(
        newer.sources[0].snapshot.as_ref().unwrap().windows[0].used_percent,
        33.0
    );
    assert!(newer.sources[0].error.is_none());
}

#[test]
fn busiest_prefers_a_worse_level_then_a_higher_share() {
    let view = build_view(
        vec![],
        vec![
            card("a", true, identity("ws-a", Some("user-a"))),
            card("b", true, identity("ws-b", Some("user-b"))),
        ],
        vec![
            observed("ws-a", "user-a", 70.0, NOW),
            observed("ws-b", "user-b", 85.0, NOW),
        ],
        &HashMap::new(),
        NOW,
    );
    assert_eq!(view.busiest.as_deref(), Some("codex:ws-b:user-b"));
}

fn team_token() -> TokenResponse {
    let id_token = test_id_token_with_claims(&json!({
        "sub": "auth0|team",
        "email": "team@example.com",
        "https://api.openai.com/auth": {
            "chatgpt_user_id": "user-team",
            "chatgpt_account_id": "ws-team",
            "chatgpt_plan_type": "team"
        }
    }));
    TokenResponse {
        access_token: "fixture-access".into(),
        refresh_token: Some("fixture-refresh".into()),
        id_token: Some(id_token),
        expires_in: Some(3600),
    }
}

#[test]
fn cards_resolve_through_managed_accounts_login_refs_and_the_native_login() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    let oauth = CodexOAuthManager::new(&env, None).unwrap();
    let account = oauth.add_account_from_tokens(&team_token(), None).unwrap();

    let mut managed =
        Provider::new("managed", "Managed", json!({"auth": {}, "config": ""})).official();
    managed.meta.auth_binding = Some(AuthBinding {
        auth_provider: AUTH_PROVIDER_CODEX_OAUTH.into(),
        account_id: Some(account.id.clone()),
    });
    // 绑定时没写账号 id：用默认托管账号。
    let mut by_default = managed.clone();
    by_default.id = "default".into();
    by_default.meta.auth_binding.as_mut().unwrap().account_id = None;
    let mut referenced = Provider::new("ref", "Ref", json!({"auth": {}, "config": ""})).official();
    referenced.set_codex_login_ref(Some("sub:auth0|side|account:ws-side"));
    let mut missing = referenced.clone();
    missing.id = "missing".into();
    missing.set_codex_login_ref(Some("sub:gone|account:ws-gone"));
    let plain = Provider::new("plain", "Plain", json!({"auth": {}, "config": ""})).official();
    let keyed = Provider::new(
        "keyed",
        "Keyed",
        json!({"auth": {"OPENAI_API_KEY": "sk-fixture"}, "config": ""}),
    )
    .official();

    let mut data = store::StoreData::default();
    let app = data.app_mut(Tool::Codex);
    app.providers = vec![managed, by_default, referenced, missing, plain, keyed];
    app.current = Some("ref".into());
    let summary = |identity: &str, user: &str, workspace: &str, live: bool| LoginSummary {
        identity: identity.into(),
        email: Some(format!("{user}@example.com")),
        workspace: Some(workspace.into()),
        plan: Some("plus".into()),
        live,
        chatgpt_user_id: Some(user.into()),
    };
    let logins = [
        summary("sub:auth0|me|account:ws-me", "user-me", "ws-me", true),
        summary(
            "sub:auth0|side|account:ws-side",
            "user-side",
            "ws-side",
            false,
        ),
    ];

    let holders = holders(&oauth, &logins);
    assert_eq!(holders.len(), 3);
    assert_eq!(holders[0].label.as_deref(), Some("team@example.com"));
    let default = oauth.try_default_account_id().unwrap();
    let cards = codex_cards(&data, &holders, default.as_deref());
    let got: Vec<_> = cards
        .iter()
        .map(|c| (c.provider_id.as_str(), c.identity.key(), c.active))
        .collect();
    assert_eq!(
        got,
        [
            ("managed", "codex:ws-team:user-team".to_string(), false),
            ("default", "codex:ws-team:user-team".to_string(), false),
            ("ref", "codex:ws-side:user-side".to_string(), true),
            ("plain", "codex:ws-me:user-me".to_string(), false),
        ]
    );
}

fn native_auth(sub: &str, user: &str, workspace: &str, email: &str) -> Value {
    let id_token = test_id_token_with_claims(&json!({
        "sub": sub,
        "email": email,
        "https://api.openai.com/auth": {
            "chatgpt_user_id": user,
            "chatgpt_account_id": workspace,
            "chatgpt_plan_type": "pro"
        }
    }));
    json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": id_token,
            "access_token": "fixture-access",
            "refresh_token": "fixture-refresh",
            "account_id": workspace
        },
        "last_refresh": "2026-10-15T00:00:00Z"
    })
}

fn sandbox() -> (tempfile::TempDir, Env) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let env = Env::with_home(&home);
    fs::create_dir_all(&env.codex_dir).unwrap();
    (dir, env)
}

#[test]
fn overview_matches_the_live_login_to_the_current_official_card() {
    let (_dir, env) = sandbox();
    fs::write(
        env.codex_auth_path(),
        native_auth("auth0|me", "user-me", "ws-me", "me@example.com").to_string(),
    )
    .unwrap();
    let official =
        Provider::new("official", "Official", json!({"auth": {}, "config": ""})).official();
    let relay = Provider::new(
        "relay",
        "Relay",
        json!({"auth": {"OPENAI_API_KEY": "sk-relay"}, "config": ""}),
    );
    for p in [official, relay] {
        store::add_provider(&env, Tool::Codex, p).unwrap();
    }
    store::set_current(&env, Tool::Codex, "official").unwrap();

    // 会话记录用的是真实时间：写一条刚刚发生的。
    let now = now_ms();
    let stamp = chrono::DateTime::from_timestamp_millis(now - 60_000)
        .unwrap()
        .to_rfc3339();
    let day = env.codex_dir.join("sessions/2026/10/15");
    fs::create_dir_all(&day).unwrap();
    fs::write(
        day.join("rollout.jsonl"),
        [
            session_meta("ws-me", "user-me"),
            token_count(
                &stamp,
                limits(weekly(51.0, now / 1000 + 86_400), Value::Null),
            ),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();

    let service = Service::start(env, None).unwrap();
    let view = service.quota_overview().unwrap();
    assert_eq!(view.sources.len(), 1, "{view:?}");
    let source = &view.sources[0];
    assert_eq!(source.key, "codex:ws-me:user-me");
    assert_eq!(source.label.as_deref(), Some("me@example.com"));
    assert_eq!(source.login.as_deref(), Some("sub:auth0|me|account:ws-me"));
    assert_eq!(
        source.users,
        [QuotaUser {
            tool: Tool::Codex,
            provider_id: "official".into(),
            provider_name: "Official".into(),
            active: true
        }]
    );
    assert_eq!(
        source.snapshot.as_ref().unwrap().windows[0].used_percent,
        51.0
    );
    assert_eq!(view.busiest.as_deref(), Some("codex:ws-me:user-me"));
    // 给前端的数据里没有 token。
    let wire = serde_json::to_string(&view).unwrap();
    assert!(!wire.contains("fixture-access") && !wire.contains("fixture-refresh"));
}

#[test]
fn refresh_queries_managed_accounts_caches_results_and_throttles() {
    let (_dir, env) = sandbox();
    let (url, seen) = serve(vec![(200, "", usage_body(42.0)), (401, "", String::new())]);
    let mut service = Service::start(env.clone(), None).unwrap();
    service.set_quota_url(&url);
    let account = service
        .runtime()
        .oauth
        .add_account_from_tokens(&team_token(), None)
        .unwrap();

    let view = service.quota_refresh(None).unwrap();
    let source = &view.sources[0];
    assert_eq!(source.key, "codex:ws-team:user-team");
    assert_eq!(source.account_id.as_deref(), Some(account.id.as_str()));
    let snap = source.snapshot.as_ref().unwrap();
    assert_eq!(snap.origin, QuotaOrigin::Network);
    assert_eq!(snap.windows[0].used_percent, 42.0);
    assert!(source.error.is_none());
    assert!(!serde_json::to_string(&view)
        .unwrap()
        .contains("fixture-access"));

    // 一分钟内再点不会再发请求。
    service
        .quota_refresh(Some(vec!["codex:ws-team:user-team".into()]))
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);

    // 重启后从缓存读出上次的结果。
    let reopened = Service::start(env, None).unwrap();
    let cached = reopened.quota_overview().unwrap();
    assert_eq!(
        cached.sources[0].snapshot.as_ref().unwrap().windows[0].used_percent,
        42.0
    );

    // 过了间隔再查，失败时保留旧数据并带上错误。
    service.forget_quota_attempts();
    let failed = service.quota_refresh(None).unwrap();
    let source = &failed.sources[0];
    assert_eq!(
        source.snapshot.as_ref().unwrap().windows[0].used_percent,
        42.0
    );
    let error = source.error.as_ref().unwrap();
    assert_eq!(
        (error.code.as_str(), error.status),
        ("unauthorized", Some(401))
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn managed_accounts_refresh_on_their_own_only_when_the_setting_is_on() {
    let (_dir, env) = sandbox();
    let (url, seen) = serve(vec![(200, "", usage_body(42.0))]);
    let mut service = Service::start(env, None).unwrap();
    service.set_quota_url(&url);
    service
        .runtime()
        .oauth
        .add_account_from_tokens(&team_token(), None)
        .unwrap();

    assert!(!service.quota_refresh_due().unwrap(), "off by default");
    assert!(seen.lock().unwrap().is_empty());

    let mut app = service.app_settings();
    app.codex_quota_refresh_minutes = 15;
    service
        .save_settings(crate::service::SettingsInput {
            app,
            keep_codex_login: false,
        })
        .unwrap();
    assert!(service.quota_refresh_due().unwrap());
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        service.quota_overview().unwrap().sources[0]
            .snapshot
            .as_ref()
            .unwrap()
            .windows[0]
            .used_percent,
        42.0
    );

    // 结果还新：即使不算一分钟的限流也不会再查。
    service.forget_quota_attempts();
    assert!(!service.quota_refresh_due().unwrap());
    assert_eq!(seen.lock().unwrap().len(), 1);

    let mut low = crate::settings::AppSettings {
        codex_quota_refresh_minutes: 2,
        ..Default::default()
    };
    crate::settings::normalize(&mut low);
    assert_eq!(low.codex_quota_refresh_minutes, 5);
}

#[test]
fn a_workspace_only_account_counts_its_users_session_records_as_recent() {
    let (_dir, env) = sandbox();
    let now = now_ms();
    let stamp = chrono::DateTime::from_timestamp_millis(now - 60_000)
        .unwrap()
        .to_rfc3339();
    let day = env.codex_dir.join("sessions/2026/10/15");
    fs::create_dir_all(&day).unwrap();
    fs::write(
        day.join("rollout.jsonl"),
        [
            session_meta("ws-team", "user-team"),
            token_count(
                &stamp,
                limits(weekly(30.0, now / 1000 + 86_400), Value::Null),
            ),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();
    let (url, seen) = serve(vec![(200, "", usage_body(42.0))]);
    let mut service = Service::start(env, None).unwrap();
    service.set_quota_url(&url);
    // 登录凭据里没有 chatgpt_user_id：身份是 `codex:ws-team:*`。
    let mut tokens = team_token();
    tokens.id_token = Some(test_id_token_with_claims(&json!({
        "sub": "auth0|team",
        "email": "team@example.com",
        "https://api.openai.com/auth": {"chatgpt_account_id": "ws-team"}
    })));
    service
        .runtime()
        .oauth
        .add_account_from_tokens(&tokens, None)
        .unwrap();
    let mut app = service.app_settings();
    app.codex_quota_refresh_minutes = 15;
    service
        .save_settings(crate::service::SettingsInput {
            app,
            keep_codex_login: false,
        })
        .unwrap();
    let id = service.runtime().oauth.try_list_accounts().unwrap()[0]
        .id
        .clone();
    assert_eq!(
        crate::quota::account_identity(&service.runtime().oauth, &id)
            .unwrap()
            .key(),
        "codex:ws-team:*"
    );

    assert!(!service.quota_refresh_due().unwrap());
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn accounts_saved_without_a_workspace_use_the_id_token_and_ask_for_a_new_sign_in() {
    let (_dir, env) = sandbox();
    let id_token = test_id_token_with_claims(&json!({
        "sub": "auth0|old",
        "email": "old@example.com",
        "https://api.openai.com/auth": {"chatgpt_user_id": "user-old", "chatgpt_account_id": "ws-old"}
    }));
    // 旧版本存下的账号没有 chatgpt_account_id：LeanFlip 把它当作需要重新登录。
    fs::create_dir_all(&env.data_dir).unwrap();
    fs::write(
        env.data_file(leanflip_core::codex::oauth::STORE_FILENAME),
        json!({
            "version": 2,
            "default_account_id": "acc-old",
            "accounts": {"acc-old": {
                "account_id": "acc-old", "email": "old@example.com", "refresh_token": "fixture-refresh",
                "authenticated_at": 1, "id_token": id_token, "token_updated_at_ms": 0
            }}
        })
        .to_string(),
    )
    .unwrap();
    let now = now_ms();
    let stamp = chrono::DateTime::from_timestamp_millis(now - 60_000)
        .unwrap()
        .to_rfc3339();
    let day = env.codex_dir.join("sessions/2026/10/15");
    fs::create_dir_all(&day).unwrap();
    fs::write(
        day.join("rollout.jsonl"),
        [
            session_meta("ws-old", "user-old"),
            token_count(
                &stamp,
                limits(weekly(64.0, now / 1000 + 86_400), Value::Null),
            ),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();
    // 没有服务在听：如果真的发了请求，会是 network 错误而不是 reauth。
    let unused = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/wham/usage", unused.local_addr().unwrap());
    drop(unused);
    let mut service = Service::start(env, None).unwrap();
    service.set_quota_url(&url);

    let view = service.quota_refresh(None).unwrap();
    let source = &view.sources[0];
    assert_eq!(source.key, "codex:ws-old:user-old");
    assert_eq!(source.account_id.as_deref(), Some("acc-old"));
    assert_eq!(
        source.snapshot.as_ref().unwrap().windows[0].used_percent,
        64.0
    );
    assert_eq!(source.error.as_ref().unwrap().code, "reauth");
}

#[test]
fn brief_text_prefers_a_nearly_full_window_then_the_balance() {
    let snap = |windows: Vec<(i64, f64)>, balance: Option<QuotaBalance>| {
        QuotaSnapshot::new(
            windows
                .into_iter()
                .map(|(minutes, used)| QuotaWindow {
                    minutes,
                    used_percent: used,
                    resets_at: None,
                    reset: false,
                    used_amount: None,
                    limit_amount: None,
                })
                .collect(),
            None,
            None,
            NOW,
            QuotaOrigin::Network,
            false,
        )
        .with_balance(balance)
    };
    let wallet = |remaining: f64| QuotaBalance {
        unit: "USD".into(),
        remaining: Some(remaining),
        limit: None,
        used: None,
        unlimited: false,
        expires_at: None,
        account_remaining: None,
    };
    let warn = |text: &str| format!("\u{26A0}\u{FE0E} {text}");
    let text = |s: QuotaSnapshot, english: bool| brief(&s, english);
    assert_eq!(
        text(snap(vec![(10080, 51.4)], None), false).as_deref(),
        Some("周 51%")
    );
    assert_eq!(
        text(snap(vec![(10080, 51.4)], None), true).as_deref(),
        Some("wk 51%")
    );
    assert_eq!(
        text(snap(vec![(300, 92.0), (10080, 40.0)], None), false),
        Some(warn("5h 92%"))
    );
    // 余额优先，除非有窗口快满了。
    assert_eq!(
        text(snap(vec![(300, 30.0)], Some(wallet(31.4))), false).as_deref(),
        Some("$31.40")
    );
    assert_eq!(
        text(snap(vec![(300, 84.0)], Some(wallet(31.4))), false),
        Some(warn("5h 84%"))
    );
    assert_eq!(
        text(snap(vec![], Some(wallet(0.0))), false),
        Some(warn("$0.00"))
    );
    assert_eq!(
        text(snap(vec![(1440, 10.0)], None), true).as_deref(),
        Some("day 10%")
    );
    assert_eq!(
        text(snap(vec![(43200, 10.0)], None), false).as_deref(),
        Some("30天 10%")
    );
    let only_credits = QuotaSnapshot::new(
        vec![],
        Some(QuotaCredits {
            has_credits: true,
            unlimited: false,
            balance: Some(60951.56),
        }),
        None,
        NOW,
        QuotaOrigin::Local,
        false,
    );
    assert_eq!(
        brief(&only_credits, false).as_deref(),
        Some("credits 60952")
    );
    assert_eq!(text(snap(vec![], None), false), None);
}
