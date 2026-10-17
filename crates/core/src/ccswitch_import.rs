//! 从 cc-switch（`~/.cc-switch`）只读导入供应商卡和托管 ChatGPT 账号。
//!
//! - 卡：优先读 `cc-switch.db`（SQLite，`providers` 表，只读打开），没有数据库时读旧版
//!   `config.json`（v2：`{ "<app>": { "providers": { id: {...} } } }`）。
//! - 账号：`codex_oauth_auth.json`（同格式 v2），id 不变，卡上的绑定照样能用。
//!
//! 绝不写 cc-switch 的任何文件。不导入「当前供应商」：LeanFlip 的指针跟着 live 走。
//! 需要本地路由的卡（非 Anthropic 格式、Copilot / ChatGPT 反代等）跳过并说明原因；
//! Codex 卡里从 `auth.json` 抄来的 ChatGPT 登录不导入（登录只认 live 和登录暂存）。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::ccswitch_export::Reason;
use crate::codex::oauth::CodexOAuthManager;
use crate::env::Env;
use crate::error::AppError;
use crate::provider::{Provider, Tool};
use crate::store;

pub const DB_FILENAME: &str = "cc-switch.db";
pub const LEGACY_CONFIG: &str = "config.json";
pub const ACCOUNTS_FILENAME: &str = "codex_oauth_auth.json";

#[cfg(test)]
mod transaction_tests;

pub fn default_dir(env: &Env) -> PathBuf {
    env.home.join(".cc-switch")
}

