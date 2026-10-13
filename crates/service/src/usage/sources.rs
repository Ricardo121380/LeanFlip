//! 各工具的会话记录。只取时间、模型、供应商和 token 数字，不读对话内容。
//!
//! 追加写的 JSONL 从上次读到的位置继续读；整份重写的 JSON 在大小或修改时间变了时重读，
//! 靠去重键不重复计数。Hermes、OpenClaw、Pi 和 Gemini CLI 按公开资料接入，没在真实数据上
//! 验证过（`tested: false`）。

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use leanflip_core::{Env, Tool};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};

use super::{Batch, Counts, Progress, Record, Scan, SourceState, SourceStatus, Who};

type Run = fn(&Env, &mut Scan) -> Result<(), String>;

struct Source {
    tool: Tool,
    tested: bool,
    experimental: bool,
    retention_days: Option<u32>,
    /// 判断“本机有没有记录”的位置；`None` 表示这个工具不在本机留用量记录。
    path: Option<PathBuf>,
    run: Option<Run>,
}

fn opencode_dir(env: &Env) -> PathBuf {
    env.home.join(".local").join("share").join("opencode")
}

fn sources(env: &Env) -> Vec<Source> {
    let src =
        |tool, tested, experimental, retention_days, path: Option<PathBuf>, run: Option<Run>| {
            Source {
                tool,
                tested,
                experimental,
                retention_days,
                path,
                run,
            }
        };
    vec![
        src(
            Tool::Claude,
            true,
            false,
            Some(30),
            Some(env.claude_dir.join("projects")),
            Some(claude),
        ),
        src(
            Tool::Codex,
            true,
            false,
            None,
            Some(env.codex_dir.join("sessions")),
            Some(codex),
        ),
        src(
            Tool::Gemini,
            false,
            false,
            Some(30),
            Some(env.gemini_dir.join("tmp")),
            Some(gemini),
        ),
        src(
            Tool::GrokBuild,
            true,
            true,
            None,
            Some(env.grok_dir.join("sessions")),
            Some(grok),
        ),
        src(Tool::ClaudeDesktop, true, false, None, None, None),
        src(
            Tool::OpenCode,
            true,
            false,
            None,
            Some(opencode_dir(env)),
            Some(opencode),
        ),
        src(
            Tool::OpenClaw,
            false,
            false,
            None,
            Some(env.openclaw_dir.join("agents")),
            Some(openclaw),
        ),
        src(
            Tool::Hermes,
            false,
            false,
            None,
            Some(env.hermes_dir.join("state.db")),
            Some(hermes),
        ),
        src(
            Tool::Pi,
            false,
            false,
            None,
            Some(env.pi_agent_dir.join("sessions")),
            Some(pi),
        ),
        src(Tool::Mcode, true, false, None, None, None),
    ]
}

impl Source {
    fn status(&self) -> SourceStatus {
        let state = match &self.path {
            None => SourceState::Unsupported,
            Some(path) if path.exists() => SourceState::Read,
            Some(_) => SourceState::Missing,
        };
        SourceStatus {
            tool: self.tool,
            state,
            tested: self.tested,
            experimental: self.experimental,
            path: self.path.as_ref().map(|p| p.display().to_string()),
            retention_days: self.retention_days,
            detail: None,
        }
    }
}

/// 不扫描，只看各工具的记录位置在不在。
pub fn source_statuses(env: &Env) -> Vec<SourceStatus> {
    sources(env).iter().map(Source::status).collect()
}

/// 扫描所有找得到的记录。一个工具出错不影响其他工具。
pub(super) fn scan_all(env: &Env, scan: &mut Scan) -> Vec<SourceStatus> {
    sources(env)
        .into_iter()
        .map(|source| {
            let mut status = source.status();
            if status.state == SourceState::Read {
                if let Some(run) = source.run {
                    if let Err(error) = run(env, scan) {
                        log::warn!("usage source {} failed: {error}", source.tool);
                        status.state = SourceState::Failed;
                        status.detail = Some(error);
                    }
                }
            }
            status
        })
        .collect()
}

// ---- 小工具 ----

fn text<E: std::fmt::Display>(error: E) -> String {
    error.to_string()
}

/// 非负整数字段；缺失或不是数字时为 0。
fn n(value: &Value, key: &str) -> i64 {
    value
        .get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .unwrap_or(0)
        .max(0)
}

