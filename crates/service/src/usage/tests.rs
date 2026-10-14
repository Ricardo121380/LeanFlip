use std::fs;
use std::io::Write;
use std::path::Path;

use leanflip_core::store::StoreData;
use leanflip_core::{Env, Provider, Tool};
use rusqlite::Connection;
use serde_json::json;

use super::*;

const DAY: i64 = 86_400_000;

fn env() -> (tempfile::TempDir, Env) {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    (dir, env)
}

fn append(path: &Path, lines: &[serde_json::Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}

fn iso(ts: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts)
        .unwrap()
        .to_rfc3339()
}

fn no_live(_: Tool) -> Option<Vec<String>> {
    Some(Vec::new())
}

fn rows(report: &UsageReport, tool: Tool) -> Vec<&UsageRow> {
    report.rows.iter().filter(|r| r.tool == tool).collect()
}

fn sum(report: &UsageReport, tool: Tool) -> (i64, i64, i64, i64, i64) {
    rows(report, tool).iter().fold((0, 0, 0, 0, 0), |a, r| {
        (
            a.0 + r.input,
            a.1 + r.cache_read,
            a.2 + r.cache_write,
            a.3 + r.output,
            a.4 + r.requests,
        )
    })
}

fn claude_line(ts: i64, id: &str, request: &str, input: i64, output: i64) -> serde_json::Value {
    json!({
        "type": "assistant", "timestamp": iso(ts), "requestId": request,
        "message": { "id": id, "model": "claude-opus-5-5", "content": [{"type": "text", "text": "never read"}],
            "usage": { "input_tokens": input, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 50, "output_tokens": output } }
    })
}

#[test]
fn claude_records_are_deduplicated_and_read_incrementally() {
    let (_dir, env) = env();
    let now = now_ms();
    let first = env.claude_dir.join("projects/-p/a.jsonl");
    append(
        &first,
        &[
            claude_line(now - DAY, "m1", "r1", 10, 5),
            json!({"type": "user", "message": {"content": "hello"}}),
            claude_line(now - DAY, "m2", "r2", 20, 7),
            json!({"type": "assistant", "timestamp": iso(now), "message": {"model": "<synthetic>", "usage": {"input_tokens": 9, "output_tokens": 9}}}),
        ],
    );
    // A resumed session copies earlier messages into a new file.
    append(
        &env.claude_dir.join("projects/-p/b.jsonl"),
        &[claude_line(now - DAY, "m1", "r1", 10, 5)],
    );
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::Claude), (30, 2000, 100, 12, 2));
    assert!(rows(&report, Tool::Claude)
        .iter()
        .all(|r| r.attribution == Attribution::None && r.provider.is_empty()));

    append(&first, &[claude_line(now, "m3", "r3", 1, 1)]);
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::Claude).4, 3);
    let again = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&again, Tool::Claude), sum(&report, Tool::Claude));
}

#[test]
fn a_half_written_line_waits_for_the_next_scan() {
    let (_dir, env) = env();
    let path = env.claude_dir.join("projects/-p/a.jsonl");
    let line = claude_line(now_ms(), "m1", "r1", 10, 5).to_string();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, &line[..20]).unwrap();
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert!(rows(&report, Tool::Claude).is_empty());
    fs::write(&path, format!("{line}\n")).unwrap();
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::Claude).4, 1);
}

fn codex_count(ts: i64, total: [i64; 3], last: [i64; 3]) -> serde_json::Value {
    let usage = |[input, cached, output]: [i64; 3]| json!({"input_tokens": input, "cached_input_tokens": cached, "output_tokens": output, "total_tokens": input + output});
    json!({"timestamp": iso(ts), "type": "event_msg", "payload": {"type": "token_count", "info": {"total_token_usage": usage(total), "last_token_usage": usage(last)}}})
}

