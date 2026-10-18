//! 用量统计。LeanFlip 不经手请求，只能读各工具留在本机的会话记录：只取时间、模型、
//! 供应商和 token 数字，不读对话内容，按“日期 × 工具 × 供应商 × 模型”汇总进 `usage.db`。
//! 工具删掉原始记录（Claude Code、Gemini CLI 默认保留 30 天）后，汇总还在。
//!
//! 另记一份切换记录：每次切换、写入、移出之后，以及每次扫描时，对比当前状态追加变化。
//! 时间线和“按时间推算”的归属都靠它；记录开始之前的用量归不到具体供应商。
//!
//! 各家的“输入”口径不同（Codex、Gemini、Grok 的输入包含缓存命中），读进来时统一拆成
//! 未缓存输入、缓存读取、缓存写入、输出四项。

mod sources;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{Local, TimeZone};
use leanflip_core::store::StoreData;
use leanflip_core::{AppError, Env, Tool};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

pub use sources::source_statuses;

const DB_FILE: &str = "usage.db";
/// 报告覆盖的天数：最长 90 天的范围，加上用来对比的前一个 90 天。
pub const REPORT_DAYS: i64 = 180;
/// 去重键保留这么久。原始记录最多保留几个月，更早的键不会再遇到。
const SEEN_TTL_MS: i64 = 400 * 86_400_000;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS usage_daily (
    day TEXT NOT NULL,
    tool TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    attribution TEXT NOT NULL,
    input INTEGER NOT NULL DEFAULT 0,
    cache_read INTEGER NOT NULL DEFAULT 0,
    cache_write INTEGER NOT NULL DEFAULT 0,
    output INTEGER NOT NULL DEFAULT 0,
    requests INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, tool, provider, model, attribution)
);
CREATE TABLE IF NOT EXISTS switch_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    tool TEXT NOT NULL,
    provider TEXT NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS progress (
    key TEXT PRIMARY KEY,
    size INTEGER NOT NULL DEFAULT 0,
    mtime INTEGER NOT NULL DEFAULT 0,
    offset INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS message_counts (
    key TEXT PRIMARY KEY, ts INTEGER NOT NULL, record TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS seen (
    key TEXT PRIMARY KEY,
    ts INTEGER NOT NULL
);
"#;

/// 一天、一个工具、一个供应商、一个模型的用量。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub day: String,
    pub tool: Tool,
    /// LeanFlip 的供应商 ID；精确归属时也可能是客户端自己的名字（没交给 LeanFlip 管理）。
    /// 空字符串：归不到供应商。
    pub provider: String,
    pub model: String,
    pub attribution: Attribution,
    pub input: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub output: i64,
    pub requests: i64,
}

/// 用量怎么归到供应商的。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Attribution {
    /// 记录里自带供应商（OpenCode、Pi 等），或 Codex 的供应商 ID 只对应一个 LeanFlip 供应商。
    Exact,
    /// 记录里没有供应商，按切换记录推算当时用的是谁。
    Time,
    /// 会话明确记录了路由，但没有证据区分使用该路由的多张卡片。
    Route,
    /// 切换记录开始之前，归不到供应商。
    None,
}

impl Attribution {
    fn as_str(self) -> &'static str {
        match self {
            Attribution::Exact => "exact",
            Attribution::Time => "time",
            Attribution::None => "none",
            Attribution::Route => "route",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "exact" => Attribution::Exact,
            "time" => Attribution::Time,
            "route" => Attribution::Route,
            _ => Attribution::None,
        }
    }
}