/// Inspect CC Switch's own path preference, without starting it or changing it.
/// Paths are rooted in the supplied home so --home remains isolated in tests.
pub fn detected_dir(env: &Env) -> Result<PathBuf, AppError> {
    let data_root = if cfg!(target_os = "macos") {
        env.home.join("Library/Application Support")
    } else if cfg!(windows) {
        env.home.join("AppData/Roaming")
    } else {
        env.home.join(".local/share")
    };
    let path = data_root.join("com.ccswitch.desktop/app_paths.json");
    let bytes = match std::fs::File::open(&path).and_then(|file| {
        let mut bytes = Vec::new();
        file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
        Ok(bytes)
    }) {
        Ok(bytes) if bytes.len() <= 64 * 1024 => bytes,
        Ok(_) => {
            return Err(AppError::InvalidInput(
                "CC Switch app_paths.json is too large; select its data directory manually".into(),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(default_dir(env)),
        Err(error) => return Err(AppError::io(&path, error)),
    };
    let settings: Value = serde_json::from_slice(&bytes).map_err(|_| {
        AppError::Config(
            "Cannot read CC Switch's data-directory preference; select its data directory manually"
                .into(),
        )
    })?;
    let Some(raw) = settings
        .get("app_config_dir_override")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(default_dir(env));
    };
    let selected = if raw == "~" {
        env.home.clone()
    } else if let Some(relative) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        env.home.join(relative)
    } else {
        PathBuf::from(raw)
    };
    if !selected.is_absolute() {
        return Err(AppError::InvalidInput(
            "CC Switch uses a relative data directory; select an absolute directory manually"
                .into(),
        ));
    }
    Ok(if selected.is_dir() {
        selected
    } else {
        default_dir(env)
    })
}

/// 扫描出的一张卡。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub tool: Tool,
    pub provider: Provider,
    /// 有值表示不会导入（中英双语原因）。
    pub skip_reason: Option<Reason>,
    pub notes: Vec<Reason>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub directory: PathBuf,
    pub source: Option<PathBuf>,
    pub candidates: Vec<Candidate>,
    pub accounts: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// (工具, id)
    pub created: Vec<(Tool, String)>,
    pub skipped: Vec<(Tool, String, Reason)>,
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
    let data = store::load_readonly(env)?;
    let candidates = rows
        .into_iter()
        .map(|(tool, provider)| {
            let skip_reason = skip_reason(&data, tool, &provider);
            let notes = crate::switch::provider_warnings(tool, &provider);
            Candidate {
                tool,
                provider,
                skip_reason,
                notes,
            }
        })
        .collect();
    let (accounts, _) = CodexOAuthManager::read_store_file(&dir.join(ACCOUNTS_FILENAME))?;
    Ok(Scan {
        directory: dir.to_path_buf(),
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
    // A retry must settle the previous account/card transaction before taking a new snapshot.
    if oauth.is_some() && tools.contains(&Tool::Codex) {
        crate::switch::settle(env, Tool::Codex)?;
    }
    let scan = scan(env, dir)?;
    let mut report = Report::default();
    let import = |data: &mut store::StoreData| {
        let mut changes = Vec::new();
        for candidate in scan.candidates {
            if !tools.contains(&candidate.tool) {
                continue;
            }
            let id = candidate.provider.id.clone();
            // 扫描之后库可能变了：再查一次重复。
            let reason = candidate.skip_reason.or_else(|| {
                data.provider(candidate.tool, &id)
                    .map(|_| duplicate_reason())
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
            changes.push(crate::state::ProviderChange {
                tool: candidate.tool,
                before: None,
                after: Some(provider.clone()),
            });
            app.providers.push(provider);
            report.created.push((candidate.tool, id));
        }
        Ok(changes)
    };
    if let Some(manager) = oauth.filter(|_| tools.contains(&Tool::Codex)) {
        let changes = import(&mut store::load(env)?)?;
        let (accounts, default) = CodexOAuthManager::read_store_file(&dir.join(ACCOUNTS_FILENAME))?;
        report.accounts_added =
            manager.import_accounts_with_providers(accounts, default, Some(changes))?;
    } else {
        store::update(env, import)?;
    }
    Ok(report)
}

fn duplicate_reason() -> Reason {
    Reason::new(
        "duplicate",
        "已有同 ID 的供应商",
        "A provider with this ID already exists",
    )
}

fn skip_reason(data: &store::StoreData, tool: Tool, provider: &Provider) -> Option<Reason> {
    if data.provider(tool, &provider.id).is_some() {
        return Some(duplicate_reason());
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
        return Some(Reason::new(
            "routing",
            format!("需要本地路由（{kind}）"),
            format!("Needs local routing ({kind})"),
        ));
    }
    if matches!(tool, Tool::Claude | Tool::ClaudeDesktop) {
        if let Some(format) = meta("apiFormat").filter(|f| !f.is_empty() && *f != "anthropic") {
            return Some(Reason::new(
                "routing",
                format!("需要本地路由（{format} 格式转换）"),
                format!("Needs local routing ({format} format conversion)"),
            ));
        }
    }
    if let Err(err) = crate::switch::preflight(data, tool, provider) {
        return Some(Reason::from_error("unsupported_config", err));
    }
    None
}

fn read_db(path: &Path) -> Result<Vec<(Tool, Provider)>, AppError> {
    use rusqlite::{Connection, OpenFlags};
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(sqlite_error)?;
    protect_sqlite(&conn, Duration::from_secs(30))?;
    authorize_sqlite_reader(&conn);
    let rows = read_provider_rows(&conn).map_err(sqlite_error)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let tool = parse_app(&row.app)?;
            Some((tool, to_provider(tool, row.value)?))
        })
        .collect())
}

pub(crate) fn sqlite_error(error: rusqlite::Error) -> AppError {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) {
        AppError::localized(
            "ccswitch_import.timeout",
            "读取数据库超时，已中断导入。请使用完整的 CC Switch 导出文件。",
            "Database import timed out and was interrupted. Use a complete CC Switch export.",
        )
    } else {
        AppError::localized(
            "ccswitch_import.sqlite",
            "无法读取 SQLite 数据，请检查文件格式、完整性和访问权限。",
            "Cannot read SQLite data; check the file format, integrity and access permissions.",
        )
    }
}