fn first_n(value: &Value, keys: &[&str]) -> i64 {
    keys.iter()
        .map(|k| n(value, k))
        .find(|&v| v > 0)
        .unwrap_or(0)
}

fn iso(value: &Value) -> Option<i64> {
    let text = value.as_str()?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.timestamp_millis())
}

fn meta(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some((meta.len() as i64, mtime))
}

fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some(ext)
}

fn name_of(path: &Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
}

/// `dir` 下深度不超过 `depth` 的文件，按路径排序。不跟随目录符号链接，不会绕圈。
fn files(dir: &Path, depth: usize, keep: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, keep: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if depth > 0 {
                    walk(&path, depth - 1, keep, out);
                }
            } else if (kind.is_file() || kind.is_symlink()) && keep(&path) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, depth, keep, &mut out);
    out.sort();
    out
}

/// 一组文件逐个读；某个文件出错时继续读其他文件，最后报告第一个错误。
fn each_file(
    paths: Vec<PathBuf>,
    mut read: impl FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let mut first = None;
    for path in paths {
        if let Err(error) = read(&path) {
            first.get_or_insert_with(|| format!("{}: {error}", path.display()));
        }
    }
    first.map_or(Ok(()), Err)
}

type Line<'a> = &'a mut dyn FnMut(&Scan, &mut Batch, &mut Value, &Value) -> rusqlite::Result<()>;

/// 追加写的 JSONL：从上次读到的位置继续读完整的行，结尾没写完的半行下次再读。
/// 不含任何 `needles` 的行直接跳过，不解析。
fn read_lines(
    scan: &mut Scan,
    tool: Tool,
    key: &str,
    path: &Path,
    needles: &[&str],
    each: Line,
) -> Result<(), String> {
    let Some((size, mtime)) = meta(path) else {
        return Ok(());
    };
    let mut progress = scan.progress(key).map_err(text)?;
    if progress.offset == size && progress.size == size && progress.mtime == mtime {
        return Ok(());
    }
    // 文件变短说明被整个替换了：从头读，但保留状态，累计值不会算两次。
    if size < progress.offset {
        progress.offset = 0;
    }
    let mut file = File::open(path).map_err(text)?;
    file.seek(SeekFrom::Start(progress.offset as u64))
        .map_err(text)?;
    let mut reader = BufReader::with_capacity(1 << 16, file);
    let mut state = match std::mem::take(&mut progress.state) {
        Value::Object(map) => Value::Object(map),
        _ => json!({}),
    };
    let mut batch = Batch::default();
    let mut offset = progress.offset;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read = reader.read_until(b'\n', &mut buf).map_err(text)?;
        if read == 0 || buf.last() != Some(&b'\n') {
            break;
        }
        offset += read as i64;
        let Ok(line) = std::str::from_utf8(&buf) else {
            continue;
        };
        if !needles.iter().any(|needle| line.contains(needle)) {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        each(scan, &mut batch, &mut state, &value).map_err(text)?;
    }
    let progress = Progress {
        size,
        mtime,
        offset,
        state,
    };
    scan.commit(tool, key, &progress, batch).map_err(text)
}

/// 整份重写的文件：大小或修改时间变了才重读，重复的记录靠去重键跳过。
fn read_whole(
    scan: &mut Scan,
    tool: Tool,
    key: &str,
    path: &Path,
    each: &mut dyn FnMut(&Scan, &mut Batch, &str) -> rusqlite::Result<()>,
) -> Result<(), String> {
    let Some((size, mtime)) = meta(path) else {
        return Ok(());
    };
    let progress = scan.progress(key).map_err(text)?;
    if progress.size == size && progress.mtime == mtime {
        return Ok(());
    }
    let content = std::fs::read_to_string(path).map_err(text)?;
    let mut batch = Batch::default();
    each(scan, &mut batch, &content).map_err(text)?;
    let progress = Progress {
        size,
        mtime,
        offset: size,
        state: Value::Null,
    };
    scan.commit(tool, key, &progress, batch).map_err(text)
}

fn open_readonly(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(text)?;
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(text)?;
    Ok(conn)
}

fn record(ts: i64, model: &str, who: Who, counts: Counts) -> Record {
    Record {
        ts,
        model: if model.is_empty() {
            "unknown".into()
        } else {
            model.into()
        },
        who,
        counts,
    }
}