#[test]
fn codex_counts_differences_of_cumulative_totals_once() {
    let (_dir, env) = env();
    let now = now_ms();
    let path = env.codex_dir.join("sessions/2026/10/13/rollout-a.jsonl");
    append(
        &path,
        &[
            json!({"timestamp": iso(now), "type": "session_meta", "payload": {"model_provider": "relay"}}),
            json!({"timestamp": iso(now), "type": "turn_context", "payload": {"model": "gpt-6"}}),
            // A resumed thread starts from an older cumulative total.
            codex_count(now, [1000, 600, 40], [100, 60, 4]),
            codex_count(now, [1000, 600, 40], [100, 60, 4]),
            codex_count(now, [1300, 800, 50], [300, 200, 10]),
        ],
    );
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    // input 400 of which 260 cached; output 14; two real requests.
    assert_eq!(sum(&report, Tool::Codex), (140, 260, 0, 14, 2));
    assert_eq!(rows(&report, Tool::Codex)[0].model, "gpt-6");

    // Archiving moves the file; it must not be read again.
    let archived = env.codex_dir.join("archived_sessions/rollout-a.jsonl");
    fs::create_dir_all(archived.parent().unwrap()).unwrap();
    fs::rename(&path, &archived).unwrap();
    append(
        &archived,
        &[codex_count(now, [1400, 850, 55], [100, 50, 5])],
    );
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::Codex), (190, 310, 0, 19, 3));
}

fn provider(id: &str, config: serde_json::Value) -> Provider {
    Provider::new(id, id.to_uppercase(), config)
}

#[test]
fn codex_model_provider_is_exact_only_when_it_names_one_provider() {
    let mut data = StoreData::default();
    let app = data.app_mut(Tool::Codex);
    app.providers.push(provider(
        "relay",
        json!({"config": "model_provider = \"relay\"\n"}),
    ));
    app.providers
        .push(provider("official", json!({"config": ""})));
    app.providers
        .push(provider("api", json!({"config": "model = \"gpt-5\"\n"})));
    let codex = codex_providers(&data);
    let rec = |raw: &str| Record {
        ts: 10,
        model: "m".into(),
        who: Who::Codex(raw.into()),
        counts: Counts::default(),
    };
    assert_eq!(
        resolve(Tool::Codex, &rec("relay"), &[], &codex),
        ("relay".into(), Attribution::Exact)
    );
    // Two providers use the built-in OpenAI provider: fall back to the switch log.
    let events = [SwitchEvent {
        ts: 5,
        tool: Tool::Codex,
        provider: "api".into(),
        name: "API".into(),
        enabled: true,
    }];
    assert_eq!(
        resolve(Tool::Codex, &rec("openai"), &events, &codex),
        ("api".into(), Attribution::Time)
    );
    assert_eq!(
        resolve(Tool::Codex, &rec("openai"), &[], &codex),
        ("codex-route:openai".into(), Attribution::Route)
    );
}

#[test]
fn switch_log_records_changes_only_and_attributes_by_time() {
    let (_dir, env) = env();
    let mut data = StoreData::default();
    let app = data.app_mut(Tool::Claude);
    app.providers.push(provider("kimi", json!({})));
    app.providers.push(provider("relay", json!({})));
    app.current = Some("kimi".into());
    let conn = open(&env).unwrap();
    note(&conn, &data, &no_live, 1_000).unwrap();
    note(&conn, &data, &no_live, 2_000).unwrap();
    data.app_mut(Tool::Claude).current = Some("relay".into());
    note(&conn, &data, &no_live, 3_000).unwrap();
    let events = load_events(&conn).unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| (e.ts, e.provider.as_str(), e.name.as_str()))
            .collect::<Vec<_>>(),
        [(1_000, "kimi", "KIMI"), (3_000, "relay", "RELAY")]
    );

    let at = |ts| {
        resolve(
            Tool::Claude,
            &Record {
                ts,
                model: "m".into(),
                who: Who::Time,
                counts: Counts::default(),
            },
            &events,
            &HashMap::new(),
        )
    };
    assert_eq!(at(500), (String::new(), Attribution::None));
    assert_eq!(at(2_500), ("kimi".into(), Attribution::Time));
    assert_eq!(at(3_000), ("relay".into(), Attribution::Time));
}

