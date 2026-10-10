//! 写 Codex 的客户端文件：`config.toml` 只替换关键字段和独有字段，其余字节不碰；
//! `auth.json` 和登录暂存在同一个操作里提交，崩溃后按 pending 前滚或丢弃。
//!
//! 移植自 cc-switch `services/provider/codex_direct.rs` 的直连路径。分两步：
//! 1. [`plan`]：在内存里算出 `config.toml` 的补丁，行有问题就在这里报错，什么都不写；
//! 2. [`run`]：拿写锁，读 live 的 `auth.json` 决定它的去向（见 [`crate::codex::login`]），
//!    再按「写完之后盘上有没有登录」定下路由表的 `requires_openai_auth`，一起提交。
//!
//! 和原版的差异：没有代理契约、模型目录、Stack、钥匙串预读和托管账号（Phase 2）。
//! 混合登录的保护全部保留：
//! - `auth.json` 只放官方登录；第三方 Key 写进 `config.toml` 的路由表；
//! - 切到第三方（保留登录关闭）时先把原生登录存进登录暂存，再删 `auth.json`；不写 `{}`；
//! - 切回官方卡时按身份（用户 + workspace）还原；在官方卡上登出后切走，登出生效；
//! - 旧版留下的第三方 Key 残留只在能证明是我们写的时候删；用户自己 `codex login --api-key`
//!   的 Key 不动；
//! - `auth.json` 和暂存都用 `Guarded`（写前 hash 不符就冲突），`auth.json` 排第一个：
//!   Codex CLI 恰好刷新了登录，就在发布任何文件之前停下；
//! - 暂存文件坏了、这次又要往里存登录：停下，不覆盖；
//! - `auth.json` 读不了（比如是个目录）：不碰它，其余照写；
//! - `cli_auth_credentials_store` 为 keyring/auto/认不出时看不到登录，按保留开关推断；
//!   ephemeral 当成没登录。

use serde_json::{Map, Value};
use toml_edit::{Item, Value as TomlValue};

use crate::codex::auth::{
    codex_auth_has_credential_login_material, codex_config_auth_store_mode,
    extract_codex_auth_api_key, CodexAuthStoreMode,
};
use crate::codex::login::{self, AuthInput, AuthTarget, LoginStash, STASH_FILENAME};
use crate::env::Env;
use crate::error::AppError;
use crate::fsutil::sorted_json_bytes;
use crate::live::engine::{digest, read_current, DeviceStore, LiveFile};
use crate::live::patch::{Guarded, LivePatch, WholeFile};
use crate::live::project::codex::{
    requires_openai_auth, CodexConfigPatch, CodexProjection, KnownTable, Route, RouteAuth,
    RouteWrite, RowInput, ROUTE_ID,
};
use crate::operation::{AppWrite, FileChange, OperationReport};
use crate::provider::{Provider, Tool};
use crate::state::PendingTarget;
use crate::store::StoreData;

/// `auth.json` 读不了（比如被换成了目录）时的警告前缀（对应 cc-switch 的
/// `codex_auth_cleanup_failed`）。
pub const WARN_AUTH_UNREADABLE: &str = "codex_auth_unreadable";

/// 官方卡：`category == "official"`，或绑定了托管账号的卡。
pub fn is_official(provider: &Provider) -> bool {
    provider.is_official_category() || provider.managed_codex_account().is_some()
}

fn project(provider: &Provider) -> Result<CodexProjection, AppError> {
    CodexProjection::of(&RowInput {
        settings: &provider.settings_config,
        official: is_official(provider),
    })
}

/// 库里所有 Codex 行能证明的事：它们的投影写过哪些表、第三方的 Key、官方卡里存着的
/// 登录（登录暂存第一次建立时用）。
struct RowFacts {
    retired: Vec<KnownTable>,
    third_party_keys: Vec<String>,
    official_logins: Vec<Value>,
}

