//! 联网查到的额度存在数据目录的 `quota-cache.json`，重启后先显示上次的结果和时间。
//! 只有数字和来源标识（中转站只存 Key 的哈希），没有凭据。文件坏了就当没有缓存。

use std::collections::HashMap;

use leanflip_core::{fsutil, AppError, Env};
use serde::{Deserialize, Serialize};

use super::{Observation, QuotaSnapshot};

const FILE: &str = "quota-cache.json";
const VERSION: u32 = 1;
const LOCK: &str = "quota-cache";
/// 超过这么久的结果对任何窗口都没用了。
const KEEP_MS: i64 = 30 * 86_400_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RelayEntry {
    pub key: String,
    pub snapshot: QuotaSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<super::relay::RelayAdapter>,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    version: u32,
    /// Codex 托管账号，按 ChatGPT 身份。
    #[serde(default)]
    entries: Vec<Observation>,
    /// 中转站，按来源标识。
    #[serde(default)]
    relays: Vec<RelayEntry>,
}

fn read(env: &Env) -> Cache {
    let Ok(bytes) = std::fs::read(env.data_file(FILE)) else {
        return Cache::default();
    };
    match serde_json::from_slice::<Cache>(&bytes) {
        Ok(cache) if cache.version == VERSION => cache,
        _ => Cache::default(),
    }
}

fn recent(snapshot: &QuotaSnapshot, now_ms: i64) -> bool {
    now_ms - snapshot.fetched_at <= KEEP_MS
}

/// 读出来的结果按现在的时间重新看窗口是否已重置，太旧的不要。
fn current(mut snapshot: QuotaSnapshot, now_ms: i64) -> Option<QuotaSnapshot> {
    recent(&snapshot, now_ms).then(|| {
        snapshot.settle(now_ms);
        snapshot
    })
}

pub(crate) fn load(env: &Env, now_ms: i64) -> Vec<Observation> {
    read(env)
        .entries
        .into_iter()
        .filter_map(|o| {
            Some(Observation {
                snapshot: current(o.snapshot, now_ms)?,
                identity: o.identity,
            })
        })
        .collect()
}

pub(crate) fn load_relays(env: &Env, now_ms: i64) -> HashMap<String, QuotaSnapshot> {
    load_relay_entries(env, now_ms)
        .into_iter()
        .map(|(key, entry)| (key, entry.snapshot))
        .collect()
}

pub(crate) fn load_relay_entries(env: &Env, now_ms: i64) -> HashMap<String, RelayEntry> {
    read(env)
        .relays
        .into_iter()
        .filter_map(|mut e| {
            e.snapshot = current(e.snapshot, now_ms)?;
            Some((e.key.clone(), e))
        })
        .collect()
}

/// 用新结果替换同一来源的旧结果，顺便丢掉过期的。后台自动刷新和用户点的查询可能同时
/// 写，读改写整个过程拿着写锁，免得后写的覆盖掉先写的。
pub(crate) fn save(
    env: &Env,
    fresh: &[Observation],
    relays: &[RelayEntry],
    now_ms: i64,
) -> Result<(), AppError> {
    if fresh.is_empty() && relays.is_empty() {
        return Ok(());
    }
    let _guard = env.device_store().lock_app(LOCK)?;
    let mut cache = read(env);
    cache.entries.retain(|old| {
        recent(&old.snapshot, now_ms) && !fresh.iter().any(|new| new.identity == old.identity)
    });
    cache.entries.extend_from_slice(fresh);
    cache.relays.retain(|old| {
        recent(&old.snapshot, now_ms) && !relays.iter().any(|new| new.key == old.key)
    });
    cache.relays.extend_from_slice(relays);
    cache.version = VERSION;
    fsutil::atomic_write_private(&env.data_file(FILE), &fsutil::sorted_json_bytes(&cache)?)
}