#[test]
fn coexist_entries_log_enable_and_disable() {
    let (_dir, env) = env();
    let conn = open(&env).unwrap();
    let data = StoreData::default();
    let live = |entries: &'static [&'static str]| {
        move |tool: Tool| {
            Some(if tool == Tool::OpenCode {
                entries.iter().map(|s| s.to_string()).collect()
            } else {
                Vec::new()
            })
        }
    };
    note(&conn, &data, &live(&["a", "b"]), 1).unwrap();
    note(&conn, &data, &live(&["b"]), 2).unwrap();
    // An unreadable client config is skipped, not treated as empty.
    note(&conn, &data, &|_| None, 3).unwrap();
    let events = load_events(&conn).unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| (e.ts, e.provider.as_str(), e.enabled))
            .collect::<Vec<_>>(),
        [(1, "a", true), (1, "b", true), (2, "a", false)]
    );
}

#[test]
fn opencode_messages_carry_their_provider() {
    let (_dir, env) = env();
    let dir = env.home.join(".local/share/opencode");
    fs::create_dir_all(&dir).unwrap();
    let db = Connection::open(dir.join("opencode.db")).unwrap();
    db.execute_batch("CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);").unwrap();
    let now = now_ms();
    let insert = |id: &str, updated: i64, completed: bool| {
        let data = json!({"role": "assistant", "providerID": "openrouter", "modelID": "glm", "time": {"created": now, "completed": if completed { json!(now) } else { json!(null) }},
            "tokens": {"input": 100, "output": 10, "reasoning": 5, "cache": {"read": 300, "write": 2}}});
        db.execute(
            "INSERT OR REPLACE INTO message VALUES (?1, 's', ?2, ?3, ?4)",
            rusqlite::params![id, now, updated, data.to_string()],
        )
        .unwrap();
    };
    insert("m1", 1, true);
    insert("m2", 2, false);
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::OpenCode), (100, 300, 2, 15, 1));
    assert_eq!(
        (
            rows(&report, Tool::OpenCode)[0].provider.as_str(),
            rows(&report, Tool::OpenCode)[0].attribution
        ),
        ("openrouter", Attribution::Exact)
    );
    // The second message completes later.
    insert("m2", 3, true);
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::OpenCode).4, 2);
}

#[test]
fn pi_records_follow_model_changes_and_skip_fork_copies() {
    let (_dir, env) = env();
    let now = now_ms();
    let msg = |id: &str| json!({"type": "message", "id": id, "timestamp": iso(now), "message": {"role": "assistant", "usage": {"input": 7, "output": 3, "cacheRead": 20, "cacheWrite": 1}}});
    append(
        &env.pi_agent_dir.join("sessions/-p/one.jsonl"),
        &[
            json!({"type": "model_change", "provider": "moonshot", "modelId": "kimi-k2"}),
            msg("e1"),
        ],
    );
    append(
        &env.pi_agent_dir.join("sessions/-p/fork.jsonl"),
        &[
            json!({"type": "model_change", "provider": "moonshot", "modelId": "kimi-k2"}),
            msg("e1"),
            msg("e2"),
        ],
    );
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::Pi), (14, 40, 2, 6, 2));
    assert_eq!(rows(&report, Tool::Pi)[0].provider, "moonshot");
    assert_eq!(rows(&report, Tool::Pi)[0].model, "kimi-k2");
}

#[test]
fn hermes_sessions_add_only_what_grew() {
    let (_dir, env) = env();
    fs::create_dir_all(&env.hermes_dir).unwrap();
    let db = Connection::open(env.hermes_dir.join("state.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE sessions (id TEXT, model TEXT, billing_provider TEXT, started_at REAL, message_count INTEGER,
            input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER);",
    )
    .unwrap();
    let started = (now_ms() / 1000) as f64;
    db.execute(
        "INSERT INTO sessions VALUES ('s1', 'deepseek-chat', 'deepseek', ?1, 4, 100, 20, 50, 0)",
        [started],
    )
    .unwrap();
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::Hermes), (100, 50, 0, 20, 2));
    db.execute(
        "UPDATE sessions SET message_count = 6, input_tokens = 130, output_tokens = 25",
        [],
    )
    .unwrap();
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::Hermes), (130, 50, 0, 25, 3));
}