/// 切换记录的一条：切换式工具是“当前供应商变成了谁”（`provider` 为空表示没有选择），
/// 并存式工具是“写入 / 移出”。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SwitchEvent {
    pub ts: i64,
    pub tool: Tool,
    pub provider: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SourceState {
    /// 读到了记录目录。
    Read,
    /// 本机没有这个工具的记录目录。
    Missing,
    /// 这个工具不在本机留用量记录。
    Unsupported,
    /// 读取出错，`detail` 是原因。
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    pub tool: Tool,
    pub state: SourceState,
    /// 格式在真实数据上验证过；没验证过的按公开资料接入，界面标“未实测”。
    pub tested: bool,
    /// 格式没有公开文档，可能随版本变化。
    pub experimental: bool,
    pub path: Option<String>,
    /// 工具默认保留原始记录的天数。
    pub retention_days: Option<u32>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    pub rows: Vec<UsageRow>,
    pub events: Vec<SwitchEvent>,
    pub sources: Vec<SourceStatus>,
    pub scanned_at: i64,
    /// Earliest locally retained usage day per tool, including days outside this report.
    pub coverage_start: HashMap<Tool, String>,
}

/// 四项 token 和请求次数。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Counts {
    pub input: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub output: i64,
    pub requests: i64,
}

impl Counts {
    fn is_empty(&self) -> bool {
        self.input == 0 && self.cache_read == 0 && self.cache_write == 0 && self.output == 0
    }
}

/// 记录里能看出的供应商线索。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Who {
    /// 记录里自带的供应商名。
    Exact(String),
    /// Codex 会话的 `model_provider`：只对应一个 LeanFlip 供应商时精确，否则按时间推算。
    Codex(String),
    /// 没有线索，按切换记录推算。
    Time,
    /// 没有线索，也不能按时间推算（并存式工具同时有多家）。
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Record {
    pub ts: i64,
    pub model: String,
    pub who: Who,
    pub counts: Counts,
}

/// 一个文件（或数据库）的读取进度。
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Progress {
    pub size: i64,
    pub mtime: i64,
    pub offset: i64,
    pub state: serde_json::Value,
}

/// 一个文件读出来、还没提交的结果。整个文件成功才提交，出错时什么都不留，下次重读。
#[derive(Default)]
pub(crate) struct Batch {
    pub records: Vec<Record>,
    keys: Vec<(String, i64)>,
    local: HashSet<String>,
    cumulative: HashMap<String, Record>,
    pub seed_only: bool,
}

/// 一次扫描：所有写入在同一个事务里，提交前崩溃不会留下半份进度。
pub(crate) struct Scan<'c> {
    tx: Transaction<'c>,
    records: Vec<(Tool, Record)>,
    read_bytes: u64,
}

