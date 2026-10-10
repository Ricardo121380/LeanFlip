//! 写 Codex 的客户端文件：`config.toml` 只替换关键字段和独有字段，其余字节不碰；
//! `auth.json` 和登录暂存在同一个操作里提交，崩溃后按 pending 前滚或丢弃。
//!
//! 移植自 cc-switch `services/provider/codex_direct.rs` 的直连路径。分两步：
//! 1. [`plan`]：在内存里算出 `config.toml` 的补丁，行有问题就在这里报错，什么都不写；
//! 2. [`run`]：拿写锁，读 live 的 `auth.json` 决定它的去向（见 [`crate::codex::login`]），
//!    再按「写完之后盘上有没有登录」定下路由表的 `requires_openai_auth`，一起提交。
//!
//! 托管 ChatGPT 账号（Phase 2）：拿写锁之前先 [`prepare`]（可能联网：取目标账号的 token
//! 束、把要切走的账号在盘上轮换过的 refresh token 采纳回来），再 plan / run。写锁里：
//! - 切走的账号：采纳时记下的那一代还在 live 里，否则停下（[`SwitchAwayGuard`]）；
//! - 目标账号：生成 bundle 时看到的 live 代数还在，否则停下（不拿旧 bundle 覆盖 Codex CLI
//!   刚轮换出的 refresh token）；
//! - 托管登录不进暂存，切走时删掉；写入托管登录时同一次提交写所有权标记，写别的东西时
//!   删掉标记（原版只在删 `auth.json` 时删标记）。
//!
//! 和原版的差异：没有代理契约、模型目录、Stack、钥匙串预读。混合登录的保护全部保留：
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
use crate::codex::oauth::live as managed_live;
use crate::codex::oauth::{CodexOAuthManager, SwitchAwayGuard};
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

/// 目标官方卡引用的登录在暂存和 live 里都没有（被登出了）时的警告前缀。
pub const WARN_LOGIN_REF_MISSING: &str = "codex_login_ref_missing";

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
    official_api_keys: Vec<String>,
}