pub(crate) fn protect_sqlite(
    conn: &rusqlite::Connection,
    time_limit: Duration,
) -> Result<(), AppError> {
    conn.execute_batch("PRAGMA trusted_schema=OFF;")
        .map_err(sqlite_error)?;
    conn.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .map_err(sqlite_error)?;
    conn.busy_timeout(time_limit).map_err(sqlite_error)?;
    let deadline = Instant::now() + time_limit;
    conn.progress_handler(10_000, Some(move || Instant::now() >= deadline));
    Ok(())
}

pub(crate) fn authorize_sqlite_reader(conn: &rusqlite::Connection) {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    conn.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
        AuthAction::Select | AuthAction::Read { .. } | AuthAction::Recursive => {
            Authorization::Allow
        }
        AuthAction::Function { function_name }
            if !function_name.eq_ignore_ascii_case("load_extension") =>
        {
            Authorization::Allow
        }
        AuthAction::Pragma {
            pragma_name,
            pragma_value: None,
        } if ["table_info", "user_version"]
            .iter()
            .any(|name| pragma_name.eq_ignore_ascii_case(name)) =>
        {
            Authorization::Allow
        }
        AuthAction::Pragma { pragma_name, .. }
            if pragma_name.eq_ignore_ascii_case("table_info") =>
        {
            Authorization::Allow
        }
        _ => Authorization::Deny,
    }));
}

/// cc-switch 数据库 / 导出里的一行：`app_type` 原样保留，卡是 cc-switch 的 JSON 形状。
#[derive(Debug, Clone)]
pub(crate) struct RawRow {
    pub app: String,
    pub value: Value,
}

/// cc-switch 的 `app_type`（含旧别名）→ 工具。
pub(crate) fn parse_app(app: &str) -> Option<Tool> {
    match app {
        "claude_desktop" | "claudeDesktop" => Some(Tool::ClaudeDesktop),
        other => Tool::from_str(other).ok(),
    }
}

/// 读 `providers` 表。老版本的表可能少几列（icon、meta…），按实际有的列读。
pub(crate) fn read_provider_rows(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<RawRow>> {
    // A provider view can execute arbitrary schema code instead of reading rows.
    let table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='providers')",
        [],
        |row| row.get(0),
    )?;
    if !table {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut columns = Vec::new();
    {
        let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('providers')")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            columns.push(row.get::<_, String>(0)?);
        }
    }
    let has = |c: &str| columns.iter().any(|x| x == c);
    let col = |c: &str| {
        if has(c) {
            c.to_string()
        } else {
            format!("NULL AS {c}")
        }
    };
    let order = if has("sort_index") {
        "ORDER BY app_type, COALESCE(sort_index, 999999), id"
    } else {
        "ORDER BY app_type, id"
    };
    let sql = format!(
        "SELECT app_type, id, name, settings_config, {}, {}, {}, {}, {}, {}, {}, {} FROM providers {order}",
        col("website_url"),
        col("category"),
        col("created_at"),
        col("sort_index"),
        col("notes"),
        col("icon"),
        col("icon_color"),
        col("meta"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let text = |i: usize| row.get::<_, Option<String>>(i);
        let int = |i: usize| row.get::<_, Option<i64>>(i);
        let mut fields = Map::new();
        fields.insert("id".into(), Value::String(text(1)?.unwrap_or_default()));
        fields.insert("name".into(), Value::String(text(2)?.unwrap_or_default()));
        fields.insert(
            "settingsConfig".into(),
            text(3)?
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(Value::Null),
        );
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                fields.insert(key.into(), value);
            }
        };
        put("websiteUrl", text(4)?.map(Value::String));
        put("category", text(5)?.map(Value::String));
        put("createdAt", int(6)?.map(Value::from));
        put("sortIndex", int(7)?.map(Value::from));
        put("notes", text(8)?.map(Value::String));
        put("icon", text(9)?.map(Value::String));
        put("iconColor", text(10)?.map(Value::String));
        put(
            "meta",
            text(11)?
                .and_then(|m| serde_json::from_str::<Value>(&m).ok())
                .filter(Value::is_object),
        );
        Ok(RawRow {
            app: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
            value: Value::Object(fields),
        })
    })?;
    rows.collect()
}