impl Scan<'_> {
    pub(crate) fn progress(&self, key: &str) -> rusqlite::Result<Progress> {
        let row = self
            .tx
            .prepare_cached("SELECT size, mtime, offset, state FROM progress WHERE key = ?1")?
            .query_row([key], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .optional()?;
        Ok(row
            .map(|(size, mtime, offset, state)| Progress {
                size,
                mtime,
                offset,
                state: serde_json::from_str(&state).unwrap_or(serde_json::Value::Null),
            })
            .unwrap_or_default())
    }

    /// 这条记录之前没算过就放进 `batch`。没有去重键的记录直接放进去。
    pub(crate) fn add(
        &self,
        batch: &mut Batch,
        key: Option<String>,
        record: Record,
    ) -> rusqlite::Result<()> {
        if record.counts.is_empty() {
            return Ok(());
        }
        if let Some(key) = key {
            if batch.local.contains(&key) {
                return Ok(());
            }
            let known = self
                .tx
                .prepare_cached("SELECT 1 FROM seen WHERE key = ?1")?
                .exists([&key])?;
            if known {
                return Ok(());
            }
            batch.local.insert(key.clone());
            batch.keys.push((key, record.ts));
        }
        if !batch.seed_only {
            batch.records.push(record);
        }
        Ok(())
    }

    /// Streaming message updates contribute only the increase since the last observation.
    pub(crate) fn add_cumulative(
        &self,
        batch: &mut Batch,
        key: Option<String>,
        record: Record,
    ) -> rusqlite::Result<()> {
        let Some(key) = key else {
            return self.add(batch, None, record);
        };
        let stored = self
            .tx
            .prepare_cached("SELECT record FROM message_counts WHERE key = ?1")?
            .query_row([&key], |r| r.get::<_, String>(0))
            .optional()?;
        let previous = batch
            .cumulative
            .get(&key)
            .cloned()
            .or_else(|| stored.and_then(|s| serde_json::from_str(&s).ok()));
        let legacy = previous.is_none()
            && self
                .tx
                .prepare_cached("SELECT 1 FROM seen WHERE key = ?1")?
                .exists([&key])?;
        let mut next = previous.clone().unwrap_or_else(|| record.clone());
        let before = previous.as_ref().map(|r| r.counts).unwrap_or_default();
        let max = |a: i64, b: i64| a.max(b);
        next.counts = Counts {
            input: max(before.input, record.counts.input),
            cache_read: max(before.cache_read, record.counts.cache_read),
            cache_write: max(before.cache_write, record.counts.cache_write),
            output: max(before.output, record.counts.output),
            requests: max(before.requests, record.counts.requests),
        };
        let mut delta = next.clone();
        delta.counts = Counts {
            input: next.counts.input - before.input,
            cache_read: next.counts.cache_read - before.cache_read,
            cache_write: next.counts.cache_write - before.cache_write,
            output: next.counts.output - before.output,
            requests: next.counts.requests - before.requests,
        };
        // Legacy versions counted the first occurrence. A one-time rescan reconstructs that baseline.
        if !legacy && !delta.counts.is_empty() {
            batch.records.push(delta);
        }
        batch.keys.push((key.clone(), next.ts));
        batch.cumulative.insert(key, next);
        Ok(())
    }

    pub(crate) fn commit(
        &mut self,
        tool: Tool,
        key: &str,
        progress: &Progress,
        batch: Batch,
    ) -> rusqlite::Result<()> {
        self.tx
            .prepare_cached(
                "INSERT INTO progress (key, size, mtime, offset, state) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(key) DO UPDATE SET size = ?2, mtime = ?3, offset = ?4, state = ?5",
            )?
            .execute(params![
                key,
                progress.size,
                progress.mtime,
                progress.offset,
                if progress.state.is_null() {
                    String::new()
                } else {
                    progress.state.to_string()
                }
            ])?;
        let mut insert = self
            .tx
            .prepare_cached("INSERT OR IGNORE INTO seen (key, ts) VALUES (?1, ?2)")?;
        for (key, ts) in &batch.keys {
            insert.execute(params![key, ts])?;
        }
        for (key, record) in &batch.cumulative {
            self.tx.execute("INSERT INTO message_counts (key, ts, record) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET record = ?3",
                params![key, record.ts, serde_json::to_string(record).expect("usage record")])?;
        }
        drop(insert);
        self.records
            .extend(batch.records.into_iter().map(|record| (tool, record)));
        Ok(())
    }
}

fn open(env: &Env) -> Result<Connection, AppError> {
    std::fs::create_dir_all(&env.data_dir).map_err(|e| AppError::io(&env.data_dir, e))?;
    let path = env.data_file(DB_FILE);
    let conn = Connection::open(&path).map_err(|e| db_error(&path, e))?;
    conn.busy_timeout(std::time::Duration::from_secs(10))
        .and_then(|()| conn.execute_batch(SCHEMA))
        .map_err(|e| db_error(&path, e))?;
    Ok(conn)
}

