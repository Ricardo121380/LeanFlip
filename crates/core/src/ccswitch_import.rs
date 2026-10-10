//! 从 cc-switch（`~/.cc-switch`）只读导入供应商卡和托管 ChatGPT 账号。
//!
//! - 卡：优先读 `cc-switch.db`（SQLite，`providers` 表，只读打开），没有数据库时读旧版
//!   `config.json`（v2：`{ "<app>": { "providers": { id: {...} } } }`）。
//! - 账号：`codex_oauth_auth.json`（同格式 v2），id 不变，卡上的绑定照样能用。
//!
//! 绝不写 cc-switch 的任何文件。不导入「当前供应商」：ccswitch-lite 的指针跟着 live 走。
//! 需要本地路由的卡（非 Anthropic 格式、Copilot / ChatGPT 反代等）跳过并说明原因；
//! Codex 卡里从 `auth.json` 抄来的 ChatGPT 登录不导入（登录只认 live 和登录暂存）。

use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::codex::oauth::CodexOAuthManager;
use crate::env::Env;
use crate::error::AppError;
use crate::provider::{Provider, Tool};
use crate::store;

pub const DB_FILENAME: &str = "cc-switch.db";
pub const LEGACY_CONFIG: &str = "config.json";
pub const ACCOUNTS_FILENAME: &str = "codex_oauth_auth.json";

pub fn default_dir(env: &Env) -> PathBuf {
    env.home.join(".cc-switch")
}

/// 扫描出的一张卡。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub tool: Tool,
    pub provider: Provider,
    /// 有值表示不会导入（原因）。
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub source: Option<PathBuf>,
    pub candidates: Vec<Candidate>,
    pub accounts: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// (工具, id)
    pub created: Vec<(Tool, String)>,
    pub skipped: Vec<(Tool, String, String)>,
    pub accounts_added: Vec<String>,
}

/// 只读扫描。`dir` 不存在时返回空结果。
pub fn scan(env: &Env, dir: &Path) -> Result<Scan, AppError> {
    let db = dir.join(DB_FILENAME);
    let legacy = dir.join(LEGACY_CONFIG);
    let (source, rows) = if db.is_file() {
        (Some(db.clone()), read_db(&db)?)
    } else if legacy.is_file() {
        (Some(legacy.clone()), read_legacy(&legacy)?)
    } else {
        (None, Vec::new())
    };
    let data = store::load(env)?;
    let candidates = rows
        .into_iter()
        .map(|(tool, provider)| {
            let skip_reason = skip_reason(&data, tool, &provider);
            Candidate {
                tool,
                provider,
                skip_reason,
            }
        })
        .collect();
    let (accounts, _) = CodexOAuthManager::read_store_file(&dir.join(ACCOUNTS_FILENAME))?;
    Ok(Scan {
        source,
        candidates,
        accounts: accounts.len(),
    })
}

/// 导入扫描结果里 `tools` 范围内、没被跳过的卡，以及托管账号（有管理器时）。
pub fn apply(
    env: &Env,
    dir: &Path,
    tools: &[Tool],
    oauth: Option<&CodexOAuthManager>,
) -> Result<Report, AppError> {
    let scan = scan(env, dir)?;
    let mut report = Report::default();
    if let Some(manager) = oauth.filter(|_| tools.contains(&Tool::Codex)) {
        let (accounts, default) = CodexOAuthManager::read_store_file(&dir.join(ACCOUNTS_FILENAME))?;
        report.accounts_added = manager.import_accounts(accounts, default)?;
    }
    store::update(env, |data| {
        for candidate in scan.candidates {
            if !tools.contains(&candidate.tool) {
                continue;
            }
            let id = candidate.provider.id.clone();
            // 扫描之后库可能变了：再查一次重复。
            let reason = candidate.skip_reason.or_else(|| {
                data.provider(candidate.tool, &id)
                    .map(|_| "已有同 id 的卡".to_string())
            });
            if let Some(reason) = reason {
                report.skipped.push((candidate.tool, id, reason));
                continue;
            }
            let app = data.app_mut(candidate.tool);
            let mut provider = candidate.provider;
            if provider.sort_index.is_none() {
                provider.sort_index = Some(app.providers.len() as i64);
            }
            app.providers.push(provider);
            report.created.push((candidate.tool, id));
        }
        Ok(())
    })?;
    Ok(report)
}