fn row_facts(data: &StoreData) -> RowFacts {
    let mut facts = RowFacts {
        retired: Vec::new(),
        third_party_keys: Vec::new(),
        official_logins: Vec::new(),
    };
    for provider in data.providers(Tool::Codex) {
        let auth = provider.settings_config.get("auth");
        if is_official(provider) {
            if provider.managed_codex_account().is_none() {
                if let Some(auth) =
                    auth.filter(|auth| codex_auth_has_credential_login_material(auth))
                {
                    facts.official_logins.push(auth.clone());
                }
            }
            continue;
        }
        if let Some(key) = auth.and_then(extract_codex_auth_api_key) {
            facts.third_party_keys.push(key);
        }
        let Some(doc) = provider
            .settings_config
            .get("config")
            .and_then(Value::as_str)
            .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
        else {
            continue;
        };
        let providers = doc.get("model_providers").and_then(Item::as_table_like);
        let base_url_of = |id: &str| {
            providers
                .and_then(|table| table.get(id))
                .and_then(Item::as_table_like)
                .and_then(|table| table.get("base_url"))
                .and_then(Item::as_str)
                .map(|url| url.trim().to_string())
        };
        // 旧版整份写入时，路由表用的是行自己的 id（custom 是现在写的，不算）。
        let selector = doc
            .get("model_provider")
            .and_then(Item::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty() && *id != ROUTE_ID);
        if let Some((id, base_url)) = selector.and_then(|id| Some((id, base_url_of(id)?))) {
            facts.retired.push(KnownTable {
                id: id.to_string(),
                base_url,
            });
        }
        if let Some(base_url) = doc
            .get("openai_base_url")
            .and_then(Item::as_str)
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty())
        {
            facts.retired.push(KnownTable {
                id: "cc-switch".to_string(),
                base_url,
            });
        }
    }
    facts
}

/// `auth.json` 的去向（拥有所有权的版本，[`run`] 里转成 [`AuthTarget`]）。
#[derive(Debug, Clone)]
enum AuthGoal {
    ThirdParty,
    Official(Value),
}

/// 行里的 `auth`（没有时是空对象）。
fn row_auth(provider: &Provider) -> Value {
    provider
        .settings_config
        .get("auth")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()))
}

/// 这家带进来的独有字段：切走时值还相同就删。
fn outgoing_exclusive(prev: Option<&Provider>) -> Vec<(String, TomlValue)> {
    let Some(provider) = prev else {
        return Vec::new();
    };
    match project(provider) {
        Ok(projection) => projection.exclusive,
        Err(err) => {
            log::warn!(
                "无法投影 Codex 供应商 {} 的独有字段，切走时不清理它们: {err}",
                provider.id
            );
            Vec::new()
        }
    }
}

/// 在内存里算好的一次 Codex 写入。
pub struct Planned {
    config: CodexConfigPatch,
    /// 第三方路由表的凭据来源：写入时按盘上有没有登录定 `requires_openai_auth`。
    stamp: Option<RouteAuth>,
    auth: AuthGoal,
    /// 切走的是没绑托管账号的官方卡：它行里的 `auth`。
    leaving_official: Option<Value>,
    retired_keys: Vec<String>,
    official_logins: Vec<Value>,
}

impl Planned {
    /// `config.toml` 的补丁（编辑器预览用）。
    pub fn config(&self) -> &CodexConfigPatch {
        &self.config
    }
}

