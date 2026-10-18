use super::*;
use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::time::Duration;

pub(super) fn read(dir: &Path) -> Result<(bool, Vec<Candidate>), AppError> {
    let path = dir.join("ai-toolbox.db");
    if !path.try_exists().map_err(|e| AppError::io(&path, e))? {
        return Ok((false, vec![]));
    }
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| invalid_format())?;
    ccswitch_import::protect_sqlite(&conn, Duration::from_secs(10))?;
    conn.execute_batch("BEGIN").map_err(|_| invalid_format())?;
    ccswitch_import::authorize_sqlite_reader(&conn);
    let mut candidates = Vec::new();
    let mut found_table = false;
    let mut total_bytes = 0;
    for (table, tool) in [
        ("claude_provider", Tool::Claude),
        ("codex_provider", Tool::Codex),
        ("gemini_cli_provider", Tool::Gemini),
        ("claude_desktop_provider", Tool::ClaudeDesktop),
    ] {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |r| r.get(0),
            )
            .map_err(|_| invalid_format())?;
        if !exists {
            continue;
        }
        found_table = true;
        // Table names come only from the fixed list above. json() accepts both JSON text and JSONB.
        let mut stmt = conn
            .prepare(&format!(
                "SELECT id, CASE WHEN length(data) <= 16777216 THEN json(data) END FROM {table} ORDER BY id LIMIT 10001"
            ))
            .map_err(|_| invalid_format())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|_| invalid_format())?;
        for row in rows {
            let (id, text) = row.map_err(|_| invalid_format())?;
            total_bytes += text.len();
            if candidates.len() >= 10000 || total_bytes > 16 * 1024 * 1024 {
                return Err(invalid_format());
            }
            let raw: Value = serde_json::from_str(&text).map_err(|_| invalid_format())?;
            let settings = match &raw["settings_config"] {
                Value::String(s) => serde_json::from_str(s).map_err(|_| invalid_format())?,
                Value::Object(_) => raw["settings_config"].clone(),
                _ => return Err(invalid_format()),
            };
            let mut meta = raw
                .get("meta")
                .filter(|v| v.is_object())
                .cloned()
                .unwrap_or(json!({}));
            // Upstream accepts camelCase/snake_case metadata and settings aliases.
            if let Some(format) = meta
                .get("api_format")
                .or_else(|| settings.get("apiFormat"))
                .or_else(|| settings.get("api_format"))
                .cloned()
            {
                if meta["apiFormat"].is_null() {
                    meta["apiFormat"] = format;
                }
            }
            if let Some(kind) = meta.get("provider_type").cloned() {
                if meta["providerType"].is_null() {
                    meta["providerType"] = kind;
                }
            }
            if meta["apiFormat"] == "anthropic_messages"
                || meta["apiFormat"] == "anthropic/messages"
            {
                meta["apiFormat"] = "anthropic".into();
            }
            let provider = ccswitch_import::to_provider(tool, json!({
                "id": format!("ai-toolbox:{id}"), "name": raw["name"], "category": raw["category"],
                "settingsConfig": settings, "meta": meta,
            })).ok_or_else(invalid_format)?;
            let mut c = candidate(tool, provider);
            let protocol_needs_gateway =
                meta["apiFormat"].as_str().is_some_and(|format| match tool {
                    Tool::Codex => !matches!(
                        format,
                        "" | "openai_responses" | "openai/responses" | "responses"
                    ),
                    Tool::Gemini => !matches!(format, "" | "gemini_native" | "gemini"),
                    _ => false,
                });
            if protocol_needs_gateway
                || !meta["gatewayProfile"].is_null()
                || !meta["gateway_profile"].is_null()
                || meta["providerType"]
                    .as_str()
                    .is_some_and(|s| !matches!(s, "api_key" | "custom" | "official"))
            {
                c.skip_reason = Some(Reason::new(
                    "gateway",
                    "依赖 AI Toolbox 网关或登录态",
                    "Requires the AI Toolbox gateway or login",
                ));
            }
            if raw["extra_settings_config"]
                .as_str()
                .is_some_and(|s| !matches!(s.trim(), "" | "{}"))
            {
                c.notes.push(Reason::new(
                    "provider_only",
                    "仅导入连接配置，不导入额外设置或公共配置",
                    "Only connection settings are imported; extra and common settings are excluded",
                ));
            }
            let official = c.provider.is_official_category();
            let settings = &mut c.provider.settings_config;
            settings
                .as_object_mut()
                .ok_or_else(invalid_format)?
                .remove("credential_ref");
            c.needs_api_key = !official
                && match tool {
                    Tool::Codex => settings["auth"]["OPENAI_API_KEY"]
                        .as_str()
                        .is_none_or(|s| s.is_empty()),
                    Tool::Claude | Tool::ClaudeDesktop => {
                        ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"]
                            .iter()
                            .all(|k| settings["env"][k].as_str().is_none_or(|s| s.is_empty()))
                    }
                    Tool::Gemini => settings["env"]["GEMINI_API_KEY"]
                        .as_str()
                        .is_none_or(|s| s.is_empty()),
                    _ => false,
                };
            // Missing objects must still permit the explicit key-entry step.
            let key = if tool == Tool::Codex { "auth" } else { "env" };
            if settings[key].is_null() {
                settings[key] = json!({});
            }
            if !settings[key].is_object() {
                return Err(invalid_format());
            }
            candidates.push(c);
        }
    }
    if !found_table {
        return Err(invalid_format());
    }
    Ok((true, candidates))
}