fn db_error(path: &Path, error: rusqlite::Error) -> AppError {
    AppError::Message(format!("{}: {error}", path.display()))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 本地日期 `YYYY-MM-DD`。
pub(crate) fn local_day(ts: i64) -> String {
    Local
        .timestamp_millis_opt(ts)
        .earliest()
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// 当前状态：切换式工具是当前供应商（没有为空），并存式工具是写进客户端的条目。
/// 并存式工具的配置读不出来时为 `None`，这次不记它。
fn current_state(
    data: &StoreData,
    live: &dyn Fn(Tool) -> Option<Vec<String>>,
) -> Vec<(Tool, Option<Vec<String>>)> {
    Tool::ALL
        .iter()
        .map(|&tool| {
            let state = if tool.is_coexist() {
                live(tool)
            } else {
                Some(
                    data.current(tool)
                        .map(|p| vec![p.id.clone()])
                        .unwrap_or_default(),
                )
            };
            (tool, state)
        })
        .collect()
}

fn load_events(conn: &Connection) -> rusqlite::Result<Vec<SwitchEvent>> {
    let mut stmt =
        conn.prepare("SELECT ts, tool, provider, name, enabled FROM switch_log ORDER BY ts, id")?;
    let rows = stmt.query_map([], |row| {
        let tool: String = row.get(1)?;
        Ok((
            row.get(0)?,
            tool,
            row.get(2)?,
            row.get(3)?,
            row.get::<_, i64>(4)? != 0,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (ts, tool, provider, name, enabled) = row?;
        if let Ok(tool) = tool.parse() {
            out.push(SwitchEvent {
                ts,
                tool,
                provider,
                name,
                enabled,
            });
        }
    }
    Ok(out)
}

/// 把当前状态和切换记录里的最后状态比较，追加变化。
fn note(
    conn: &Connection,
    data: &StoreData,
    live: &dyn Fn(Tool) -> Option<Vec<String>>,
    ts: i64,
) -> rusqlite::Result<()> {
    let events = load_events(conn)?;
    let name_of = |tool: Tool, id: &str| {
        data.provider(tool, id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    let mut insert = conn.prepare(
        "INSERT INTO switch_log (ts, tool, provider, name, enabled) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for (tool, state) in current_state(data, live) {
        let Some(now) = state else { continue };
        let mut mine = events.iter().filter(|e| e.tool == tool);
        if tool.is_coexist() {
            let mut before: Vec<String> = Vec::new();
            for e in mine {
                before.retain(|id| id != &e.provider);
                if e.enabled {
                    before.push(e.provider.clone());
                }
            }
            for id in now.iter().filter(|id| !before.contains(id)) {
                insert.execute(params![ts, tool.as_str(), id, name_of(tool, id), 1])?;
            }
            for id in before.iter().filter(|id| !now.contains(id)) {
                insert.execute(params![ts, tool.as_str(), id, name_of(tool, id), 0])?;
            }
        } else {
            let current = now.first().cloned().unwrap_or_default();
            match mine.next_back() {
                Some(last) if last.provider == current => {}
                // 从来没选过供应商的工具不记“没有选择”。
                None if current.is_empty() => {}
                _ => {
                    let name = if current.is_empty() {
                        String::new()
                    } else {
                        name_of(tool, &current)
                    };
                    insert.execute(params![ts, tool.as_str(), current, name, 1])?;
                }
            }
        }
    }
    Ok(())
}

/// 切换、写入、移出之后调用：记下状态变化。失败不影响操作本身。
pub fn record_state(
    env: &Env,
    data: &StoreData,
    live: &dyn Fn(Tool) -> Option<Vec<String>>,
) -> Result<(), AppError> {
    let conn = open(env)?;
    note(&conn, data, live, now_ms()).map_err(|e| db_error(&env.data_file(DB_FILE), e))
}

/// Codex 的 `model_provider` → 用它的 LeanFlip 供应商。
fn codex_providers(data: &StoreData) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for provider in data.providers(Tool::Codex) {
        let config = provider
            .settings_config
            .get("config")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let id = if provider.is_official_category() {
            "openai".to_string()
        } else {
            config
                .parse::<toml_edit::DocumentMut>()
                .ok()
                .and_then(|doc| {
                    doc.get("model_provider")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "openai".to_string())
        };
        map.entry(id).or_default().push(provider.id.clone());
    }
    map
}

/// 记录 → (供应商, 归属方式)。
fn resolve(
    tool: Tool,
    record: &Record,
    events: &[SwitchEvent],
    codex: &HashMap<String, Vec<String>>,
) -> (String, Attribution) {
    let by_time = || {
        if tool.is_coexist() {
            return (String::new(), Attribution::None);
        }
        match events.iter().rfind(|e| e.tool == tool && e.ts <= record.ts) {
            Some(e) => (e.provider.clone(), Attribution::Time),
            None => (String::new(), Attribution::None),
        }
    };
    match &record.who {
        Who::Exact(name) => (name.clone(), Attribution::Exact),
        Who::Codex(raw) => {
            let candidates = codex.get(raw).map(Vec::as_slice).unwrap_or_default();
            if let [only] = candidates {
                return (only.clone(), Attribution::Exact);
            }
            let (selected, _) = by_time();
            if candidates.contains(&selected) {
                (selected, Attribution::Time)
            } else {
                // The route is known even before LeanFlip started recording switches.
                // Do not assign it to an arbitrary card or a conflicting current route.
                (format!("codex-route:{raw}"), Attribution::Route)
            }
        }
        Who::Time => by_time(),
        Who::Unknown => (String::new(), Attribution::None),
    }
}

/// 扫描新增的记录、记下当前状态，返回最近 [`REPORT_DAYS`] 天的汇总。
pub fn load(
    env: &Env,
    data: &StoreData,
    live: &dyn Fn(Tool) -> Option<Vec<String>>,
) -> Result<UsageReport, AppError> {
    let path = env.data_file(DB_FILE);
    let fail = |e| db_error(&path, e);
    let mut conn = open(env)?;
    let now = now_ms();
    note(&conn, data, live, now).map_err(fail)?;
    let events = load_events(&conn).map_err(fail)?;
    let codex = codex_providers(data);

    // 扫描在一个事务里完成，事务结束后再读汇总。
    let sources = {
        let mut scan = Scan {
            tx: conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(fail)?,
            records: Vec::new(),
            read_bytes: 0,
        };
        let found = sources::scan_all(env, &mut scan);
        let mut totals: HashMap<(String, Tool, String, String, Attribution), Counts> =
            HashMap::new();
        for (tool, record) in std::mem::take(&mut scan.records) {
            let (provider, attribution) = resolve(tool, &record, &events, &codex);
            let entry = totals
                .entry((
                    local_day(record.ts),
                    tool,
                    provider,
                    record.model.clone(),
                    attribution,
                ))
                .or_default();
            entry.input += record.counts.input;
            entry.cache_read += record.counts.cache_read;
            entry.cache_write += record.counts.cache_write;
            entry.output += record.counts.output;
            entry.requests += record.counts.requests;
        }
        {
            let mut upsert = scan
            .tx
            .prepare(
                "INSERT INTO usage_daily (day, tool, provider, model, attribution, input, cache_read, cache_write, output, requests)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(day, tool, provider, model, attribution) DO UPDATE SET
                    input = input + ?6, cache_read = cache_read + ?7, cache_write = cache_write + ?8,
                    output = output + ?9, requests = requests + ?10",
            )
            .map_err(fail)?;
            for ((day, tool, provider, model, attribution), c) in &totals {
                upsert
                    .execute(params![
                        day,
                        tool.as_str(),
                        provider,
                        model,
                        attribution.as_str(),
                        c.input,
                        c.cache_read,
                        c.cache_write,
                        c.output,
                        c.requests
                    ])
                    .map_err(fail)?;
            }
            scan.tx
                .execute(
                    "DELETE FROM message_counts WHERE ts < ?1",
                    [now - SEEN_TTL_MS],
                )
                .map_err(fail)?;
            scan.tx
                .execute("DELETE FROM seen WHERE ts < ?1", [now - SEEN_TTL_MS])
                .map_err(fail)?;
        }
        reconcile_codex_routes(env, &scan.tx, &events, &codex).map_err(fail)?;
        scan.tx.commit().map_err(fail)?;
        found
    };

    let cutoff = local_day(now - (REPORT_DAYS - 1) * 86_400_000);
    let mut stmt = conn
        .prepare(
            "SELECT day, tool, provider, model, attribution, input, cache_read, cache_write, output, requests
             FROM usage_daily WHERE day >= ?1 ORDER BY day, tool, provider, model",
        )
        .map_err(fail)?;
    let rows = stmt
        .query_map([cutoff], |row| {
            let tool: String = row.get(1)?;
            let attribution: String = row.get(4)?;
            Ok((
                tool,
                UsageRow {
                    day: row.get(0)?,
                    tool: Tool::Claude,
                    provider: row.get(2)?,
                    model: row.get(3)?,
                    attribution: Attribution::parse(&attribution),
                    input: row.get(5)?,
                    cache_read: row.get(6)?,
                    cache_write: row.get(7)?,
                    output: row.get(8)?,
                    requests: row.get(9)?,
                },
            ))
        })
        .map_err(fail)?
        .filter_map(Result::ok)
        .filter_map(|(tool, mut row)| {
            tool.parse().ok().map(|tool| {
                row.tool = tool;
                row
            })
        })
        .collect();
    let coverage_start = conn
        .prepare("SELECT tool, MIN(day) FROM usage_daily GROUP BY tool")
        .map_err(fail)?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(fail)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(fail)?
        .into_iter()
        .filter_map(|(tool, day)| tool.parse().ok().map(|tool| (tool, day)))
        .collect();
    Ok(UsageReport {
        coverage_start,
        rows,
        events,
        sources,
        scanned_at: now,
    })
}

// Reattribute legacy Codex totals only when available source records account for
// ALL five counters of a day/model exactly. Incomplete/deleted history stays intact.
// This one-time replay never changes totals or resets the incremental scanner.
fn reconcile_codex_routes(
    env: &Env,
    tx: &Transaction<'_>,
    events: &[SwitchEvent],
    codex: &HashMap<String, Vec<String>>,
) -> rusqlite::Result<()> {
    const MARKER: &str = "migration:codex-routes-v1";
    if tx
        .query_row(
            "SELECT 1 FROM progress WHERE key = ?1",
            [MARKER],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        return Ok(());
    }
    let mut scratch = Connection::open_in_memory()?;
    scratch.execute_batch(SCHEMA)?;
    let mut replay = Scan {
        tx: scratch.transaction()?,
        records: vec![],
        read_bytes: 0,
    };
    if sources::codex(env, &mut replay).is_err() {
        return Ok(());
    }
    let mut days: HashMap<(String, String), HashMap<(String, Attribution), Counts>> =
        HashMap::new();
    for (_, record) in replay.records {
        let who = resolve(Tool::Codex, &record, events, codex);
        let count = days
            .entry((local_day(record.ts), record.model))
            .or_default()
            .entry(who)
            .or_default();
        count.input += record.counts.input;
        count.cache_read += record.counts.cache_read;
        count.cache_write += record.counts.cache_write;
        count.output += record.counts.output;
        count.requests += record.counts.requests;
    }
    for ((day, model), groups) in days {
        let found = tx.query_row("SELECT COALESCE(SUM(input),0), COALESCE(SUM(cache_read),0), COALESCE(SUM(cache_write),0), COALESCE(SUM(output),0), COALESCE(SUM(requests),0) FROM usage_daily WHERE tool='codex' AND day=?1 AND model=?2", params![day, model], |r| Ok(Counts { input: r.get(0)?, cache_read:r.get(1)?,cache_write:r.get(2)?,output:r.get(3)?,requests:r.get(4)? }))?;
        let mut total = Counts::default();
        for count in groups.values() {
            total.input += count.input;
            total.cache_read += count.cache_read;
            total.cache_write += count.cache_write;
            total.output += count.output;
            total.requests += count.requests;
        }
        if total != found {
            continue;
        }
        tx.execute(
            "DELETE FROM usage_daily WHERE tool='codex' AND day=?1 AND model=?2",
            params![day, model],
        )?;
        for ((provider, attribution), count) in groups {
            tx.execute("INSERT INTO usage_daily (day,tool,provider,model,attribution,input,cache_read,cache_write,output,requests) VALUES (?1,'codex',?2,?3,?4,?5,?6,?7,?8,?9)", params![day,provider,model,attribution.as_str(),count.input,count.cache_read,count.cache_write,count.output,count.requests])?;
        }
    }
    tx.execute(
        "INSERT INTO progress (key,state) VALUES (?1,'done')",
        [MARKER],
    )?;
    Ok(())
}
