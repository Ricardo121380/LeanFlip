//! 导入 cc-switch 的**导出文件**（另一台机器上「设置 → 数据 → 导出 SQL 备份」得到的
//! `cc-switch-export-*.sql`），只进 LeanFlip 的卡片库，不写任何客户端 live 配置。
//!
//! 支持的格式（按内容识别，不看扩展名）：
//! - **SQL 导出**（cc-switch v3.8+）：以 `-- CC Switch SQLite 导出` 开头的 SQL 文本。在一个
//!   一次性的内存 SQLite 里执行（拒绝 ATTACH / VACUUM / 虚拟表 / 未知动作 / 除
//!   `foreign_keys`、`user_version` 外的 PRAGMA，限时），然后只读 `providers` 表。
//!   每行一条 INSERT 的旧导出和多行 VALUES 的新导出都能读；老版本少列也能读。
//! - **数据库快照**（`backups/*.db`、`cc-switch.db`）：SQLite 文件，只读打开。
//! - **旧版 JSON**（v3.8 之前的 `config.json` 导出）：v2 按应用分节，v1 只有 Claude。
//!
//! 导入规则：
//! - 需要本地路由 / 代理的卡（Copilot、ChatGPT / xAI 反代、非 Anthropic 格式的 Claude 卡、
//!   Claude Desktop 代理模式、Codex Chat 格式等）跳过并写明原因；Oh My OpenCode 插件
//!   配置不是供应商，跳过。
//! - 不导入任何 OAuth token：Codex 卡里抄来的 ChatGPT 登录、`env` 里的 `*OAUTH*`、
//!   用量脚本（可能带 token）都去掉；托管账号本身不在导出文件里。
//! - 和库里已有的卡同 id：按策略跳过或改名（`id-2`…）；配置完全一样的卡总是跳过。
//! - 推广链接（返利 / 追踪 / 短链）形式的 `websiteUrl` 不带过来。

use std::collections::HashSet;
use std::io::Write as _;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ccswitch_import::{
    authorize_sqlite_reader, legacy_rows, parse_app, protect_sqlite, read_provider_rows,
    sqlite_error, to_provider, RawRow,
};
use crate::env::Env;
use crate::error::AppError;
use crate::provider::{Provider, Tool};
use crate::store::{self, StoreData};

pub const SQL_HEADER: &str = "-- CC Switch SQLite 导出";
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";
/// 导出里可能带用量日志，给足空间，但不无限。
pub const MAX_EXPORT_BYTES: usize = 256 * 1024 * 1024;
const SQL_TIME_LIMIT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportFormat {
    SqlDump,
    SqliteDb,
    LegacyJson,
}

/// 解析好的导出文件。
#[derive(Debug, Clone)]
pub struct ParsedExport {
    pub format: ExportFormat,
    /// SQL 导出头里的 `user_version`（cc-switch 数据库版本）。
    pub schema_version: Option<i64>,
    rows: Vec<RawRow>,
}

impl ParsedExport {
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

/// 中英双语的说明，前端按界面语言挑。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reason {
    pub code: &'static str,
    pub zh: String,
    pub en: String,
}

impl Reason {
    pub fn new(code: &'static str, zh: impl Into<String>, en: impl Into<String>) -> Self {
        Self {
            code,
            zh: zh.into(),
            en: en.into(),
        }
    }