// ---- Claude Code：`projects/**/*.jsonl`，每条 assistant 消息带 `usage`。 ----

fn claude(env: &Env, scan: &mut Scan) -> Result<(), String> {
    let mut paths = files(&env.claude_dir.join("projects"), 5, &|p| {
        has_ext(p, "jsonl")
    });
    // 较新的版本也可能写在 XDG 目录下。
    paths.extend(files(
        &env.home.join(".config").join("claude").join("projects"),
        5,
        &|p| has_ext(p, "jsonl"),
    ));
    each_file(paths, |path| {
        let key = format!("claude:{}", path.display());
        read_lines(
            scan,
            Tool::Claude,
            &key,
            path,
            &["\"usage\""],
            &mut |scan, batch, _, v| {
                let message = &v["message"];
                let usage = &message["usage"];
                if v["type"] != "assistant" || !usage.is_object() {
                    return Ok(());
                }
                let model = message["model"].as_str().unwrap_or_default();
                let Some(ts) = iso(&v["timestamp"]) else {
                    return Ok(());
                };
                if model == "<synthetic>" {
                    return Ok(());
                }
                // 恢复或分叉会话会把旧消息复制进新文件，用消息 ID + 请求 ID 去重。
                let key = match (message["id"].as_str(), v["requestId"].as_str()) {
                    (Some(id), Some(request)) => Some(format!("c:{id}:{request}")),
                    _ => None,
                };
                let counts = Counts {
                    input: n(usage, "input_tokens"),
                    cache_read: n(usage, "cache_read_input_tokens"),
                    cache_write: n(usage, "cache_creation_input_tokens"),
                    output: n(usage, "output_tokens"),
                    requests: 1,
                };
                scan.add(batch, key, record(ts, model, Who::Time, counts))
            },
        )
    })
}

// ---- Codex：`sessions/年/月/日/rollout-*.jsonl`。 ----
//
// `token_count` 事件里的 `total_token_usage` 是会话累计值，同一次请求可能重复出现，续接的会话
// 从旧的累计值开始，所以按累计值求差，差为 0 的跳过。输入包含缓存命中部分。

#[derive(Clone, Copy, Default)]
struct CodexTotals {
    input: i64,
    cached: i64,
    cache_write: i64,
    output: i64,
    total: i64,
}

impl CodexTotals {
    fn from(v: &Value) -> Self {
        Self {
            input: n(v, "input_tokens"),
            cached: n(v, "cached_input_tokens"),
            cache_write: n(v, "cache_write_input_tokens"),
            output: n(v, "output_tokens"),
            total: n(v, "total_tokens"),
        }
    }

    fn minus(self, o: Self) -> Self {
        Self {
            input: (self.input - o.input).max(0),
            cached: (self.cached - o.cached).max(0),
            cache_write: (self.cache_write - o.cache_write).max(0),
            output: (self.output - o.output).max(0),
            total: (self.total - o.total).max(0),
        }
    }
}

