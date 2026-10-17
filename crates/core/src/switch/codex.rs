//! Prepare network-dependent account work before taking the client write lock.
//! The transaction then validates generations and publishes credential/config edits together.
use crate::codex::{
    auth::*,
    login::{self, AuthInput, AuthTarget, LoginStash, STASH_FILENAME},
    oauth::{live as managed, CodexOAuthManager, SwitchAwayGuard},
};
use crate::{
    env::Env,
    error::AppError,
    fsutil::sorted_json_bytes,
    live::{
        engine::{digest, read_current, LiveFile},
        patch::{Guarded, LivePatch, WholeFile},
        project::codex::{
            requires_openai_auth, CodexConfigPatch, CodexProjection, KnownTable, Route, RouteAuth,
            RouteWrite, RowInput,
        },
    },
    operation::{AppWrite, FileChange, OperationReport},
    provider::{Provider, Tool},
    state::PendingTarget,
    store::StoreData,
};
use serde_json::{json, Value};
pub const WARN_AUTH_UNREADABLE: &str = "codex_auth_unreadable";
pub const WARN_LOGIN_REF_MISSING: &str = "codex_login_ref_missing";
pub fn is_official(provider: &Provider) -> bool {
    provider.is_official_category() || provider.managed_codex_account().is_some()
}
fn projection(provider: &Provider) -> Result<CodexProjection, AppError> {
    CodexProjection::of(&RowInput {
        settings: &provider.settings_config,
        official: is_official(provider),
    })
}
fn auth_of(provider: &Provider) -> Value {
    provider
        .settings_config
        .get("auth")
        .cloned()
        .unwrap_or_else(|| json!({}))
}
#[derive(Default)]
struct CatalogFacts {
    routes: Vec<KnownTable>,
    foreign: Vec<String>,
    official: Vec<String>,
    logins: Vec<Value>,
}
impl CatalogFacts {
    fn read(data: &StoreData) -> Self {
        let mut facts = Self::default();
        for provider in data.providers(Tool::Codex) {
            let auth = auth_of(provider);
            if is_official(provider) {
                if provider.managed_codex_account().is_none() {
                    if codex_auth_has_credential_login_material(&auth) {
                        facts.logins.push(auth);
                    } else if let Some(key) = extract_codex_auth_api_key(&auth) {
                        facts.official.push(key);
                    }
                }
            } else {
                if let Some(key) = extract_codex_auth_api_key(&auth) {
                    facts.foreign.push(key);
                }
                if let Some(root) = provider
                    .settings_config
                    .get("config")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<toml::Value>().ok())
                {
                    if let Some(id) = root
                        .get("model_provider")
                        .and_then(toml::Value::as_str)
                        .filter(|s| *s != "custom")
                    {
                        if let Some(url) = root
                            .get("model_providers")
                            .and_then(|v| v.get(id))
                            .and_then(|v| v.get("base_url"))
                            .and_then(toml::Value::as_str)
                        {
                            facts.routes.push(KnownTable {
                                id: id.into(),
                                base_url: url.trim().into(),
                            });
                        }
                    }
                    if let Some(url) = root.get("openai_base_url").and_then(toml::Value::as_str) {
                        facts.routes.push(KnownTable {
                            id: "cc-switch".into(),
                            base_url: url.trim().into(),
                        });
                    }
                }
            }
        }
        facts
    }
}
pub struct Planned {
    config: CodexConfigPatch,
    stamp: Option<RouteAuth>,
    target: Provider,
    previous: Option<Provider>,
}
impl Planned {
    pub(super) fn use_login_state(&mut self, source: &str, preserve: bool, login_on_disk: bool) {
        let login = match codex_config_auth_store_mode(source) {
            CodexAuthStoreMode::File => login_on_disk,
            CodexAuthStoreMode::Ephemeral => false,
            _ => is_official(&self.target) || preserve,
        };
        if let (Some(auth), RouteWrite::Custom(table)) = (self.stamp, &mut self.config.route) {
            if matches!(auth, RouteAuth::Bearer | RouteAuth::EnvKey) {
                table.insert(
                    "requires_openai_auth",
                    toml_edit::value(requires_openai_auth(auth, login)),
                );
            }
        }
    }
    pub fn config(&self) -> &CodexConfigPatch {
        &self.config
    }
}
pub fn plan(
    data: &StoreData,
    previous: Option<&Provider>,
    target: &Provider,
) -> Result<Planned, AppError> {
    if target.managed_codex_account() == Some("") {
        return Err(AppError::localized(
            "codex.oauth.binding_without_account",
            "这个供应商没有指定托管账号",
            "This provider has no managed account ID",
        ));
    }
    let selected = projection(target)?;
    let (route, stamp) = match selected.route {
        Route::Official => (RouteWrite::Official, None),
        Route::Default => (RouteWrite::Default, None),
        Route::Custom { table, auth } => (RouteWrite::Custom(table), Some(auth)),
        Route::BuiltIn { id, table } => (RouteWrite::BuiltIn { id, table }, None),
    };
    Ok(Planned {
        config: CodexConfigPatch {
            top: selected.top,
            nested: selected.nested,
            exclusive: selected.exclusive,
            outgoing: previous
                .and_then(|p| projection(p).ok())
                .map(|p| p.exclusive)
                .unwrap_or_default(),
            route,
            retired: CatalogFacts::read(data).routes,
        },
        stamp,
        target: target.clone(),
        previous: previous.cloned(),
    })
}
#[derive(Debug, Default)]
pub struct Prepared {
    target: Option<(String, Value, Option<String>)>,
    outgoing: Option<(String, SwitchAwayGuard)>,
}
pub fn prepare(
    manager: Option<&CodexOAuthManager>,
    previous: Option<&Provider>,
    target: &Provider,
) -> Result<Prepared, AppError> {
    let manager = || {
        manager.ok_or_else(|| {
            AppError::localized(
                "codex.oauth.manager_unavailable",
                "账号管理器不可用",
                "Account manager unavailable",
            )
        })
    };
    let mut result = Prepared::default();
    if let Some(id) = target.managed_codex_account().filter(|s| !s.is_empty()) {
        let token = manager()?.token_bundle(id)?;
        result.target = Some((id.into(), token.auth_value(), token.observed_live_refresh));
    }
    if let Some(id) = previous
        .and_then(Provider::managed_codex_account)
        .filter(|s| !s.is_empty() && Some(*s) != target.managed_codex_account())
    {
        result.outgoing = Some((id.into(), manager()?.prepare_switch_away(id)?));
    }
    Ok(result)
}
fn guard(before: Option<&[u8]>, content: WholeFile) -> Guarded {
    Guarded {
        expected_pre: digest(before),
        then: content,
    }
}
struct VaultRead {
    value: LoginStash,
    bytes: Option<Vec<u8>>,
    broken: bool,
}
fn vault(env: &Env, rows: &[Value], live: Option<&Value>) -> VaultRead {
    let loaded = read_current(&env.data_file(STASH_FILENAME));
    let (bytes, failed) = match loaded {
        Ok(bytes) => (bytes, false),
        Err(_) => (None, true),
    };
    let decoded = bytes.as_deref().map(serde_json::from_slice::<LoginStash>);
    let valid = decoded
        .as_ref()
        .is_none_or(|r| r.as_ref().is_ok_and(|s| s.version <= login::STASH_VERSION));
    let broken = failed || !valid;
    let value = if broken {
        LoginStash {
            initialized: true,
            ..Default::default()
        }
    } else {
        match decoded {
            Some(Ok(stash)) => stash.loaded(rows, live),
            _ => LoginStash::seeded_from_rows(rows),
        }
    };
    VaultRead {
        value,
        bytes,
        broken,
    }
}
fn changed() -> AppError {
    AppError::localized(
        "codex.oauth.account_changed",
        "准备后的账号已更新或删除，请重试",
        "Account changed or was removed after preparation; retry",
    )
}
pub fn run(
    env: &Env,
    data: &StoreData,
    op: &str,
    mut planned: Planned,
    prepared: &Prepared,
    pending: PendingTarget,
) -> Result<OperationReport, AppError> {
    crate::command_trust::require(env, Tool::Codex, &planned.target)?;
    let commit = |target: &PendingTarget| super::commit_target(env, Tool::Codex, target);
    let transaction = AppWrite::begin(env.device_store(), Tool::Codex.as_str(), &commit)?;
    if &crate::store::load(env)? != data {
        return Err(AppError::localized(
            "live.catalog_changed",
            "供应商库在准备期间发生变化，请基于最新配置重试。",
            "The provider catalog changed during preparation. Retry using the current configuration.",
        ));
    }
    if let Some((id, auth, _)) = &prepared.target {
        let (accounts, _) = CodexOAuthManager::read_store_file(
            &env.data_file(crate::codex::oauth::STORE_FILENAME),
        )?;
        let valid = accounts.iter().any(|a| {
            a.account_id == *id
                && Some(a.refresh_token.as_str())
                    == auth
                        .pointer("/tokens/refresh_token")
                        .and_then(Value::as_str)
                && a.id_token.as_deref() == auth.pointer("/tokens/id_token").and_then(Value::as_str)
                && a.chatgpt_account_id.as_deref()
                    == auth.pointer("/tokens/account_id").and_then(Value::as_str)
        });
        if !valid {
            return Err(changed());
        }
    }
    if let Some((id, guard)) = &prepared.outgoing {
        guard.ensure_unchanged(env, id)?;
    }
    let mut warnings = Vec::new();
    let auth_path = env.codex_auth_path();
    let (auth_bytes, readable) = match read_current(&auth_path) {
        Ok(value) => (value, true),
        Err(_) => {
            warnings.push(format!("{WARN_AUTH_UNREADABLE}: 无法读取 auth.json，已保留 / auth.json is unreadable and was preserved"));
            (None, false)
        }
    };
    let live = auth_bytes
        .as_deref()
        .map(|bytes| serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null));
    let marker_path = managed::marker_path(env);
    let marker_bytes = read_current(&marker_path)?;
    let marker = marker_bytes
        .as_deref()
        .and_then(|b| serde_json::from_slice::<managed::Marker>(b).ok());
    if let Some((id, _, observed)) = &prepared.target {
        let current = live
            .as_ref()
            .filter(|a| managed::auth_matches_marker(a, marker.as_ref(), id))
            .and_then(|a| a.pointer("/tokens/refresh_token"))
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string());
        // Disappearance also means the user logged out; never restore silently.
        if current != *observed {
            return Err(AppError::localized(
                "codex.oauth.live_changed",
                "客户端登录在准备后发生变化，请重试",
                "Client login changed after preparation; retry",
            ));
        }
        if !readable {
            return Err(changed());
        }
    }
    let outgoing_missing = prepared
        .outgoing
        .as_ref()
        .is_some_and(|(_, g)| matches!(g, SwitchAwayGuard::MissingAccount));
    let managed_ids: Vec<&str> = prepared
        .outgoing
        .iter()
        .filter(|(_, g)| !matches!(g, SwitchAwayGuard::MissingAccount))
        .map(|(id, _)| id.as_str())
        .chain(prepared.target.iter().map(|(id, _, _)| id.as_str()))
        .collect();
    let managed_live = live.as_ref().is_some_and(|a| {
        managed_ids
            .iter()
            .any(|id| managed::auth_matches_marker(a, marker.as_ref(), id))
    });
    let facts = CatalogFacts::read(data);
    let loaded = vault(env, &facts.logins, live.as_ref());
    let preserve = data.settings.preserve_codex_official_auth_on_switch;
    let row = auth_of(&planned.target);
    let target = if planned.target.managed_codex_account().is_some() {
        AuthTarget::Managed {
            auth: &prepared.target.as_ref().ok_or_else(changed)?.1,
        }
    } else if is_official(&planned.target) {
        AuthTarget::Official {
            row_auth: &row,
            login_ref: planned.target.codex_login_ref(),
        }
    } else {
        AuthTarget::ThirdParty { preserve }
    };
    let leaving = planned
        .previous
        .as_ref()
        .filter(|p| is_official(p) && p.managed_codex_account().is_none());
    let leaving_auth = leaving.map(auth_of);
    let decision = login::plan(AuthInput {
        live: live.as_ref(),
        live_is_managed: managed_live,
        third_party_keys: &facts.foreign,
        leaving_official: leaving_auth.as_ref(),
        leaving_official_ref: leaving.and_then(Provider::codex_login_ref),
        official_api_keys: &facts.official,
        target,
        stash: loaded.value,
    });
    if loaded.broken && decision.stash.is_some() {
        return Err(AppError::localized(
            "codex.login_stash_unreadable",
            "登录暂存无法读取，本次没有覆盖原内容",
            "Login stash is unreadable; its contents were preserved",
        ));
    }
    if decision.missing_login.is_some() {
        warnings.push(format!("{WARN_LOGIN_REF_MISSING}: 引用的登录已不存在，请重新登录 / Referenced login is missing"));
    }
    let config_source = read_current(&env.codex_config_path())?
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    planned.use_login_state(&config_source, preserve, decision.login_on_disk);
    let mut credentials: Vec<(LiveFile, Guarded)> = Vec::new();
    if readable {
        if let Some(next) = &decision.auth {
            let content = match next {
                Some(value) => WholeFile::Write(sorted_json_bytes(value)?),
                None => WholeFile::Delete,
            };
            credentials.push((
                LiveFile::private(&auth_path),
                guard(auth_bytes.as_deref(), content),
            ));
        }
    }
    let wrote_managed = prepared
        .target
        .as_ref()
        .is_some_and(|(_, auth, _)| decision.auth.as_ref().and_then(Option::as_ref) == Some(auth));
    if wrote_managed && readable {
        let (id, auth, _) = prepared.target.as_ref().unwrap();
        if let Some(bytes) = managed::marker_bytes(auth, id)? {
            credentials.push((
                LiveFile::private(&marker_path),
                guard(marker_bytes.as_deref(), WholeFile::Write(bytes)),
            ));
        }
    } else if readable && marker_bytes.is_some() {
        let stale = !live.as_ref().is_some_and(|a| {
            marker
                .as_ref()
                .is_some_and(|m| managed::auth_matches_marker(a, Some(m), &m.account_id))
        });
        if decision.auth.is_some() || outgoing_missing || stale {
            credentials.push((
                LiveFile::private(&marker_path),
                guard(marker_bytes.as_deref(), WholeFile::Delete),
            ));
        }
    }
    if let Some(stash) = &decision.stash {
        let bytes = serde_json::to_vec_pretty(stash)
            .map_err(|source| AppError::JsonSerialize { source })?;
        credentials.push((
            LiveFile::private(env.data_file(STASH_FILENAME)),
            guard(loaded.bytes.as_deref(), WholeFile::Write(bytes)),
        ));
    }
    let mut changes: Vec<_> = credentials
        .iter()
        .map(|(file, patch)| FileChange {
            file: file.clone(),
            patch: patch as &dyn LivePatch,
        })
        .collect();
    changes.push(FileChange {
        file: LiveFile::private(env.codex_config_path()),
        patch: &planned.config,
    });
    let mut report = transaction.run(op, &changes, pending)?;
    report.warnings.extend(warnings);
    Ok(report)
}
pub fn available_logins(env: &Env) -> Result<Vec<login::LoginSummary>, AppError> {
    let facts = CatalogFacts::read(&crate::store::load_readonly(env)?);
    let live = read_current(&env.codex_auth_path())?
        .map(|b| {
            serde_json::from_slice::<Value>(&b)
                .map_err(|e| AppError::json(env.codex_auth_path(), e))
        })
        .transpose()?;
    let marker = managed::read_marker(env);
    let native = live.as_ref().filter(|a| {
        !marker
            .as_ref()
            .is_some_and(|m| managed::auth_matches_marker(a, Some(m), &m.account_id))
    });
    let loaded = vault(env, &facts.logins, native);
    if loaded.broken {
        return Err(AppError::Config(
            "登录暂存无法读取 / Login stash is unreadable".into(),
        ));
    }
    let mut summaries: Vec<_> = native
        .and_then(|a| login::summarize(a, true))
        .into_iter()
        .collect();
    for auth in loaded.value.logins.values() {
        if let Some(summary) = login::summarize(auth, false) {
            if !summaries.iter().any(|s| s.identity == summary.identity) {
                summaries.push(summary);
            }
        }
    }
    Ok(summaries)
}
/// Validate the binding in the same operation as the provider write.
pub fn validate_login_ref(env: &Env, provider: &Provider) -> Result<(), AppError> {
    let Some(identity) = provider.codex_login_ref() else {
        return Ok(());
    };
    if !provider.is_official_category()
        || provider.managed_codex_account().is_some()
        || codex_live_auth_is_stale_third_party_residue(&auth_of(provider))
    {
        return Err(AppError::Config(
            "This provider cannot reference a local login / 此供应商不能引用本机登录".into(),
        ));
    }
    if !available_logins(env)?
        .iter()
        .any(|s| s.identity == identity)
    {
        return Err(AppError::Config(
            "Referenced login was not found / 未找到引用的登录".into(),
        ));
    }
    Ok(())
}
pub fn set_login_ref(
    env: &Env,
    services: super::Services<'_>,
    provider_id: &str,
    identity: Option<&str>,
) -> Result<Option<OperationReport>, AppError> {
    let mut provider = crate::store::load(env)?
        .provider(Tool::Codex, provider_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(provider_id.into()))?;
    let auth = auth_of(&provider);
    if !provider.is_official_category()
        || provider.managed_codex_account().is_some()
        || codex_live_auth_is_stale_third_party_residue(&auth)
    {
        return Err(AppError::localized(
            "codex.login_ref.not_applicable",
            "这个供应商不能引用本机登录",
            "This provider cannot reference a local login",
        ));
    }
    if let Some(identity) = identity {
        if !available_logins(env)?
            .iter()
            .any(|s| s.identity == identity)
        {
            return Err(AppError::localized(
                "codex.login_ref.unknown",
                "未找到引用的登录",
                "Referenced login was not found",
            ));
        }
    }
    provider.set_codex_login_ref(identity);
    super::update_provider_with(env, services, Tool::Codex, provider)
}