    /// 把错误转成双语说明：本地化错误各取各的语言，其他错误两边同一句。
    pub fn from_error(code: &'static str, err: AppError) -> Self {
        match err {
            AppError::Localized { zh, en, .. } => Self::new(code, zh, en),
            other => {
                let text = other.to_string();
                Self::new(code, text.clone(), text)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DuplicatePolicy {
    #[default]
    Skip,
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemStatus {
    Import,
    Skip,
    Duplicate,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    /// cc-switch 的 `app_type`（不认识的应用也原样给出）。
    pub app: String,
    pub tool: Option<Tool>,
    pub source_id: String,
    pub name: String,
    pub status: ItemStatus,
    /// 会以这个 id 导入（改名时和 `source_id` 不同）。
    pub target_id: Option<String>,
    pub reason: Option<Reason>,
    /// 能导入但要注意的地方（比如绑定的托管账号要先登录）。
    pub notes: Vec<Reason>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub format: ExportFormat,
    pub schema_version: Option<i64>,
    pub items: Vec<PreviewItem>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// (工具, 原 id, 导入后的 id)
    pub created: Vec<(Tool, String, String)>,
    pub skipped: usize,
}

// ─── 解析 ────────────────────────────────────────────────────────────

fn invalid(zh: impl Into<String>, en: impl Into<String>) -> AppError {
    AppError::localized("ccswitch_export.invalid", zh, en)
}

/// 识别并解析导出文件。不认识的内容、截断的 SQL、没有 `providers` 表的文件都报错。
pub fn parse(bytes: &[u8]) -> Result<ParsedExport, AppError> {
    if bytes.len() > MAX_EXPORT_BYTES {
        return Err(invalid(
            format!("文件太大（超过 {} MB）", MAX_EXPORT_BYTES / 1024 / 1024),
            format!(
                "File too large (over {} MB)",
                MAX_EXPORT_BYTES / 1024 / 1024
            ),
        ));
    }
    if bytes.starts_with(SQLITE_MAGIC) {
        return parse_sqlite_file(bytes, SQL_TIME_LIMIT);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| not_an_export())?;
    let text = text.trim_start_matches('\u{feff}');
    let head = text.trim_start();
    if head.starts_with(SQL_HEADER) {
        return parse_sql(head, SQL_TIME_LIMIT);
    }
    if head.starts_with('{') {
        return parse_legacy_json(head);
    }
    Err(not_an_export())
}

fn not_an_export() -> AppError {
    invalid(
        "不是 cc-switch 的导出文件（支持 SQL 导出、.db 备份、旧版 JSON 配置）",
        "Not a cc-switch export (supported: SQL export, .db backup, legacy JSON config)",
    )
}

fn parse_sql(sql: &str, time_limit: Duration) -> Result<ParsedExport, AppError> {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use rusqlite::Connection;

    let conn = Connection::open_in_memory().map_err(sqlite_error)?;
    protect_sqlite(&conn, time_limit)?;
    conn.authorizer(Some(|ctx: AuthContext<'_>| {
        let escapes = match ctx.action {
            AuthAction::Attach { .. } | AuthAction::Detach { .. } => true,
            AuthAction::CreateVtable { .. } | AuthAction::DropVtable { .. } => true,
            AuthAction::Unknown { .. } => true,
            AuthAction::Function { function_name } => {
                function_name.eq_ignore_ascii_case("load_extension")
            }
            AuthAction::Pragma { pragma_name, .. } => !["foreign_keys", "user_version"]
                .iter()
                .any(|p| pragma_name.eq_ignore_ascii_case(p)),
            _ => false,
        };
        if escapes {
            Authorization::Deny
        } else {
            Authorization::Allow
        }
    }));
    let result = conn.execute_batch(sql);
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    result.map_err(sqlite_error)?;
    authorize_sqlite_reader(&conn);
    if !conn.is_autocommit() {
        return Err(invalid(
            "SQL 导出不完整（事务没有结束），文件可能被截断",
            "The SQL export is incomplete (unfinished transaction); the file may be truncated",
        ));
    }
    let schema_version = conn
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .ok();
    Ok(ParsedExport {
        format: ExportFormat::SqlDump,
        schema_version,
        rows: providers_of(&conn)?,
    })
}

fn parse_sqlite_file(bytes: &[u8], time_limit: Duration) -> Result<ParsedExport, AppError> {
    use rusqlite::{Connection, OpenFlags};
    let mut file = tempfile::NamedTempFile::new().map_err(|e| AppError::io("tempfile", e))?;
    file.write_all(bytes)
        .and_then(|_| file.flush())
        .map_err(|e| AppError::io(file.path(), e))?;
    let conn = Connection::open_with_flags(
        file.path(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(sqlite_error)?;
    protect_sqlite(&conn, time_limit)?;
    authorize_sqlite_reader(&conn);
    let schema_version = conn
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .ok();
    Ok(ParsedExport {
        format: ExportFormat::SqliteDb,
        schema_version,
        rows: providers_of(&conn)?,
    })
}

fn providers_of(conn: &rusqlite::Connection) -> Result<Vec<RawRow>, AppError> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='providers')",
            [],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    if !exists {
        return Err(invalid(
            "文件里没有 providers 表，不是完整的 cc-switch 导出",
            "No providers table: this is not a complete cc-switch export",
        ));
    }
    read_provider_rows(conn).map_err(sqlite_error)
}

fn parse_legacy_json(text: &str) -> Result<ParsedExport, AppError> {
    let root: Value = serde_json::from_str(text).map_err(|_| {
        invalid(
            "JSON 无法解析，请检查导出文件是否完整。",
            "Cannot parse JSON; check that the export file is complete.",
        )
    })?;
    let rows = legacy_rows(&root);
    if rows.is_empty() {
        return Err(not_an_export());
    }
    Ok(ParsedExport {
        format: ExportFormat::LegacyJson,
        schema_version: root.get("version").and_then(Value::as_i64),
        rows,
    })
}

// ─── 预览 / 导入 ─────────────────────────────────────────────────────

/// 一行的初步判断（不含重复）：能导入的卡，或跳过原因。
fn classify(data: &StoreData, row: &RawRow) -> Result<(Tool, Provider, Vec<Reason>), Reason> {
    let Some(tool) = parse_app(&row.app) else {
        return Err(Reason::new(
            "unsupported_app",
            format!("LeanFlip 不支持这个应用（{}）", row.app),
            format!("LeanFlip does not support this app ({})", row.app),
        ));
    };
    let Some(provider) = to_provider(tool, row.value.clone()) else {
        return Err(Reason::new(
            "malformed",
            "供应商数据不完整（缺少 ID 或配置）",
            "Incomplete provider (missing ID or settings)",
        ));
    };
    let meta = |key: &str| {
        provider
            .meta
            .extra
            .get(key)
            .or_else(|| provider.extra.get(key))
            .and_then(Value::as_str)
    };
    if let Some(category @ ("omo" | "omo-slim")) = provider.category.as_deref() {
        return Err(Reason::new(
            "plugin",
            format!("Oh My OpenCode 插件配置（{category}），不是供应商"),
            format!("Oh My OpenCode plugin config ({category}), not a provider"),
        ));
    }
    if let Some(kind @ ("github_copilot" | "codex_oauth" | "xai_oauth")) = meta("providerType") {
        // 官方 Codex 卡带 codex_oauth 标记只是 cc-switch 的写法，LeanFlip 直接支持。
        let official_codex = tool == Tool::Codex && provider.is_official_category();
        if !official_codex {
            return Err(Reason::new(
                "routing",
                format!("需要 cc-switch 的本地路由 / 反代（{kind}）"),
                format!("Needs cc-switch's local routing / proxy ({kind})"),
            ));
        }
    }
    if matches!(tool, Tool::Claude | Tool::ClaudeDesktop) {
        if let Some(format) = meta("apiFormat").filter(|f| !f.is_empty() && *f != "anthropic") {
            return Err(Reason::new(
                "routing",
                format!("需要本地路由做格式转换（{format}）"),
                format!("Needs local routing for format conversion ({format})"),
            ));
        }
    }
    if let Err(err) = crate::switch::preflight(data, tool, &provider) {
        return Err(Reason::from_error("unsupported_config", err));
    }
    let mut notes = crate::switch::provider_warnings(tool, &provider);
    if let Some(account) = provider
        .meta
        .auth_binding
        .as_ref()
        .and_then(|b| b.account_id.as_deref())
    {
        notes.push(Reason::new(
            "account_binding",
            format!("绑定了托管 ChatGPT 账号（{account}）；账号不在导出文件里，请在 Codex 账号页登录后再切换"),
            format!("Bound to managed ChatGPT account ({account}); accounts are not in the export, sign in on the Codex accounts page before switching"),
        ));
    }
    Ok((tool, provider, notes))
}

fn cards(data: &StoreData, tool: Tool) -> &[Provider] {
    data.app(tool)
        .map(|a| a.providers.as_slice())
        .unwrap_or(&[])
}

fn unique_id(base: &str, taken: &HashSet<String>) -> String {
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken.contains(id))
        .expect("unbounded")
}

struct Planned {
    item: PreviewItem,
    provider: Option<Provider>,
}

fn plan(data: &StoreData, parsed: &ParsedExport, policy: DuplicatePolicy) -> Vec<Planned> {
    let mut taken: HashSet<(Tool, String)> = Tool::ALL
        .iter()
        .flat_map(|&tool| cards(data, tool).iter().map(move |p| (tool, p.id.clone())))
        .collect();
    let mut out = Vec::new();
    for row in &parsed.rows {
        let source_id = row
            .value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let name = row
            .value
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .unwrap_or(&source_id)
            .to_string();
        let mut item = PreviewItem {
            app: row.app.clone(),
            tool: parse_app(&row.app),
            source_id: source_id.clone(),
            name,
            status: ItemStatus::Skip,
            target_id: None,
            reason: None,
            notes: Vec::new(),
        };
        let (tool, mut provider, notes) = match classify(data, row) {
            Ok(ok) => ok,
            Err(reason) => {
                item.reason = Some(reason);
                out.push(Planned {
                    item,
                    provider: None,
                });
                continue;
            }
        };
        item.notes = notes;
        if let Some(same) = cards(data, tool)
            .iter()
            .find(|p| p.settings_config == provider.settings_config)
        {
            item.status = ItemStatus::Duplicate;
            item.reason = Some(Reason::new(
                "same_config",
                format!("和已有的供应商「{}」配置完全相同", same.name),
                format!("Same settings as the existing provider \"{}\"", same.name),
            ));
            out.push(Planned {
                item,
                provider: None,
            });
            continue;
        }
        if taken.contains(&(tool, provider.id.clone())) {
            item.status = ItemStatus::Duplicate;
            match policy {
                DuplicatePolicy::Skip => {
                    item.reason = Some(Reason::new(
                        "same_id",
                        "已有同 ID 的供应商，已跳过",
                        "A provider with this ID already exists; skipped",
                    ));
                    out.push(Planned {
                        item,
                        provider: None,
                    });
                    continue;
                }
                DuplicatePolicy::Rename => {
                    let ids: HashSet<String> = taken
                        .iter()
                        .filter(|(t, _)| *t == tool)
                        .map(|(_, id)| id.clone())
                        .collect();
                    let new_id = unique_id(&provider.id, &ids);
                    item.reason = Some(Reason::new(
                        "renamed",
                        format!("已有同 ID 的供应商，改名为 {new_id} 导入"),
                        format!("A provider with this ID exists; imported as {new_id}"),
                    ));
                    provider.id = new_id;
                }
            }
        } else {
            item.status = ItemStatus::Import;
        }
        taken.insert((tool, provider.id.clone()));
        item.target_id = Some(provider.id.clone());
        out.push(Planned {
            item,
            provider: Some(provider),
        });
    }
    out
}

/// 只读预览：每一行会导入、跳过（原因）还是重复。
pub fn preview(
    env: &Env,
    parsed: &ParsedExport,
    policy: DuplicatePolicy,
) -> Result<Preview, AppError> {
    let data = store::load_readonly(env)?;
    Ok(Preview {
        format: parsed.format,
        schema_version: parsed.schema_version,
        items: plan(&data, parsed, policy)
            .into_iter()
            .map(|p| p.item)
            .collect(),
    })
}

/// 把 `tools` 范围内能导入的卡写进卡片库。只改 LeanFlip 自己的库，不碰 live 配置。
pub fn apply(
    env: &Env,
    parsed: &ParsedExport,
    tools: &[Tool],
    policy: DuplicatePolicy,
) -> Result<Report, AppError> {
    let mut report = Report::default();
    store::update(env, |data| {
        // 在锁里重新规划：预览之后库可能变了。
        for planned in plan(data, parsed, policy) {
            let Some(mut provider) = planned.provider else {
                report.skipped += 1;
                continue;
            };
            let tool = planned.item.tool.expect("planned providers have a tool");
            if !tools.contains(&tool) {
                report.skipped += 1;
                continue;
            }
            let app = data.app_mut(tool);
            provider.sort_index = Some(app.providers.len() as i64);
            report
                .created
                .push((tool, planned.item.source_id.clone(), provider.id.clone()));
            app.providers.push(provider);
        }
        Ok(())
    })?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    #[test]
    fn sqlite_errors_never_echo_source_text() {
        let sql = format!("{SQL_HEADER}\nfixture_private_api_key_invalid_sql;");
        let error = parse(sql.as_bytes()).unwrap_err();
        assert!(!error.to_string().contains("fixture_private_api_key"));
        assert!(matches!(
            error,
            AppError::Localized {
                key: "ccswitch_import.sqlite",
                ..
            }
        ));
    }

    #[test]
    fn database_readers_reject_provider_views_and_interrupt_queries() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let conn = rusqlite::Connection::open(file.path()).unwrap();
        conn.execute_batch("CREATE VIEW providers AS WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c) SELECT x AS id FROM c;").unwrap();
        drop(conn);
        let started = Instant::now();
        let bytes = std::fs::read(file.path()).unwrap();
        assert!(parse_sqlite_file(&bytes, Duration::from_millis(30)).is_err());
        let conn = rusqlite::Connection::open_with_flags(
            file.path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        protect_sqlite(&conn, Duration::from_millis(30)).unwrap();
        authorize_sqlite_reader(&conn);
        assert!(read_provider_rows(&conn).is_err());
        let result = conn.query_row(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c) SELECT sum(x) FROM c",
            [],
            |row| row.get::<_, i64>(0),
        );
        assert_eq!(
            result.unwrap_err().sqlite_error_code(),
            Some(rusqlite::ErrorCode::OperationInterrupted)
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn runaway_sql_is_interrupted() {
        let sql = format!(
            "{SQL_HEADER}\nCREATE TABLE providers (id TEXT, app_type TEXT, name TEXT, settings_config TEXT);\n\
             CREATE TABLE t (x);\n\
             INSERT INTO t WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c;\n"
        );
        let started = Instant::now();
        let err = parse_sql(&sql, Duration::from_millis(200)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(matches!(
            err,
            AppError::Localized {
                key: "ccswitch_import.timeout",
                ..
            }
        ));
    }

    #[test]
    fn oversized_files_are_refused_before_parsing() {
        let big = vec![b' '; MAX_EXPORT_BYTES + 1];
        assert!(parse(&big).unwrap_err().to_string().contains("MB"));
    }
}