/// 算出写入内容；行有问题（比如会把官方登录发给第三方）就在这里报错，什么都不写。
pub fn plan(
    data: &StoreData,
    prev: Option<&Provider>,
    target: &Provider,
) -> Result<Planned, AppError> {
    if target.managed_codex_account().is_some() {
        return Err(AppError::localized(
            "codex.managed_account_unsupported",
            "这张卡绑定了托管的 ChatGPT 账号，ccswitch-lite 目前（Phase 1）还不支持托管账号，什么都没改",
            "This card is bound to a managed ChatGPT account, which ccswitch-lite does not support yet (phase 1). Nothing was changed",
        ));
    }
    let facts = row_facts(data);
    let projection = project(target)?;
    let auth = if is_official(target) {
        AuthGoal::Official(row_auth(target))
    } else {
        AuthGoal::ThirdParty
    };
    let (route, stamp) = match &projection.route {
        Route::Official => (RouteWrite::Official, None),
        Route::Custom { table, auth: kind } => (RouteWrite::Custom(table.clone()), Some(*kind)),
        Route::BuiltIn { id, table } => (
            RouteWrite::BuiltIn {
                id: id.clone(),
                table: table.clone(),
            },
            None,
        ),
        Route::Default => (RouteWrite::Default, None),
    };
    let leaving_official = prev
        .filter(|provider| is_official(provider) && provider.managed_codex_account().is_none())
        .map(row_auth);
    let config = CodexConfigPatch {
        top: projection.top,
        nested: projection.nested,
        exclusive: projection.exclusive,
        outgoing: outgoing_exclusive(prev),
        route,
        retired: facts.retired,
    };
    Ok(Planned {
        config,
        stamp,
        auth,
        leaving_official,
        retired_keys: facts.third_party_keys,
        official_logins: facts.official_logins,
    })
}

/// 读到的登录暂存。
struct LoadedStash {
    stash: LoginStash,
    pre: Option<Vec<u8>>,
    /// 文件在但解析不了（截断、半截拷贝）：里面可能还有登录，不能覆盖。按空的用，这次
    /// 要往里存登录就停下。
    unreadable: Option<String>,
}

/// 读登录暂存。`live` 是现在的 `auth.json`：升级更早版本的暂存时要用。
fn load_stash(store: &DeviceStore, official_logins: &[Value], live: Option<&Value>) -> LoadedStash {
    let path = store.file(STASH_FILENAME);
    let pre = read_current(&path).ok().flatten();
    let (stash, unreadable) = match pre.as_deref().map(serde_json::from_slice::<LoginStash>) {
        Some(Ok(stash)) => (stash.loaded(official_logins, live), None),
        Some(Err(err)) => {
            log::warn!("Codex 登录暂存 {} 无法解析: {err}", path.display());
            (
                LoginStash {
                    initialized: true,
                    ..LoginStash::default()
                },
                Some(err.to_string()),
            )
        }
        None => (LoginStash::seeded_from_rows(official_logins), None),
    };
    LoadedStash {
        stash,
        pre,
        unreadable,
    }
}

fn guarded(pre: Option<&[u8]>, then: WholeFile) -> Guarded {
    Guarded {
        expected_pre: digest(pre),
        then,
    }
}