fn row_facts(data: &StoreData) -> RowFacts {
    let mut facts = RowFacts {
        retired: Vec::new(),
        third_party_keys: Vec::new(),
        official_logins: Vec::new(),
        official_api_keys: Vec::new(),
    };
    for provider in data.providers(Tool::Codex) {
        let auth = provider.settings_config.get("auth");
        if is_official(provider) {
            if provider.managed_codex_account().is_none() {
                if let Some(auth) =
                    auth.filter(|auth| codex_auth_has_credential_login_material(auth))
                {
                    facts.official_logins.push(auth.clone());
                } else if let Some(key) = auth.and_then(extract_codex_auth_api_key) {
                    facts.official_api_keys.push(key);
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
    /// 行里的 `auth` 和卡引用的登录身份。
    Official(Value, Option<String>),
    Managed,
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
    /// 切走的是没绑托管账号的官方卡：它行里的 `auth` 和引用的登录。
    leaving_official: Option<Value>,
    leaving_official_ref: Option<String>,
    retired_keys: Vec<String>,
    official_logins: Vec<Value>,
    official_api_keys: Vec<String>,
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
    if target.managed_codex_account() == Some("") {
        return Err(AppError::localized(
            "codex.oauth.binding_without_account",
            "这张卡绑定了托管 ChatGPT 账号，但没有指定是哪个账号，什么都没改",
            "This card is bound to a managed ChatGPT account but names no account. Nothing was changed",
        ));
    }
    let facts = row_facts(data);
    let projection = project(target)?;
    let auth = if target.managed_codex_account().is_some() {
        AuthGoal::Managed
    } else if is_official(target) {
        AuthGoal::Official(
            row_auth(target),
            target.codex_login_ref().map(str::to_string),
        )
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
    let leaving =
        prev.filter(|provider| is_official(provider) && provider.managed_codex_account().is_none());
    let leaving_official = leaving.map(row_auth);
    let leaving_official_ref = leaving
        .and_then(Provider::codex_login_ref)
        .map(str::to_string);
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
        leaving_official_ref,
        retired_keys: facts.third_party_keys,
        official_logins: facts.official_logins,
        official_api_keys: facts.official_api_keys,
    })
}

/// 拿写锁之前准备好的托管账号凭据。
#[derive(Debug, Default)]
pub struct Prepared {
    /// 目标托管账号、它的登录，和生成时 live 里它的 refresh token。
    target_login: Option<(String, Value, Option<String>)>,
    /// 要切走的托管账号和采纳后记下的那一代。
    outgoing: Option<(String, SwitchAwayGuard)>,
}

fn manager_required(manager: Option<&CodexOAuthManager>) -> Result<&CodexOAuthManager, AppError> {
    manager.ok_or_else(|| {
        AppError::localized(
            "codex.oauth.manager_unavailable",
            "这次切换涉及托管的 ChatGPT 账号，但账号管理器不可用，什么都没改",
            "This switch involves a managed ChatGPT account, but the account manager is unavailable. Nothing was changed",
        )
    })
}

/// 取目标托管账号的登录（必要时刷新），并在切走托管账号前采纳 Codex CLI 轮换过的
/// refresh token。都可能联网，所以在拿写锁之前做。
pub fn prepare(
    manager: Option<&CodexOAuthManager>,
    prev: Option<&Provider>,
    target: &Provider,
) -> Result<Prepared, AppError> {
    let target_account = target
        .managed_codex_account()
        .filter(|account| !account.is_empty());
    let target_login = match target_account {
        Some(account) => {
            let bundle = manager_required(manager)?.token_bundle(account)?;
            Some((
                account.to_string(),
                bundle.auth_value(),
                bundle.observed_live_refresh.clone(),
            ))
        }
        None => None,
    };
    let outgoing = match prev
        .and_then(Provider::managed_codex_account)
        .filter(|account| !account.is_empty() && Some(*account) != target_account)
    {
        Some(account) => Some((
            account.to_string(),
            manager_required(manager)?.prepare_switch_away(account)?,
        )),
        None => None,
    };
    Ok(Prepared {
        target_login,
        outgoing,
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
    prepared: &Prepared,
    pending: PendingTarget,
) -> Result<OperationReport, AppError> {
    let commit = |target: &PendingTarget| super::commit_target(env, Tool::Codex, target);
    // 先补完上一次的操作，再读 auth.json 和登录暂存：补完会改写它们。
    let write = AppWrite::begin(env.device_store(), Tool::Codex.as_str(), &commit)?;
    let store = &write.store;

    // 切走的托管账号：采纳之后 CLI 又刷新（或登出）了就停下，免得删掉新 token。
    if let Some((account, guard)) = &prepared.outgoing {
        guard.ensure_unchanged(env, account)?;
    }

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

    // live 是不是这次涉及的托管账号的登录（切走时清掉，不进暂存）。已删除的账号放弃了
    // 所有权：它留下的登录按用户自己的登录处理。
    let marker_path = managed_live::marker_path(env);
    let marker_pre = read_current(&marker_path).ok().flatten();
    let marker = managed_live::read_marker(env);
    let managed_accounts: Vec<&str> = prepared
        .outgoing
        .iter()
        .filter(|(_, guard)| !matches!(guard, SwitchAwayGuard::MissingAccount))
        .map(|(account, _)| account.as_str())
        .chain(
            prepared
                .target_login
                .iter()
                .map(|(account, _, _)| account.as_str()),
        )
        .collect();
    let live_is_managed = live_auth.as_ref().is_some_and(|auth| {
        managed_accounts
            .iter()
            .any(|account| managed_live::auth_matches_marker(auth, marker.as_ref(), account))
    });
    let outgoing_missing = prepared
        .outgoing
        .as_ref()
        .is_some_and(|(_, guard)| matches!(guard, SwitchAwayGuard::MissingAccount));

    // 目标托管账号：生成 bundle 时看到的 live 代数要还在（期间 Codex CLI 轮换了 refresh
    // token，拿旧 bundle 覆盖会让新 token 作废）。
    if let Some((account, _, observed)) = &prepared.target_login {
        let now = live_auth
            .as_ref()
            .filter(|auth| managed_live::auth_matches_marker(auth, marker.as_ref(), account))
            .and_then(|auth| auth.pointer("/tokens/refresh_token"))
            .and_then(Value::as_str)
            .map(|token| token.trim().to_string());
        if now.is_some() && now != *observed {
            return Err(AppError::localized(
                "codex.oauth.live_changed",
                format!("Codex CLI 账号 {account} 的 live 凭据在切换期间已刷新；为避免覆盖新的 refresh token，本次操作已取消，请重试"),
                format!("The live credentials of Codex account {account} were refreshed during the switch; to avoid overwriting the new refresh token nothing was changed. Please try again"),
            ));
        }
    }

    let LoadedStash {
        stash,
        pre: stash_pre,
        unreadable: stash_unreadable,
    } = load_stash(store, &planned.official_logins, live_auth.as_ref());
    let preserve = data.settings.preserve_codex_official_auth_on_switch;
    let target = match (&planned.auth, &prepared.target_login) {
        (AuthGoal::Managed, Some((_, auth, _))) => AuthTarget::Managed { auth },
        (AuthGoal::Managed, None) => {
            return Err(AppError::Message(
                "托管账号卡没有准备好登录（缺少 prepare 步骤），什么都没改".into(),
            ))
        }
        (AuthGoal::ThirdParty, _) => AuthTarget::ThirdParty { preserve },
        (AuthGoal::Official(row_auth, login_ref), _) => AuthTarget::Official {
            row_auth,
            login_ref: login_ref.as_deref(),
        },
    };
    let auth_plan = login::plan(AuthInput {
        live: live_auth.as_ref(),
        live_is_managed,
        third_party_keys: &planned.retired_keys,
        leaving_official: planned.leaving_official.as_ref(),
        leaving_official_ref: planned.leaving_official_ref.as_deref(),
        official_api_keys: &planned.official_api_keys,
        target,
        stash,
    });
    if let Some(id) = &auth_plan.missing_login {
        warnings.push(format!(
            "{WARN_LOGIN_REF_MISSING}: 这张官方卡引用的登录 {id} 已不在暂存里（可能在 Codex 里登出了），auth.json 没有改动；请在 Codex 里重新登录"
        ));
    }

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

    // 所有权标记：写入托管登录时同一次提交写它；把 auth.json 换成别的（或删掉）、切走的
    // 账号已删除、标记已经对不上 live 时删掉它。auth.json 读不了时不动标记。
    let writes_managed = matches!(
        (&prepared.target_login, &auth_plan.auth),
        (Some((_, login, _)), Some(Some(written))) if written == login
    );
    let marker_stale = marker_pre.is_some()
        && !live_auth.as_ref().is_some_and(|auth| {
            marker
                .as_ref()
                .is_some_and(|m| managed_live::auth_matches_marker(auth, Some(m), &m.account_id))
        });
    let marker_patch = if writes_managed {
        let (account, login, _) = prepared.target_login.as_ref().expect("writes_managed");
        managed_live::marker_bytes(login, account)?
            .map(|bytes| guarded(marker_pre.as_deref(), WholeFile::Write(bytes)))
    } else if marker_pre.is_some()
        && auth_readable
        && (auth_plan.auth.is_some() || outgoing_missing || marker_stale)
    {
        Some(guarded(marker_pre.as_deref(), WholeFile::Delete))
    } else {
        None
    };

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
    if let Some(patch) = &marker_patch {
        changes.push(FileChange {
            file: LiveFile::private(&marker_path),
            patch,
        });
    }
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

/// 能被官方卡引用的原生登录：登录暂存里的，加上 live 里现在的（托管账号写的不算）。
/// 只读，不写任何文件。
pub fn available_logins(env: &Env) -> Result<Vec<login::LoginSummary>, AppError> {
    super::settle(env, Tool::Codex)?;
    let data = crate::store::load(env)?;
    let facts = row_facts(&data);
    let live = read_current(&env.codex_auth_path())
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let marker = managed_live::read_marker(env);
    let live_native = live.as_ref().filter(|auth| {
        !marker
            .as_ref()
            .is_some_and(|m| managed_live::auth_matches_marker(auth, Some(m), &m.account_id))
    });
    let loaded = load_stash(&env.device_store(), &facts.official_logins, live_native);
    let mut logins: Vec<login::LoginSummary> = Vec::new();
    if let Some(summary) = live_native.and_then(|auth| login::summarize(auth, true)) {
        logins.push(summary);
    }
    for auth in loaded.stash.logins.values() {
        if let Some(summary) = login::summarize(auth, false) {
            if !logins.iter().any(|l| l.identity == summary.identity) {
                logins.push(summary);
            }
        }
    }
    Ok(logins)
}

/// 让官方卡引用某个登录（`None` 取消引用，回到「跟随当前登录」）。是当前卡时立刻
/// 换上那个登录。只接受没绑托管账号、行里没存 API Key 的官方卡，和 [`available_logins`]
/// 里有的身份。
pub fn set_login_ref(
    env: &Env,
    services: super::Services<'_>,
    provider_id: &str,
    identity: Option<&str>,
) -> Result<Option<OperationReport>, AppError> {
    let data = crate::store::load(env)?;
    let mut provider = data
        .provider(Tool::Codex, provider_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("codex 供应商 {provider_id}")))?;
    let row = row_auth(&provider);
    if !provider.is_official_category()
        || provider.managed_codex_account().is_some()
        || extract_codex_auth_api_key(&row)
            .is_some_and(|_| !codex_auth_has_credential_login_material(&row))
    {
        return Err(AppError::localized(
            "codex.login_ref.not_applicable",
            "只有不绑定托管账号、不存 API Key 的官方卡可以引用登录",
            "Only official cards without a managed account or API key can reference a login",
        ));
    }
    if let Some(id) = identity {
        if !available_logins(env)?
            .iter()
            .any(|login| login.identity == id)
        {
            return Err(AppError::localized(
                "codex.login_ref.unknown",
                format!("找不到登录 {id}（暂存和当前 auth.json 里都没有）"),
                format!("Login {id} was not found in the stash or the current auth.json"),
            ));
        }
    }
    provider.set_codex_login_ref(identity);
    super::update_provider_with(env, services, Tool::Codex, provider)
}