fn codex(env: &Env, scan: &mut Scan) -> Result<(), String> {
    // 归档时文件从 sessions 搬到 archived_sessions，按文件名记进度，搬家后不会重读。
    let mut paths = files(&env.codex_dir.join("sessions"), 4, &|p| has_ext(p, "jsonl"));
    let archived = files(&env.codex_dir.join("archived_sessions"), 1, &|p| {
        has_ext(p, "jsonl")
    });
    let active: std::collections::HashSet<String> =
        paths.iter().map(|p| name_of(p).to_string()).collect();
    paths.extend(
        archived
            .into_iter()
            .filter(|p| !active.contains(name_of(p))),
    );
    each_file(paths, |path| {
        let key = format!("codex:{}", name_of(path));
        read_lines(
            scan,
            Tool::Codex,
            &key,
            path,
            &["token_count", "session_meta", "turn_context"],
            &mut |scan, batch, state, v| {
                let payload = &v["payload"];
                match v["type"].as_str() {
                    Some("session_meta") => {
                        state["provider"] =
                            json!(payload["model_provider"].as_str().unwrap_or("openai"));
                    }
                    Some("turn_context") => {
                        if let Some(model) = payload["model"].as_str() {
                            state["model"] = json!(model);
                        }
                    }
                    Some("event_msg") if payload["type"] == "token_count" => {
                        let info = &payload["info"];
                        if !info["total_token_usage"].is_object() {
                            return Ok(());
                        }
                        let current = CodexTotals::from(&info["total_token_usage"]);
                        let previous = if state["prev"].is_object() {
                            CodexTotals::from(&state["prev"])
                        } else if info["last_token_usage"].is_object() {
                            current.minus(CodexTotals::from(&info["last_token_usage"]))
                        } else {
                            // 没有单次用量又没有基准，无法分辨哪些属于这个文件，从下一次开始算。
                            current
                        };
                        state["prev"] = info["total_token_usage"].clone();
                        let delta = current.minus(previous);
                        if delta.total == 0 {
                            return Ok(());
                        }
                        let Some(ts) = iso(&v["timestamp"]) else {
                            return Ok(());
                        };
                        let counts = Counts {
                            input: (delta.input - delta.cached - delta.cache_write).max(0),
                            cache_read: delta.cached,
                            cache_write: delta.cache_write,
                            output: delta.output,
                            requests: 1,
                        };
                        let provider = state["provider"].as_str().unwrap_or("openai").to_string();
                        let model = state["model"].as_str().unwrap_or_default().to_string();
                        scan.add(
                            batch,
                            None,
                            record(ts, &model, Who::Codex(provider), counts),
                        )?;
                    }
                    _ => {}
                }
                Ok(())
            },
        )
    })
}

// ---- Gemini CLI：`tmp/<项目>/chats/` 下的会话，模型消息带 `tokens`（未实测）。 ----
//
// 输入包含缓存命中部分；思考 token 单独记，算进输出。

fn gemini_message(
    scan: &Scan,
    batch: &mut Batch,
    session: &str,
    m: &Value,
) -> rusqlite::Result<()> {
    let tokens = &m["tokens"];
    if m["type"] != "gemini" || !tokens.is_object() {
        return Ok(());
    }
    let Some(ts) = iso(&m["timestamp"]) else {
        return Ok(());
    };
    let input = first_n(tokens, &["input", "prompt", "promptTokenCount"]);
    let cached = first_n(tokens, &["cached", "cachedContentTokenCount"]);
    let counts = Counts {
        input: (input - cached).max(0) + first_n(tokens, &["tool", "toolUsePromptTokenCount"]),
        cache_read: cached,
        cache_write: 0,
        output: first_n(tokens, &["output", "candidates", "candidatesTokenCount"])
            + first_n(tokens, &["thoughts", "thoughtsTokenCount"]),
        requests: 1,
    };
    let key = m["id"].as_str().map(|id| format!("g:{session}:{id}"));
    scan.add(
        batch,
        key,
        record(
            ts,
            m["model"].as_str().unwrap_or("gemini"),
            Who::Time,
            counts,
        ),
    )
}

fn gemini(env: &Env, scan: &mut Scan) -> Result<(), String> {
    let paths = files(&env.gemini_dir.join("tmp"), 2, &|p| {
        let in_chats = p
            .parent()
            .and_then(|d| d.file_name())
            .and_then(|n| n.to_str())
            == Some("chats");
        (in_chats && (has_ext(p, "json") || has_ext(p, "jsonl")))
            || (name_of(p).starts_with("session-") && has_ext(p, "json"))
    });
    each_file(paths, |path| {
        let key = format!("gemini:{}", path.display());
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        read_whole(
            scan,
            Tool::Gemini,
            &key,
            path,
            &mut |scan, batch, content| {
                let documents: Vec<Value> = if has_ext(path, "jsonl") {
                    content
                        .lines()
                        .filter_map(|line| serde_json::from_str(line).ok())
                        .collect()
                } else {
                    serde_json::from_str(content).into_iter().collect()
                };
                let mut session = stem.clone();
                for doc in &documents {
                    if let Some(id) = doc["sessionId"].as_str() {
                        session = id.to_string();
                    }
                    match doc["messages"].as_array() {
                        Some(messages) => {
                            for m in messages {
                                gemini_message(scan, batch, &session, m)?;
                            }
                        }
                        None => gemini_message(scan, batch, &session, doc)?,
                    }
                }
                Ok(())
            },
        )
    })
}