fn read_config_text(env: &Env) -> String {
    read_current(&env.codex_config_path())
        .ok()
        .flatten()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

/// 执行一次 Codex 写入：拿写锁，决定 `auth.json` 的去向，和 `config.toml`、登录暂存一起
/// 提交，最后落定 `pending`（指针）。
pub fn run(
    env: &Env,
    data: &StoreData,
    op: &str,
    planned: Planned,
    pending: PendingTarget,
) -> Result<OperationReport, AppError> {
    let commit = |target: &PendingTarget| super::commit_target(env, Tool::Codex, target);
    // 先补完上一次的操作，再读 auth.json 和登录暂存：补完会改写它们。
    let write = AppWrite::begin(env.device_store(), Tool::Codex.as_str(), &commit)?;
    let store = &write.store;

    // auth.json 读不了（比如被换成了目录）：不碰它，其余照常写。
    let auth_path = env.codex_auth_path();
    let mut warnings = Vec::new();
    let (auth_pre, auth_readable) = match read_current(&auth_path) {
        Ok(pre) => (pre, true),
        Err(err) => {
            log::warn!("读取 Codex auth.json 失败，这次不改动它: {err}");
            warnings.push(format!(
                "{WARN_AUTH_UNREADABLE}: 读取 {} 失败，这次没有改动它: {err}",
                auth_path.display()
            ));
            (None, false)
        }
    };
    let live_auth = auth_pre
        .as_deref()
        .map(|bytes| serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null));

    let LoadedStash {
        stash,
        pre: stash_pre,
        unreadable: stash_unreadable,
    } = load_stash(store, &planned.official_logins, live_auth.as_ref());
    let preserve = data.settings.preserve_codex_official_auth_on_switch;
    let target = match &planned.auth {
        AuthGoal::ThirdParty => AuthTarget::ThirdParty { preserve },
        AuthGoal::Official(row_auth) => AuthTarget::Official { row_auth },
    };
    let auth_plan = login::plan(AuthInput {
        live: live_auth.as_ref(),
        // Phase 1 没有托管账号：live 里的登录一律按用户自己的原生登录处理。
        live_is_managed: false,
        third_party_keys: &planned.retired_keys,
        leaving_official: planned.leaving_official.as_ref(),
        target,
        stash,
    });

    // 暂存坏了只当它是空的读；要往里存登录时照写会覆盖掉里面原有的登录，停下。
    if let (Some(err), Some(_)) = (&stash_unreadable, &auth_plan.stash) {
        let path = store.file(STASH_FILENAME);
        return Err(AppError::localized(
            "codex.login_stash_unreadable",
            format!(
                "Codex 登录暂存 {} 无法解析（{err}）。这次切换要把 auth.json 里的登录存进去，照写会覆盖暂存里原有的登录。请修复或移走这个文件后重试。本次没有写入任何文件",
                path.display()
            ),
            format!(
                "The Codex login stash {} cannot be parsed ({err}). This switch needs to save the login from auth.json into it, and writing it would overwrite the logins it already holds. Repair or move the file away and try again. Nothing was written",
                path.display()
            ),
        ));
    }

    // Codex 把登录存在哪由 `cli_auth_credentials_store` 决定：只存 auth.json 时看它；存在
    // 系统钥匙串（keyring、auto）或认不出时看不到登录，按保留登录开关推断；ephemeral 从不
    // 落盘，当成没登录。
    let config_text = read_config_text(env);
    let login = match codex_config_auth_store_mode(&config_text) {
        CodexAuthStoreMode::File => auth_plan.login_on_disk,
        CodexAuthStoreMode::Ephemeral => false,
        CodexAuthStoreMode::Keyring | CodexAuthStoreMode::Auto | CodexAuthStoreMode::Unknown => {
            !matches!(planned.auth, AuthGoal::ThirdParty) || preserve
        }
    };
    let mut config = planned.config;
    if let (Some(kind), RouteWrite::Custom(table)) = (planned.stamp, &mut config.route) {
        if matches!(kind, RouteAuth::Bearer | RouteAuth::EnvKey) {
            table.insert(
                "requires_openai_auth",
                toml_edit::value(requires_openai_auth(kind, login)),
            );
        }
    }

    let auth_patch = auth_plan
        .auth
        .clone()
        .filter(|_| auth_readable)
        .map(|auth| {
            let then = match auth {
                Some(auth) => sorted_json_bytes(&auth).map(WholeFile::Write),
                None => Ok(WholeFile::Delete),
            };
            then.map(|then| guarded(auth_pre.as_deref(), then))
        })
        .transpose()?;

    let stash_patch = auth_plan
        .stash
        .as_ref()
        .map(|stash| {
            serde_json::to_vec_pretty(stash)
                .map(|bytes| guarded(stash_pre.as_deref(), WholeFile::Write(bytes)))
                .map_err(|e| AppError::Message(format!("序列化 Codex 登录暂存失败: {e}")))
        })
        .transpose()?;

    let mut changes: Vec<FileChange<'_>> = Vec::new();
    // auth.json 放第一个：Codex CLI 恰好在这时刷新了登录，就在发布任何文件之前停下。
    if let Some(patch) = &auth_patch {
        changes.push(FileChange {
            file: LiveFile::private(&auth_path),
            patch: patch as &dyn LivePatch,
        });
    }
    changes.push(FileChange {
        file: LiveFile::private(env.codex_config_path()),
        patch: &config,
    });
    if let Some(patch) = &stash_patch {
        changes.push(FileChange {
            file: LiveFile::private(store.file(STASH_FILENAME)),
            patch,
        });
    }
    let mut report = write.run(op, &changes, pending)?;
    report.warnings.extend(warnings);
    Ok(report)
}