fn skip_reason(data: &store::StoreData, tool: Tool, provider: &Provider) -> Option<String> {
    if data.provider(tool, &provider.id).is_some() {
        return Some("已有同 id 的卡".into());
    }
    // cc-switch 把这些放在 meta 里；手改过的旧版 config.json 也可能放在顶层。
    let meta = |key: &str| {
        provider
            .meta
            .extra
            .get(key)
            .or_else(|| provider.extra.get(key))
            .and_then(Value::as_str)
    };
    if let Some(kind @ ("github_copilot" | "codex_oauth" | "xai_oauth")) = meta("providerType") {
        return Some(format!("需要本地路由（{kind}）"));
    }
    if matches!(tool, Tool::Claude | Tool::ClaudeDesktop) {
        if let Some(format) = meta("apiFormat").filter(|f| !f.is_empty() && *f != "anthropic") {
            return Some(format!("需要本地路由（{format} 格式转换）"));
        }
    }
    if let Err(err) = crate::switch::preflight(data, tool, provider) {
        return Some(err.to_string());
    }
    None
}

fn read_db(path: &Path) -> Result<Vec<(Tool, Provider)>, AppError> {
    use rusqlite::{Connection, OpenFlags};
    let db_err =
        |e: rusqlite::Error| AppError::Config(format!("读取 {} 失败: {e}", path.display()));
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db_err)?;
    let mut stmt = conn
        .prepare(
            "SELECT app_type, id, name, settings_config, website_url, category, created_at, \
             sort_index, notes, icon, icon_color, meta FROM providers \
             ORDER BY app_type, COALESCE(sort_index, 999999), created_at, id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
            ))
        })
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        let (app, id, name, settings, website, category, created, sort, notes, icon, color, meta) =
            row.map_err(db_err)?;
        let Ok(tool) = Tool::from_str(&app) else {
            continue;
        };
        let mut fields = Map::new();
        fields.insert("id".into(), Value::String(id));
        fields.insert("name".into(), Value::String(name));
        fields.insert(
            "settingsConfig".into(),
            serde_json::from_str(&settings).unwrap_or(Value::Null),
        );
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                fields.insert(key.into(), value);
            }
        };
        put("websiteUrl", website.map(Value::String));
        put("category", category.map(Value::String));
        put("createdAt", created.map(Value::from));
        put("sortIndex", sort.map(Value::from));
        put("notes", notes.map(Value::String));
        put("icon", icon.map(Value::String));
        put("iconColor", color.map(Value::String));
        put(
            "meta",
            meta.and_then(|m| serde_json::from_str::<Value>(&m).ok())
                .filter(Value::is_object),
        );
        if let Some(provider) = to_provider(tool, Value::Object(fields)) {
            out.push((tool, provider));
        }
    }
    Ok(out)
}

fn read_legacy(path: &Path) -> Result<Vec<(Tool, Provider)>, AppError> {
    let bytes = std::fs::read(path).map_err(|e| AppError::io(path, e))?;
    let root: Value = serde_json::from_slice(&bytes).map_err(|e| AppError::json(path, e))?;
    let mut out = Vec::new();
    let Some(apps) = root.as_object() else {
        return Ok(out);
    };
    for (app, section) in apps {
        let Ok(tool) = Tool::from_str(app) else {
            continue;
        };
        let Some(providers) = section.get("providers").and_then(Value::as_object) else {
            continue;
        };
        for (id, value) in providers {
            let mut value = value.clone();
            if let Some(obj) = value.as_object_mut() {
                obj.entry("id").or_insert_with(|| Value::String(id.clone()));
                obj.entry("name")
                    .or_insert_with(|| Value::String(id.clone()));
            }
            if let Some(provider) = to_provider(tool, value) {
                out.push((tool, provider));
            }
        }
    }
    Ok(out)
}

fn to_provider(tool: Tool, value: Value) -> Option<Provider> {
    let mut provider: Provider = serde_json::from_value(value).ok()?;
    if provider.id.trim().is_empty() {
        return None;
    }
    // 只在 cc-switch 里有意义的运行时字段不带过来。
    provider.extra.remove("inFailoverQueue");
    provider.extra.remove("isCurrent");
    if tool == Tool::Codex {
        strip_codex_login(&mut provider.settings_config);
    }
    Some(provider)
}

/// Codex 卡里抄进来的 ChatGPT 登录（tokens / auth_mode）不导入，只留 API Key。
fn strip_codex_login(settings: &mut Value) {
    let Some(auth) = settings.get_mut("auth").and_then(Value::as_object_mut) else {
        return;
    };
    for key in [
        "tokens",
        "auth_mode",
        "last_refresh",
        "id_token",
        "access_token",
        "refresh_token",
    ] {
        auth.shift_remove(key);
    }
}