// ---- Grok Build：`sessions/<目录>/<会话>/updates.jsonl` 里每轮结束的 `turn_completed`。 ----
//
// 每条是这一轮的用量（不是累计），按模型拆开；输入包含缓存命中部分。格式没有公开文档。

fn grok(env: &Env, scan: &mut Scan) -> Result<(), String> {
    let paths = files(&env.grok_dir.join("sessions"), 2, &|p| {
        name_of(p) == "updates.jsonl"
    });
    each_file(paths, |path| {
        let key = format!("grok:{}", path.display());
        read_lines(
            scan,
            Tool::GrokBuild,
            &key,
            path,
            &["turn_completed"],
            &mut |scan, batch, _, v| {
                let params = &v["params"];
                let update = &params["update"];
                let usage = &update["usage"];
                if update["sessionUpdate"] != "turn_completed" || !usage.is_object() {
                    return Ok(());
                }
                let Some(ts) = v["timestamp"]
                    .as_i64()
                    .or_else(|| params["_meta"]["agentTimestampMs"].as_i64())
                else {
                    return Ok(());
                };
                let per_model: Vec<(&str, &Value)> = match usage["modelUsage"].as_object() {
                    Some(models) if !models.is_empty() => {
                        models.iter().map(|(k, v)| (k.as_str(), v)).collect()
                    }
                    _ => vec![("grok", usage)],
                };
                for (model, u) in per_model {
                    let cache_read = n(u, "cachedReadTokens");
                    let cache_write = n(u, "cacheCreationTokens");
                    let counts = Counts {
                        input: (n(u, "inputTokens") - cache_read - cache_write).max(0),
                        cache_read,
                        cache_write,
                        output: n(u, "outputTokens"),
                        requests: n(u, "modelCalls").max(1),
                    };
                    let key = update["prompt_id"]
                        .as_str()
                        .map(|id| format!("x:{id}:{model}"));
                    scan.add(batch, key, record(ts, model, Who::Time, counts))?;
                }
                Ok(())
            },
        )
    })
}

// ---- OpenCode：`~/.local/share/opencode/opencode.db`（1.2 起），更早的版本是 `storage/message/`。 ----
//
// assistant 消息自带 providerID，就是客户端配置里的条目名，也就是 LeanFlip 的供应商 ID。

fn opencode_message(
    scan: &Scan,
    batch: &mut Batch,
    id: &str,
    created: i64,
    m: &Value,
) -> rusqlite::Result<()> {
    let tokens = &m["tokens"];
    // 还没完成的消息先不算，完成后 time_updated 会变，下次扫描再读到。
    if m["role"] != "assistant" || !tokens.is_object() || m["time"]["completed"].is_null() {
        return Ok(());
    }
    let ts = m["time"]["created"].as_i64().unwrap_or(created);
    let counts = Counts {
        input: n(tokens, "input"),
        cache_read: n(&tokens["cache"], "read"),
        cache_write: n(&tokens["cache"], "write"),
        output: n(tokens, "output") + n(tokens, "reasoning"),
        requests: 1,
    };
    let who = m["providerID"]
        .as_str()
        .map(|p| Who::Exact(p.to_string()))
        .unwrap_or(Who::Unknown);
    scan.add(
        batch,
        Some(format!("o:{id}")),
        record(ts, m["modelID"].as_str().unwrap_or_default(), who, counts),
    )
}

fn opencode_db(scan: &mut Scan, path: &Path) -> Result<(), String> {
    let key = format!("opencode:{}", path.display());
    let (size, mtime) = meta(path).unwrap_or_default();
    let progress = scan.progress(&key).map_err(text)?;
    let conn = open_readonly(path)?;
    let mut stmt = conn
        .prepare("SELECT id, time_created, time_updated, data FROM message WHERE time_updated > ?1 ORDER BY time_updated")
        .map_err(text)?;
    let mut rows = stmt.query([progress.offset]).map_err(text)?;
    let mut batch = Batch::default();
    let mut latest = progress.offset;
    while let Some(row) = rows.next().map_err(text)? {
        let id: String = row.get(0).map_err(text)?;
        let created: i64 = row.get(1).map_err(text)?;
        let updated: i64 = row.get(2).map_err(text)?;
        let data: String = row.get(3).map_err(text)?;
        latest = latest.max(updated);
        if let Ok(m) = serde_json::from_str::<Value>(&data) {
            opencode_message(scan, &mut batch, &id, created, &m).map_err(text)?;
        }
    }
    let progress = Progress {
        size,
        mtime,
        offset: latest,
        state: Value::Null,
    };
    scan.commit(Tool::OpenCode, &key, &progress, batch)
        .map_err(text)
}

