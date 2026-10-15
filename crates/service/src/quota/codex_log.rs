//! Codex 本机会话记录里的 `rate_limits`。
//!
//! 每个 `token_count` 事件都带一份服务端返回的限额快照。文件第一行的 `session_meta` 记着
//! 发起会话的身份：`creator_account_id` 是工作区，`creator_user_id` 是 `chatgpt_user_id`。
//! 只看最近改过的文件，每个文件只读开头一行和末尾一段，不做统计页那样的全量扫描。

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{parse_credits, CodexIdentity, Observation, QuotaOrigin, QuotaSnapshot, QuotaWindow};

/// 周窗口再多一天：更早的记录对哪个窗口都没用了。
const RECENT_MS: i64 = 8 * 86_400_000;
const MAX_FILES: usize = 200;
const TAIL_BYTES: u64 = 1 << 20;
/// 末尾是超长记录时，每个文件最多往前读这么多块。
const MAX_CHUNKS: u64 = 8;
/// Codex 主额度的 `limit_id`；其他 id 是附加额度，只在找不到主额度时用。
const MAIN_LIMIT: &str = "codex";

/// 每个身份最新的一份快照。读不了的文件跳过。
pub(crate) fn read(codex_dir: &Path, now_ms: i64) -> Vec<Observation> {
    read_with(codex_dir, now_ms, TAIL_BYTES)
}

pub(super) fn read_with(codex_dir: &Path, now_ms: i64, tail: u64) -> Vec<Observation> {
    let mut files = Vec::new();
    // sessions/年/月/日/*.jsonl；归档后文件搬到 archived_sessions/*.jsonl。
    collect(&codex_dir.join("sessions"), 3, &mut files);
    collect(&codex_dir.join("archived_sessions"), 0, &mut files);
    files.retain(|(_, modified)| now_ms - modified <= RECENT_MS);
    files.sort_by_key(|a| std::cmp::Reverse(a.1));
    files.truncate(MAX_FILES);

    let mut found: Vec<Observation> = Vec::new();
    for (path, _) in files {
        let Some(identity) = creator(&path) else {
            continue;
        };
        if found.iter().any(|f| f.identity == identity) {
            continue;
        }
        if let Some(snapshot) = last_rate_limits(&path, now_ms, tail) {
            found.push(Observation { identity, snapshot });
        }
    }
    found
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<(PathBuf, i64)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let path = entry.path();
        if meta.is_dir() {
            if depth > 0 {
                collect(&path, depth - 1, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            if let Some(modified) = meta.modified().ok().and_then(millis) {
                out.push((path, modified));
            }
        }
    }
}

fn millis(time: std::time::SystemTime) -> Option<i64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 第一行的 `session_meta`。旧版本没有身份字段，这样的会话认不出是谁的，跳过。
fn creator(path: &Path) -> Option<CodexIdentity> {
    let mut line = Vec::new();
    BufReader::new(File::open(path).ok()?)
        .read_until(b'\n', &mut line)
        .ok()?;
    let value: Value = serde_json::from_slice(&line).ok()?;
    if value["type"] != "session_meta" {
        return None;
    }
    let payload = &value["payload"];
    Some(CodexIdentity {
        workspace: text(&payload["creator_account_id"])?,
        user: text(&payload["creator_user_id"]),
    })
}

/// 从文件末尾往前一块一块找最后一条带 `rate_limits` 的 `token_count`。每轮对话都会写
/// 一条，看过大约一块的完整记录还没找到就不再找。比一块还长的记录（大段工具输出）不会
/// 是它，只跳过、不算进看过的量，所以末尾是这样的记录时会接着往前读，最多读
/// `MAX_CHUNKS` 块。
fn last_rate_limits(path: &Path, now_ms: i64, chunk: u64) -> Option<QuotaSnapshot> {
    let chunk = chunk.max(1);
    let mut file = File::open(path).ok()?;
    let mut end = file.metadata().ok()?.len();
    let floor = end.saturating_sub(chunk.saturating_mul(MAX_CHUNKS));
    // 上一块开头那行的后半截；`long` 表示那行太长，已经丢掉不看。
    let mut carry: Vec<u8> = Vec::new();
    let mut long = false;
    let mut looked = 0;
    let mut fallback = None;
    while end > floor && looked < chunk {
        let start = end.saturating_sub(chunk).max(floor);
        let mut bytes = vec![0; usize::try_from(end - start).ok()?];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut bytes).ok()?;
        bytes.append(&mut carry);
        let lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
        let last = lines.len() - 1;
        // 没读到文件开头时，第一行的前半截在更前面，留到下一块。
        let first = usize::from(start > 0);
        for (index, line) in lines.iter().enumerate().skip(first).rev() {
            if index == last && long {
                continue;
            }
            looked += line.len() as u64 + 1;
            let Some((snapshot, main)) = token_count(line, now_ms) else {
                continue;
            };
            if main {
                return Some(snapshot);
            }
            fallback.get_or_insert(snapshot);
        }
        if start > 0 {
            long = (long && last == 0) || lines[0].len() as u64 > chunk;
            if !long {
                carry = lines[0].to_vec();
            }
        }
        end = start;
    }
    fallback
}

/// 一行 `token_count` 里的额度，和它是不是主额度。
fn token_count(line: &[u8], now_ms: i64) -> Option<(QuotaSnapshot, bool)> {
    let line = std::str::from_utf8(line).ok()?;
    if !line.contains("\"rate_limits\"") {
        return None;
    }
    let value = serde_json::from_str::<Value>(line).ok()?;
    let payload = &value["payload"];
    if payload["type"] != "token_count" || !payload["rate_limits"].is_object() {
        return None;
    }
    let snapshot = parse(&payload["rate_limits"], &value["timestamp"], now_ms)?;
    let main = matches!(
        payload["rate_limits"]["limit_id"].as_str(),
        None | Some(MAIN_LIMIT)
    );
    Some((snapshot, main))
}

pub(super) fn parse(limits: &Value, timestamp: &Value, now_ms: i64) -> Option<QuotaSnapshot> {
    let fetched_at = timestamp
        .as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp_millis())?;
    // 先按记录当时的样子建快照，再按现在的时间看哪些窗口已经重置。
    let windows = ["primary", "secondary"]
        .iter()
        .filter_map(|key| window(&limits[*key], fetched_at))
        .collect::<Vec<_>>();
    let credits = parse_credits(&limits["credits"]);
    if windows.is_empty() && credits.is_none() {
        return None;
    }
    let mut snapshot = QuotaSnapshot::new(
        windows,
        credits,
        text(&limits["plan_type"]),
        fetched_at,
        QuotaOrigin::Local,
        !limits["rate_limit_reached_type"].is_null(),
    );
    snapshot.settle(now_ms);
    Some(snapshot)
}

fn window(value: &Value, at_ms: i64) -> Option<QuotaWindow> {
    let minutes = value["window_minutes"].as_i64().filter(|m| *m > 0)?;
    let used_percent = value["used_percent"].as_f64()?;
    let resets_at = value["resets_at"].as_i64();
    Some(QuotaWindow {
        minutes,
        used_percent,
        resets_at,
        reset: resets_at.is_some_and(|at| at.saturating_mul(1000) <= at_ms),
        used_amount: None,
        limit_amount: None,
    })
}
