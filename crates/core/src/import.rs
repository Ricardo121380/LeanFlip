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
//! - Gemini CLI：`.env` 里有关键字段（地址、Key、模型……）就导入成第三方卡（只存关键
//!   字段和 `model.name`），另补一张官方卡；没有就只建官方卡（Google 登录）。
//! - Claude Desktop：`_meta.json` 里 CC Switch 的 profile 正生效、有网关地址和 Key 就导入
//!   成第三方卡，另补官方卡；否则只建官方卡。
//! - Grok Build：`config.toml` 没有模型表就只建官方卡（xAI 登录）；有就把 `models.default`
//!   指向的那张表（指不到时取唯一那张）导入成第三方卡，另补官方卡。只存这一张表，用户的
//!   其他表和设置留在 live。

use serde_json::{json, Value};

use crate::codex::auth::{
    codex_live_auth_is_stale_third_party_residue, extract_codex_auth_api_key,
};
use crate::env::Env;
use crate::error::AppError;
use crate::live::engine::read_current;
use crate::live::project::claude::{store_into_row, ClaudeProjection};
use crate::live::project::codex::{CodexProjection, Route, RowInput};
use crate::live::project::gemini::GeminiProjection;
use crate::live::project::grok::{normalized_text, GrokProjection};
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
        Tool::Gemini => gemini_cards(env)?,
        Tool::GrokBuild => grok_cards(env)?,
        Tool::ClaudeDesktop => {
            let official = official_card(json!({ "env": {} }));
            match crate::switch::claude_desktop::imported_card(env)? {
                Some(card) => (vec![card, official], IMPORTED_ID.to_string()),
                None => (vec![official], OFFICIAL_ID.to_string()),
            }
        }
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

fn gemini_cards(env: &Env) -> Result<(Vec<Provider>, String), AppError> {
    let official = official_card(json!({ "env": {}, "config": {} }));
    let dotenv = read_text(&env.gemini_env_path())?.unwrap_or_default();
    let live_env: serde_json::Map<String, Value> = crate::live::patch::dotenv::entries(&dotenv)
        .into_iter()
        .map(|(key, value)| (key, Value::String(value)))
        .collect();
    let settings_path = env.gemini_settings_path();
    let settings = match read_text(&settings_path)? {
        Some(text) if !text.trim().is_empty() => serde_json::from_str::<Value>(&text)
            .map_err(|source| AppError::json(&settings_path, source))?,
        _ => json!({}),
    };
    let projection = GeminiProjection::of(
        &json!({ "env": Value::Object(live_env), "config": settings }),
        false,
    );
    if projection.env.is_empty() {
        return Ok((vec![official], OFFICIAL_ID.to_string()));
    }
    let env_row: serde_json::Map<String, Value> = projection
        .env
        .iter()
        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
        .collect();
    let mut config = json!({});
    if let Some(model) = &projection.model_name {
        config = json!({ "model": { "name": model } });
    }
    let imported = Provider::new(
        IMPORTED_ID,
        "当前配置 (Imported)",
        json!({ "env": Value::Object(env_row), "config": config }),
    );
    // 和切换时一样校验：缺凭据的第三方配置不导入，免得下次切回来写坏 live。
    crate::switch::gemini::projection(&imported)?;
    Ok((vec![imported, official], IMPORTED_ID.to_string()))
}

fn grok_cards(env: &Env) -> Result<(Vec<Provider>, String), AppError> {
    let official = official_card(json!({ "config": "" }));
    let text = read_text(&env.grok_config_path())?.unwrap_or_default();
    if text.trim().is_empty() || crate::grok_config::is_official_live_config(&text) {
        return Ok((vec![official], OFFICIAL_ID.to_string()));
    }
    let projection = GrokProjection::of(&json!({ "config": text }), false)?;
    let (name, table) = projection.table.as_ref().expect("非官方投影一定有模型表");
    let imported = Provider::new(
        IMPORTED_ID,
        "当前配置 (Imported)",
        json!({ "config": normalized_text(name, table) }),
    );
    Ok((vec![imported, official], IMPORTED_ID.to_string()))
}
