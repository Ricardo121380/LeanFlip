//! 联网查到的额度存在数据目录的 `quota-cache.json`，重启后先显示上次的结果和时间。
//! 只有数字和身份标识，没有凭据。文件坏了就当没有缓存。

use leanflip_core::{fsutil, AppError, Env};
use serde::{Deserialize, Serialize};

use super::Observation;

const FILE: &str = "quota-cache.json";
const VERSION: u32 = 1;
/// 超过这么久的结果对任何窗口都没用了。
const KEEP_MS: i64 = 30 * 86_400_000;

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    version: u32,
    #[serde(default)]
    entries: Vec<Observation>,
}

pub(crate) fn load(env: &Env) -> Vec<Observation> {
    let Ok(bytes) = std::fs::read(env.data_file(FILE)) else {
        return Vec::new();
    };
    match serde_json::from_slice::<Cache>(&bytes) {
        Ok(cache) if cache.version == VERSION => cache.entries,
        _ => Vec::new(),
    }
}

/// 用新结果替换同一身份的旧结果，顺便丢掉过期的。
pub(crate) fn save(env: &Env, fresh: Vec<Observation>, now_ms: i64) -> Result<(), AppError> {
    if fresh.is_empty() {
        return Ok(());
    }
    let mut entries = load(env);
    entries.retain(|old| {
        now_ms - old.snapshot.fetched_at <= KEEP_MS
            && !fresh.iter().any(|new| new.identity == old.identity)
    });
    entries.extend(fresh);
    let cache = Cache {
        version: VERSION,
        entries,
    };
    fsutil::atomic_write_private(&env.data_file(FILE), &fsutil::sorted_json_bytes(&cache)?)
}