#[test]
fn grok_turns_split_by_model_and_count_model_calls() {
    let (_dir, env) = env();
    let now = now_ms();
    let turn = |prompt: &str| {
        json!({"timestamp": now, "method": "_x.ai/session/update", "params": {"update": {"sessionUpdate": "turn_completed", "prompt_id": prompt,
        "usage": {"inputTokens": 1000, "cachedReadTokens": 800, "outputTokens": 30, "modelCalls": 3,
            "modelUsage": {"grok-4.7-build": {"inputTokens": 1000, "cachedReadTokens": 800, "cacheCreationTokens": 0, "outputTokens": 30, "modelCalls": 3}}}}}})
    };
    let path = env.grok_dir.join("sessions/%2Fp/s1/updates.jsonl");
    append(&path, &[turn("p1"), turn("p1"), turn("p2")]);
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::GrokBuild), (400, 1600, 0, 60, 6));
    assert_eq!(rows(&report, Tool::GrokBuild)[0].model, "grok-4.7-build");
}

#[test]
fn gemini_sessions_are_reread_without_counting_twice() {
    let (_dir, env) = env();
    let now = now_ms();
    let message = |id: &str| {
        json!({"id": id, "type": "gemini", "timestamp": iso(now), "model": "gemini-2.5-pro", "content": "never read",
        "tokens": {"input": 500, "output": 40, "cached": 300, "thoughts": 10, "tool": 5, "total": 555}})
    };
    let path = env.gemini_dir.join("tmp/hash/chats/session-1.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        json!({"sessionId": "s1", "messages": [{"id": "u1", "type": "user"}, message("g1")]})
            .to_string(),
    )
    .unwrap();
    load(&env, &StoreData::default(), &no_live).unwrap();
    fs::write(&path, json!({"sessionId": "s1", "messages": [{"id": "u1", "type": "user"}, message("g1"), message("g2"), {"pad": "changes the size"}]}).to_string()).unwrap();
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::Gemini), (410, 600, 0, 100, 2));
}

#[test]
fn statuses_mark_missing_and_untested_sources() {
    let (_dir, env) = env();
    fs::create_dir_all(env.claude_dir.join("projects")).unwrap();
    let statuses = source_statuses(&env);
    let of = |tool| statuses.iter().find(|s| s.tool == tool).unwrap();
    assert_eq!(of(Tool::Claude).state, SourceState::Read);
    assert_eq!(of(Tool::Codex).state, SourceState::Missing);
    assert_eq!(of(Tool::ClaudeDesktop).state, SourceState::Unsupported);
    assert!(!of(Tool::Hermes).tested && !of(Tool::Pi).tested && !of(Tool::OpenClaw).tested);
    assert_eq!(of(Tool::Claude).retention_days, Some(30));
}

#[test]
fn a_broken_source_does_not_stop_the_others() {
    let (_dir, env) = env();
    fs::create_dir_all(&env.hermes_dir).unwrap();
    fs::write(env.hermes_dir.join("state.db"), b"not a database").unwrap();
    append(
        &env.claude_dir.join("projects/-p/a.jsonl"),
        &[claude_line(now_ms(), "m1", "r1", 10, 5)],
    );
    let report = load(&env, &StoreData::default(), &no_live).unwrap();
    assert_eq!(sum(&report, Tool::Claude).4, 1);
    let hermes = report
        .sources
        .iter()
        .find(|s| s.tool == Tool::Hermes)
        .unwrap();
    assert_eq!(hermes.state, SourceState::Failed);
    assert!(hermes.detail.is_some());
}

#[test]
fn claude_streaming_updates_share_one_request_and_original_attribution() {
    let (_root, env) = env();
    let path = env.claude_dir.join("projects/test/stream.jsonl");
    let time = now_ms();
    append(
        &path,
        &[
            claude_line(time, "stream", "r", 100, 1),
            claude_line(time + 1000, "stream", "r", 100, 20),
        ],
    );
    let data = StoreData::default();
    let first = load(&env, &data, &no_live).unwrap();
    assert_eq!(sum(&first, Tool::Claude), (100, 1000, 50, 20, 1));
    let copy = env.claude_dir.join("projects/test/copy.jsonl");
    fs::copy(&path, copy).unwrap();
    append(&path, &[claude_line(time, "stream", "r", 100, 1)]);
    assert_eq!(
        sum(&load(&env, &data, &no_live).unwrap(), Tool::Claude),
        (100, 1000, 50, 20, 1)
    );
}