fn opencode(env: &Env, scan: &mut Scan) -> Result<(), String> {
    let dir = opencode_dir(env);
    // 不同发布渠道用不同的文件名：opencode.db、opencode-stable.db……
    let dbs = files(&dir, 0, &|p| {
        name_of(p).starts_with("opencode") && has_ext(p, "db")
    });
    let mut result = each_file(dbs, |path| opencode_db(scan, path));
    let legacy = files(&dir.join("storage").join("message"), 1, &|p| {
        has_ext(p, "json")
    });
    let older = each_file(legacy, |path| {
        let key = format!("opencode:{}", path.display());
        read_whole(
            scan,
            Tool::OpenCode,
            &key,
            path,
            &mut |scan, batch, content| {
                let Ok(m) = serde_json::from_str::<Value>(content) else {
                    return Ok(());
                };
                let id = m["id"].as_str().unwrap_or_default().to_string();
                if id.is_empty() {
                    return Ok(());
                }
                opencode_message(scan, batch, &id, 0, &m)
            },
        )
    });
    if result.is_ok() {
        result = older;
    }
    result
}

// ---- Pi 和 OpenClaw：同一种 JSONL 记录格式（未实测）。 ----

fn pi_line(
    scan: &Scan,
    batch: &mut Batch,
    state: &mut Value,
    v: &Value,
    prefix: &str,
) -> rusqlite::Result<()> {
    match v["type"].as_str() {
        Some("model_change") => {
            if let Some(provider) = v["provider"].as_str() {
                state["provider"] = json!(provider);
            }
            if let Some(model) = v["modelId"].as_str().or_else(|| v["model"].as_str()) {
                state["model"] = json!(model);
            }
            Ok(())
        }
        Some("message") => {
            let m = &v["message"];
            let usage = &m["usage"];
            if m["role"] != "assistant" || !usage.is_object() {
                return Ok(());
            }
            let Some(ts) = iso(&v["timestamp"]).or_else(|| m["timestamp"].as_i64()) else {
                return Ok(());
            };
            let provider = m["provider"]
                .as_str()
                .or_else(|| state["provider"].as_str());
            let model = m["model"]
                .as_str()
                .or_else(|| state["model"].as_str())
                .unwrap_or_default();
            let counts = Counts {
                input: n(usage, "input"),
                cache_read: n(usage, "cacheRead"),
                cache_write: n(usage, "cacheWrite"),
                output: n(usage, "output"),
                requests: 1,
            };
            // 分叉会话会复制旧消息，换了会话 ID 也能对上。
            let key = v["id"]
                .as_str()
                .map(|id| format!("{prefix}:{id}:{ts}:{}:{}", counts.input, counts.output));
            let who = provider
                .map(|p| Who::Exact(p.to_string()))
                .unwrap_or(Who::Unknown);
            scan.add(batch, key, record(ts, model, who, counts))
        }
        _ => Ok(()),
    }
}

fn pi(env: &Env, scan: &mut Scan) -> Result<(), String> {
    let paths = files(&env.pi_agent_dir.join("sessions"), 3, &|p| {
        has_ext(p, "jsonl")
    });
    each_file(paths, |path| {
        let key = format!("pi:{}", path.display());
        read_lines(
            scan,
            Tool::Pi,
            &key,
            path,
            &["\"usage\"", "model_change"],
            &mut |scan, batch, state, v| pi_line(scan, batch, state, v, "p"),
        )
    })
}

