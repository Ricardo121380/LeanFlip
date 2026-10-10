//! 供应商库：`<data_dir>/providers.json`（默认 `~/.cc-switch-lite/providers.json`）。
//!
//! 为什么用 JSON 文件而不是 SQLite：
//! - 数据量很小（每个工具十几张卡），整份读写的成本可以忽略；
//! - 人能直接看、能 diff、能手工修，出问题时比数据库好排查；
//! - 不引入 C 依赖（rusqlite 要编 SQLite），Tauri 跨平台打包更省事；
//! - 写入走和客户端文件一样的原子写（临时文件 + rename，0600）。
//!
//! 代价：没有跨进程的事务。进程内用一把锁串行读改写；同一台机器上同时开两个
//! ccswitch-lite 进程改库时，后写的会覆盖先写的（客户端文件本身有 hash 校验，不受影响）。
//! 真需要时再换 SQLite，`Store` 的接口不变。
//!
//! 文件坏了（解析失败）不会被自动覆盖：这是用户数据，报错让用户处理。

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::env::Env;
use crate::error::AppError;
use crate::fsutil::atomic_write_private;
use crate::provider::{Provider, Tool};

pub const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreData {
    #[serde(default = "store_version")]
    pub version: u32,
    #[serde(default)]
    pub apps: BTreeMap<String, AppProviders>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn store_version() -> u32 {
    STORE_VERSION
}

impl Default for StoreData {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            apps: BTreeMap::new(),
            settings: Settings::default(),
            extra: Map::new(),
        }
    }
}

/// 一个工具的供应商列表和当前指针（每个工具只有一个当前供应商）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppProviders {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(default)]
    pub providers: Vec<Provider>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl AppProviders {
    pub fn get(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|provider| provider.id == id)
    }

    pub fn current_provider(&self) -> Option<&Provider> {
        self.current.as_deref().and_then(|id| self.get(id))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// 切到第三方时保留 Codex 的 ChatGPT 登录（`auth.json` 原样不动）。默认关闭，和
    /// cc-switch 一致：关闭时登录先存进登录暂存再删掉 `auth.json`，切回官方卡时还原。
    #[serde(default)]
    pub preserve_codex_official_auth_on_switch: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl StoreData {
    pub fn app(&self, tool: Tool) -> Option<&AppProviders> {
        self.apps.get(tool.as_str())
    }

    pub fn app_mut(&mut self, tool: Tool) -> &mut AppProviders {
        self.apps.entry(tool.as_str().to_string()).or_default()
    }

    pub fn providers(&self, tool: Tool) -> &[Provider] {
        self.app(tool)
            .map(|app| app.providers.as_slice())
            .unwrap_or(&[])
    }

    pub fn provider(&self, tool: Tool, id: &str) -> Option<&Provider> {
        self.app(tool).and_then(|app| app.get(id))
    }

    pub fn current(&self, tool: Tool) -> Option<&Provider> {
        self.app(tool).and_then(AppProviders::current_provider)
    }
}

fn store_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

/// 读供应商库。文件不存在时是空库。
pub fn load(env: &Env) -> Result<StoreData, AppError> {
    let _guard = store_lock().lock().unwrap_or_else(|e| e.into_inner());
    // 数据目录还不存在时库一定是空的，不为了读去建目录。
    let _file = env
        .data_dir
        .is_dir()
        .then(|| crate::live::engine::lock_file(&store_lock_path(env), false));
    load_unlocked(env)
}

/// 跨进程的库锁：读加共享锁、读改写加排他锁（两个 ccswitch-lite 进程同时改库时，
/// 后一个等前一个写完再读，不会互相覆盖）。
fn store_lock_path(env: &Env) -> std::path::PathBuf {
    env.data_dir.join("providers.json.lock")
}

fn load_unlocked(env: &Env) -> Result<StoreData, AppError> {
    let path = env.store_path();
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(StoreData::default()),
        Err(source) => return Err(AppError::io(&path, source)),
    };
    serde_json::from_slice(&bytes).map_err(|source| AppError::json(&path, source))
}

fn save_unlocked(env: &Env, data: &StoreData) -> Result<(), AppError> {
    let bytes =
        serde_json::to_vec_pretty(data).map_err(|source| AppError::JsonSerialize { source })?;
    atomic_write_private(&env.store_path(), &bytes)
}

/// 读改写供应商库（进程内串行）。`change` 返回错误时什么都不写。
pub fn update<R>(
    env: &Env,
    change: impl FnOnce(&mut StoreData) -> Result<R, AppError>,
) -> Result<R, AppError> {
    let _guard = store_lock().lock().unwrap_or_else(|e| e.into_inner());
    let _file = crate::live::engine::lock_file(&store_lock_path(env), true);
    let mut data = load_unlocked(env)?;
    let result = change(&mut data)?;
    save_unlocked(env, &data)?;
    Ok(result)
}