#[test]
fn legacy_claude_first_seen_baseline_is_upgraded_without_double_counting() {
    let (_root, env) = env();
    let path = env.claude_dir.join("projects/test/stream.jsonl");
    let time = now_ms();
    append(&path, &[claude_line(time, "legacy-stream", "r", 100, 1)]);
    let data = StoreData::default();
    load(&env, &data, &no_live).unwrap();
    let db = open(&env).unwrap();
    db.execute("DELETE FROM message_counts", []).unwrap();
    db.execute(
        "UPDATE progress SET state='{}' WHERE key LIKE 'claude:%'",
        [],
    )
    .unwrap();
    drop(db);
    append(
        &path,
        &[claude_line(time + 1000, "legacy-stream", "r", 100, 20)],
    );
    assert_eq!(
        sum(&load(&env, &data, &no_live).unwrap(), Tool::Claude),
        (100, 1000, 50, 20, 1)
    );
}

#[test]
fn duplicate_official_routes_recover_complete_history_without_changing_totals() {
    let (_dir, env) = env();
    let mut data = StoreData::default();
    for id in ["official-one", "official-two"] {
        data.app_mut(Tool::Codex)
            .providers
            .push(provider(id, json!({"config":""})).official());
    }
    let now = now_ms();
    let complete = env.codex_dir.join("sessions/complete.jsonl");
    let incomplete = env.codex_dir.join("sessions/incomplete.jsonl");
    for (index, path) in [&complete, &incomplete].into_iter().enumerate() {
        append(
            path,
            &[
                json!({"type":"session_meta", "payload":{"id":format!("session-{index}"), "model_provider":"openai"}}),
                codex_count(now - DAY * (index as i64 + 1), [100, 20, 10], [100, 20, 10]),
            ],
        );
    }
    let before = load(&env, &data, &no_live).unwrap();
    assert!(rows(&before, Tool::Codex)
        .iter()
        .all(|r| r.provider == "codex-route:openai" && r.attribution == Attribution::Route));
    // Model the old database and a source file that has since expired.
    let conn = open(&env).unwrap();
    conn.execute(
        "UPDATE usage_daily SET provider='', attribution='none' WHERE tool='codex'",
        [],
    )
    .unwrap();
    conn.execute(
        "DELETE FROM progress WHERE key='migration:codex-routes-v1'",
        [],
    )
    .unwrap();
    fs::remove_file(incomplete).unwrap();
    let after = load(&env, &data, &no_live).unwrap();
    assert_eq!(sum(&before, Tool::Codex), sum(&after, Tool::Codex));
    let repaired = after
        .rows
        .iter()
        .find(|r| r.day == local_day(now - DAY))
        .unwrap();
    assert_eq!(repaired.provider, "codex-route:openai");
    let retained = after
        .rows
        .iter()
        .find(|r| r.day == local_day(now - DAY * 2))
        .unwrap();
    assert!(retained.provider.is_empty());
    assert_eq!(after.rows, load(&env, &data, &no_live).unwrap().rows);
    assert_eq!(after.coverage_start[&Tool::Codex], local_day(now - DAY * 2));
}

#[test]
fn a_recorded_route_cannot_be_overridden_by_a_conflicting_switch_log() {
    let codex = HashMap::from([("openai".into(), vec!["one".into(), "two".into()])]);
    let record = Record {
        ts: 10,
        model: "m".into(),
        who: Who::Codex("openai".into()),
        counts: Counts::default(),
    };
    let events = [SwitchEvent {
        ts: 5,
        tool: Tool::Codex,
        provider: "third-party".into(),
        name: "".into(),
        enabled: true,
    }];
    assert_eq!(
        resolve(Tool::Codex, &record, &events, &codex),
        ("codex-route:openai".into(), Attribution::Route)
    );
}
