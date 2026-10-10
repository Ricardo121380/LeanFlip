//! 首次运行：把客户端现在的配置导入成一张卡，并补一张官方卡。
//!
//! 只在这个工具还没有任何卡时做，不写任何客户端文件，只写供应商库：
//! - Claude：`settings.json` 里有关键字段（地址、Key、模型……）就导入成「当前配置」卡
//!   （只存关键字段和独有字段），另补一张空的官方卡；没有就只建官方卡；
//! - Codex：`config.toml` 没有第三方路由就导入成官方卡（只投影关键字段，其余设置留在 live）；
//!   有路由就导入成第三方卡，另补一张官方卡。**ChatGPT 的 OAuth 登录从不复制进库**：
//!   登录只在 `auth.json` 和设备本地的登录暂存里，不进可能被同步、导出的供应商库。
//!   旧版 cc-switch 留在 `auth.json` 里的第三方 Key（没有登录、只有 `OPENAI_API_KEY`）
//!   会作为这张第三方卡的 Key 导入。

use serde_json::{json, Value};

use crate::codex::auth::{codex_live_auth_is_stale_third_party_residue, extract_codex_auth_api_key};
use crate::env::Env;
use crate::error::AppError;
use crate::live::engine::read_current;
use crate::live::project::claude::{store_into_row, ClaudeProjection};
use crate::live::project::codex::{CodexProjection, Route, RowInput};
use crate::provider::{Provider, Tool};
use crate::store;

pub const IMPORTED_ID: &str = "default";
pub const OFFICIAL_ID: &str = "official";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// 新建的卡的 id（空表示这个工具已经有卡，什么都没做）。
    pub created: Vec<String>,
    /// 当前指针。
    pub current: Option<String>,
}

/// 这个工具还没有卡时，按 live 导入。
pub fn import_live(env: &Env, tool: Tool) -> Result<ImportReport, AppError> {
    crate::switch::settle(env, tool)?;
    if !store::load(env)?.providers(tool).is_empty() {
        return Ok(ImportReport {
            created: Vec::new(),
            current: None,
        });
    }
    let (cards, current) = match tool {
        Tool::Claude => claude_cards(env)?,
        Tool::Codex => codex_cards(env)?,
    };
    store::update(env, |data| {
        let app = data.app_mut(tool);
        if !app.providers.is_empty() {
            return Ok(ImportReport {
                created: Vec::new(),
                current: app.current.clone(),
            });
        }
        let created = cards.iter().map(|card| card.id.clone()).collect();
        for (index, mut card) in cards.into_iter().enumerate() {
            card.sort_index = Some(index as i64);
            app.providers.push(card);
        }
        app.current = Some(current.clone());
        Ok(ImportReport {
            created,
            current: Some(current),
        })
    })
}

fn read_text(path: &std::path::Path) -> Result<Option<String>, AppError> {
    Ok(read_current(path)
        .map_err(AppError::from)?
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
}

fn official_card(settings: Value) -> Provider {
    Provider::new(OFFICIAL_ID, "官方 (Official)", settings).official()
}

fn claude_cards(env: &Env) -> Result<(Vec<Provider>, String), AppError> {
    let path = env.claude_settings_path();
    let official = official_card(json!({ "env": {} }));
    let Some(text) = read_text(&path)? else {
        return Ok((vec![official], OFFICIAL_ID.to_string()));
    };
    let live: Value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text).map_err(|source| AppError::json(&path, source))?
    };
    let projection = ClaudeProjection::of(&live);
    if projection.top.is_empty() && projection.env.is_empty() {
        return Ok((vec![official], OFFICIAL_ID.to_string()));
    }
    let imported = Provider::new(
        IMPORTED_ID,
        "当前配置 (Imported)",
        store_into_row(&json!({}), &projection),
    );
    Ok((vec![imported, official], IMPORTED_ID.to_string()))
}

fn codex_cards(env: &Env) -> Result<(Vec<Provider>, String), AppError> {
    let config = read_text(&env.codex_config_path())?.unwrap_or_default();
    let as_official = json!({ "auth": {}, "config": config });
    let route = CodexProjection::of(&RowInput {
        settings: &as_official,
        official: false,
    });
    match route {
        Ok(projection) if matches!(projection.route, Route::Default) => {
            // 没有第三方路由：live 就是官方配置。
            Ok((vec![official_card(as_official)], OFFICIAL_ID.to_string()))
        }
        Ok(_) => Ok((
            vec![
                Provider::new(IMPORTED_ID, "当前配置 (Imported)", as_official),
                official_card(json!({ "auth": {}, "config": "" })),
            ],
            IMPORTED_ID.to_string(),
        )),
        Err(err) => {
            // 路由要 Key 但 config.toml 里没有：旧版可能把 Key 留在了 auth.json。只认残留
            // 形态（没有登录，只有 OPENAI_API_KEY），有登录的绝不当成第三方 Key。
            let residue_key = read_text(&env.codex_auth_path())
                .ok()
                .flatten()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .filter(codex_live_auth_is_stale_third_party_residue)
                .and_then(|auth| extract_codex_auth_api_key(&auth));
            let Some(key) = residue_key else {
                return Err(err);
            };
            let settings = json!({ "auth": { "OPENAI_API_KEY": key }, "config": config });
            CodexProjection::of(&RowInput {
                settings: &settings,
                official: false,
            })?;
            Ok((
                vec![
                    Provider::new(IMPORTED_ID, "当前配置 (Imported)", settings),
                    official_card(json!({ "auth": {}, "config": "" })),
                ],
                IMPORTED_ID.to_string(),
            ))
        }
    }
}
