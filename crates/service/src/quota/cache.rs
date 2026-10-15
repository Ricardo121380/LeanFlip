//! 联网查到的额度存在数据目录的 `quota-cache.json`，重启后先显示上次的结果和时间。
//! 只有数字和来源标识（中转站只存 Key 的哈希），没有凭据。文件坏了就当没有缓存。

use std::collections::HashMap;

use leanflip_core::{fsutil, AppError, Env};
use serde::{Deserialize, Serialize};

use super::{Observation, QuotaSnapshot};

const FILE: &str = "quota-cache.json";
const VERSION: u32 = 1;
/// 超过这么久的结果对任何窗口都没用了。
const KEEP_MS: i64 = 30 * 86_400_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RelayEntry {
    pub key: String,
    pub snapshot: QuotaSnapshot,
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

pub(crate) fn load(env: &Env) -> Vec<Observation> {
    read(env).entries
}

pub(crate) fn load_relays(env: &Env) -> HashMap<String, QuotaSnapshot> {
    read(env)
        .relays
        .into_iter()
        .map(|e| (e.key, e.snapshot))
        .collect()
}

/// 用新结果替换同一来源的旧结果，顺便丢掉过期的。
pub(crate) fn save(
    env: &Env,
    fresh: Vec<Observation>,
    relays: Vec<RelayEntry>,
    now_ms: i64,
) -> Result<(), AppError> {
    if fresh.is_empty() && relays.is_empty() {
        return Ok(());
    }
    let mut cache = read(env);
    let recent = |snapshot: &QuotaSnapshot| now_ms - snapshot.fetched_at <= KEEP_MS;
    cache.entries.retain(|old| {
        recent(&old.snapshot) && !fresh.iter().any(|new| new.identity == old.identity)
    });
    cache.entries.extend(fresh);
    cache
        .relays
        .retain(|old| recent(&old.snapshot) && !relays.iter().any(|new| new.key == old.key));
    cache.relays.extend(relays);
    cache.version = VERSION;
    fsutil::atomic_write_private(&env.data_file(FILE), &fsutil::sorted_json_bytes(&cache)?)
}