fn read_legacy(path: &Path) -> Result<Vec<(Tool, Provider)>, AppError> {
    let bytes = std::fs::read(path).map_err(|e| AppError::io(path, e))?;
    let root: Value = serde_json::from_slice(&bytes).map_err(|e| AppError::json(path, e))?;
    Ok(legacy_rows(&root)
        .into_iter()
        .filter_map(|row| {
            let tool = parse_app(&row.app)?;
            Some((tool, to_provider(tool, row.value)?))
        })
        .collect())
}

/// 旧版 `config.json`：v2 `{ "<app>": { "providers": { id: {...} } } }`；
/// v1 只有 Claude：`{ "providers": {...}, "current": "..." }`。
pub(crate) fn legacy_rows(root: &Value) -> Vec<RawRow> {
    let mut out = Vec::new();
    let Some(apps) = root.as_object() else {
        return out;
    };
    let mut push_section = |app: &str, providers: &Map<String, Value>| {
        for (id, value) in providers {
            let mut value = value.clone();
            if let Some(obj) = value.as_object_mut() {
                obj.entry("id").or_insert_with(|| Value::String(id.clone()));
                obj.entry("name")
                    .or_insert_with(|| Value::String(id.clone()));
            }
            out.push(RawRow {
                app: app.to_string(),
                value,
            });
        }
    };
    if let Some(providers) = apps.get("providers").and_then(Value::as_object) {
        push_section("claude", providers);
    }
    for (app, section) in apps {
        if let Some(providers) = section.get("providers").and_then(Value::as_object) {
            push_section(app, providers);
        }
    }
    out
}

/// cc-switch 专用、LeanFlip 用不上的 meta：用量脚本（里面可能有 token）、合作伙伴 /
/// 促销标记、计费 / 限额、测速端点等。
const DROPPED_META: &[&str] = &[
    "usageScript",
    "isPartner",
    "primePartner",
    "partnerPromotionKey",
    "costMultiplier",
    "pricingModelSource",
    "limitDailyUsd",
    "limitMonthlyUsd",
    "endpointAutoSelect",
    "customEndpoints",
    "testConfig",
    "proxyConfig",
    "promptCacheKey",
];

pub(crate) fn to_provider(tool: Tool, value: Value) -> Option<Provider> {
    let mut provider: Provider = serde_json::from_value(value).ok()?;
    if provider.id.trim().is_empty() || !provider.settings_config.is_object() {
        return None;
    }
    // 只在 cc-switch 里有意义的运行时字段不带过来。
    provider.extra.remove("inFailoverQueue");
    provider.extra.remove("isCurrent");
    for key in DROPPED_META {
        provider.meta.extra.remove(*key);
    }
    if provider
        .extra
        .get("websiteUrl")
        .and_then(Value::as_str)
        .is_some_and(crate::promo::is_promo_link)
    {
        provider.extra.remove("websiteUrl");
    }
    if tool == Tool::Codex {
        strip_codex_login(&mut provider.settings_config);
    }
    strip_oauth_env(&mut provider.settings_config);
    Some(provider)
}

/// `env` 里的 OAuth token（如 `CLAUDE_CODE_OAUTH_TOKEN`）不导入。
fn strip_oauth_env(settings: &mut Value) {
    if let Some(env) = settings.get_mut("env").and_then(Value::as_object_mut) {
        env.retain(|key, _| !key.to_ascii_uppercase().contains("OAUTH"));
    }
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