/// 落定当前指针。可以重复执行（崩溃恢复会再跑一次）；指向的卡已经被删掉时照样记下，
/// 读的时候 [`AppProviders::current_provider`] 会是 `None`。
pub fn set_current(env: &Env, tool: Tool, id: &str) -> Result<(), AppError> {
    update(env, |data| {
        let app = data.app_mut(tool);
        if app.current.as_deref() != Some(id) {
            app.current = Some(id.to_string());
        }
        Ok(())
    })
}

/// 新增一张卡。id 重复时报错。
pub fn add_provider(env: &Env, tool: Tool, provider: Provider) -> Result<(), AppError> {
    update(env, |data| {
        let app = data.app_mut(tool);
        if app.get(&provider.id).is_some() {
            return Err(AppError::InvalidInput(format!(
                "{tool} 已经有 id 为 {} 的供应商",
                provider.id
            )));
        }
        app.providers.push(provider);
        Ok(())
    })
}

/// 删除一张卡。当前供应商不能删（先切到别的卡）。
pub fn remove_provider(env: &Env, tool: Tool, id: &str) -> Result<Provider, AppError> {
    update(env, |data| {
        let app = data.app_mut(tool);
        if app.current.as_deref() == Some(id) {
            return Err(AppError::InvalidInput(format!(
                "{id} 是 {tool} 的当前供应商，先切到别的供应商再删除"
            )));
        }
        let index = app
            .providers
            .iter()
            .position(|provider| provider.id == id)
            .ok_or_else(|| AppError::NotFound(format!("{tool} 供应商 {id}")))?;
        Ok(app.providers.remove(index))
    })
}

/// 替换一张卡的内容（id 不变），返回旧版本。
pub fn replace_provider(env: &Env, tool: Tool, provider: Provider) -> Result<Provider, AppError> {
    update(env, |data| {
        let app = data.app_mut(tool);
        let slot = app
            .providers
            .iter_mut()
            .find(|existing| existing.id == provider.id)
            .ok_or_else(|| AppError::NotFound(format!("{tool} 供应商 {}", provider.id)))?;
        Ok(std::mem::replace(slot, provider))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env() -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        (dir, env)
    }

    #[test]
    fn crud_and_pointer() {
        let (_dir, env) = env();
        assert_eq!(load(&env).unwrap(), StoreData::default());
        add_provider(&env, Tool::Claude, Provider::new("a", "A", json!({}))).unwrap();
        add_provider(&env, Tool::Claude, Provider::new("b", "B", json!({}))).unwrap();
        assert!(add_provider(&env, Tool::Claude, Provider::new("a", "A2", json!({}))).is_err());
        set_current(&env, Tool::Claude, "a").unwrap();
        assert!(remove_provider(&env, Tool::Claude, "a").is_err());
        remove_provider(&env, Tool::Claude, "b").unwrap();
        let data = load(&env).unwrap();
        assert_eq!(data.current(Tool::Claude).unwrap().id, "a");
        assert_eq!(data.providers(Tool::Claude).len(), 1);
        assert!(data.providers(Tool::Codex).is_empty());
        assert!(!data.settings.preserve_codex_official_auth_on_switch);
    }

    #[cfg(unix)]
    #[test]
    fn store_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, env) = env();
        set_current(&env, Tool::Codex, "x").unwrap();
        let mode = fs::metadata(env.store_path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_corrupt_store_is_reported_not_overwritten() {
        let (_dir, env) = env();
        fs::create_dir_all(&env.data_dir).unwrap();
        fs::write(env.store_path(), "{ broken").unwrap();
        assert!(load(&env).is_err());
        assert!(set_current(&env, Tool::Claude, "a").is_err());
        assert_eq!(fs::read_to_string(env.store_path()).unwrap(), "{ broken");
    }

    #[test]
    fn unknown_fields_survive() {
        let (_dir, env) = env();
        fs::create_dir_all(&env.data_dir).unwrap();
        fs::write(
            env.store_path(),
            r#"{"version":1,"future":1,"apps":{"gemini":{"current":"g","providers":[]}},"settings":{"other":true}}"#,
        )
        .unwrap();
        set_current(&env, Tool::Claude, "a").unwrap();
        let raw: Value = serde_json::from_slice(&fs::read(env.store_path()).unwrap()).unwrap();
        assert_eq!(raw["future"], 1);
        assert_eq!(raw["apps"]["gemini"]["current"], "g");
        assert_eq!(raw["settings"]["other"], true);
        assert_eq!(raw["apps"]["claude"]["current"], "a");
    }
}