fn openclaw_db(scan: &mut Scan, path: &Path) -> Result<(), String> {
    let key = format!("openclaw:{}", path.display());
    let (size, mtime) = meta(path).unwrap_or_default();
    let mut progress = scan.progress(&key).map_err(text)?;
    let conn = open_readonly(path)?;
    let mut stmt = conn
        .prepare("SELECT rowid, event_json FROM transcript_events WHERE rowid > ?1 AND event_json IS NOT NULL ORDER BY rowid")
        .map_err(text)?;
    let mut rows = stmt.query([progress.offset]).map_err(text)?;
    let mut state = match std::mem::take(&mut progress.state) {
        Value::Object(map) => Value::Object(map),
        _ => json!({}),
    };
    let mut batch = Batch::default();
    let mut last = progress.offset;
    while let Some(row) = rows.next().map_err(text)? {
        last = row.get(0).map_err(text)?;
        let event: String = row.get(1).map_err(text)?;
        if let Ok(v) = serde_json::from_str::<Value>(&event) {
            pi_line(scan, &mut batch, &mut state, &v, "w").map_err(text)?;
        }
    }
    let progress = Progress {
        size,
        mtime,
        offset: last,
        state,
    };
    scan.commit(Tool::OpenClaw, &key, &progress, batch)
        .map_err(text)
}

fn openclaw(env: &Env, scan: &mut Scan) -> Result<(), String> {
    let agents = env.openclaw_dir.join("agents");
    // 新版写进每个 agent 的 SQLite（只读 event_json，压缩的事件跳过），旧版是 sessions/*.jsonl；
    // 迁移后两边都有同一条，靠去重键只算一次。
    let dbs = files(&agents, 2, &|p| name_of(p) == "openclaw-agent.sqlite");
    let mut result = each_file(dbs, |path| openclaw_db(scan, path));
    let jsonl = files(&agents, 2, &|p| {
        has_ext(p, "jsonl")
            && p.parent()
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                == Some("sessions")
    });
    let legacy = each_file(jsonl, |path| {
        let key = format!("openclaw:{}", path.display());
        read_lines(
            scan,
            Tool::OpenClaw,
            &key,
            path,
            &["\"usage\"", "model_change"],
            &mut |scan, batch, state, v| pi_line(scan, batch, state, v, "w"),
        )
    });
    if result.is_ok() {
        result = legacy;
    }
    result
}

// ---- Hermes：`state.db` 的 sessions 表按会话累计（未实测）。 ----
//
// 每个会话记住上次算过的累计值，只加差值，算在会话开始那天。

fn hermes_db(scan: &mut Scan, path: &Path) -> Result<(), String> {
    let conn = open_readonly(path)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, model, billing_provider, started_at, COALESCE(message_count, 0),
                    COALESCE(input_tokens, 0), COALESCE(output_tokens, 0),
                    COALESCE(cache_read_tokens, 0), COALESCE(cache_write_tokens, 0)
             FROM sessions WHERE model IS NOT NULL AND TRIM(model) != ''",
        )
        .map_err(text)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, f64>(3)?,
                [
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ],
            ))
        })
        .map_err(text)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(text)?;
    for (id, model, provider, started, now) in rows {
        let key = format!("hermes:{}:{id}", path.display());
        let progress = scan.progress(&key).map_err(text)?;
        let before: Vec<i64> = (0..5)
            .map(|i| progress.state[i].as_i64().unwrap_or(0))
            .collect();
        if before == now {
            continue;
        }
        let d: Vec<i64> = (0..5).map(|i| (now[i] - before[i]).max(0)).collect();
        let counts = Counts {
            input: d[1],
            output: d[2],
            cache_read: d[3],
            cache_write: d[4],
            // message_count 包括用户消息，大约一半是模型回复。
            requests: (d[0] + 1) / 2,
        };
        let who = provider
            .filter(|p| !p.trim().is_empty())
            .map(Who::Exact)
            .unwrap_or(Who::Unknown);
        let mut batch = Batch::default();
        scan.add(
            &mut batch,
            None,
            record((started * 1000.0) as i64, &model, who, counts),
        )
        .map_err(text)?;
        let progress = Progress {
            size: 0,
            mtime: 0,
            offset: 0,
            state: json!(now),
        };
        scan.commit(Tool::Hermes, &key, &progress, batch)
            .map_err(text)?;
    }
    Ok(())
}

fn hermes(env: &Env, scan: &mut Scan) -> Result<(), String> {
    let mut dbs = vec![env.hermes_dir.join("state.db")];
    dbs.extend(files(&env.hermes_dir.join("profiles"), 1, &|p| {
        name_of(p) == "state.db"
    }));
    each_file(dbs.into_iter().filter(|p| p.is_file()).collect(), |path| {
        hermes_db(scan, path)
    })
}
