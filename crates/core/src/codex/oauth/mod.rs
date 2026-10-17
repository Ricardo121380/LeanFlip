//! Account persistence and refresh coordination for LeanFlip.
//!
//! Disk is authoritative. Each mutation reloads under one OS lock; access-token
//! caches are bound to the exact persisted account generation, never just its ID.
//! Network calls do not hold the UI/device-flow mutex.
pub mod live;
#[doc(hidden)]
pub mod testing;
pub mod transport;
use super::auth::{extract_codex_id_token_user_identity, jwt_claims};
use crate::{
    env::Env,
    error::AppError,
    fsutil,
    live::{
        engine::{digest, lock_file, read_current, LiveFile},
        patch::{Guarded, WholeFile},
    },
    operation::{AppWrite, FileChange},
    provider::Tool,
    state::PendingTarget,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};
use transport::{Clock, DevicePoll, OAuthTransport, SystemClock, TokenResponse, TransportError};
pub const STORE_FILENAME: &str = "codex_oauth_auth.json";
pub const STORE_VERSION: u32 = 2;
pub const TOKEN_REFRESH_BUFFER_MS: i64 = 60_000;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountData {
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub refresh_token: String,
    pub authenticated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(default)]
    pub token_updated_at_ms: i64,
}
impl AccountData {
    fn owner(&self) -> Option<String> {
        self.id_token
            .as_deref()
            .and_then(extract_codex_id_token_user_identity)
    }
    pub fn reauth_required(&self) -> bool {
        self.owner().is_none()
            || self
                .chatgpt_account_id
                .as_deref()
                .is_none_or(|s| s.trim().is_empty())
            || self.refresh_token.trim().is_empty()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: String,
    pub email: Option<String>,
    pub workspace: Option<String>,
    pub plan: Option<String>,
    pub authenticated_at: i64,
    pub reauth_required: bool,
    pub is_default: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFlow {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceFlowPoll {
    Pending,
    Added(AccountInfo),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptOutcome {
    Synchronized { state_changed: bool },
    Adopted,
    ProvablyOlder,
    Ambiguous,
    NotManaged,
}
impl AdoptOutcome {
    pub fn changed(self) -> bool {
        matches!(
            self,
            Self::Adopted
                | Self::Synchronized {
                    state_changed: true
                }
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchAwayGuard {
    ExistingAccount(Option<String>),
    MissingAccount,
}
impl SwitchAwayGuard {
    pub fn ensure_unchanged(&self, env: &Env, account: &str) -> Result<(), AppError> {
        match self {
            Self::ExistingAccount(Some(token)) => live::ensure_live_unchanged(env, account, token)?,
            Self::ExistingAccount(None) if live::read_live_refresh(env, account)?.is_some() => {
                return Err(failure("codex.oauth.live_changed", "准备后出现了新的客户端登录，请重试 / A client login appeared after preparation; retry"));
            }
            _ => {}
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBundle {
    pub workspace: String,
    pub access_token: String,
    pub id_token: Option<String>,
    pub refresh_token: String,
    pub last_refresh: String,
    pub observed_live_refresh: Option<String>,
}
impl TokenBundle {
    pub fn auth_value(&self) -> Value {
        live::managed_auth_value(
            &self.workspace,
            &self.access_token,
            self.id_token.as_deref(),
            &self.refresh_token,
            &self.last_refresh,
        )
    }
}
pub type RefreshResults = Vec<(String, Result<bool, AppError>)>;
#[derive(Default, Serialize, Deserialize)]
struct Accounts {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    accounts: BTreeMap<String, AccountData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_account_id: Option<String>,
}
impl Accounts {
    fn read(path: &Path) -> Result<Self, AppError> {
        let Some(bytes) = read_current(path)? else {
            return Ok(Self::default());
        };
        let mut db: Self = serde_json::from_slice(&bytes).map_err(|e| AppError::json(path, e))?;
        if ![1, 2].contains(&db.version)
            || db
                .accounts
                .iter()
                .any(|(key, a)| key.trim().is_empty() || key != &a.account_id)
        {
            return Err(AppError::Config(
                "账号库的版本或索引无效 / Invalid account database".into(),
            ));
        }
        db.fix_default();
        Ok(db)
    }
    fn fix_default(&mut self) {
        if self
            .default_account_id
            .as_ref()
            .is_none_or(|id| !self.accounts.contains_key(id))
        {
            self.default_account_id = self
                .accounts
                .values()
                .max_by_key(|a| (a.authenticated_at, std::cmp::Reverse(&a.account_id)))
                .map(|a| a.account_id.clone());
        }
    }
    fn info(&self, account: &AccountData) -> AccountInfo {
        AccountInfo {
            id: account.account_id.clone(),
            email: account.email.clone(),
            workspace: account.chatgpt_account_id.clone(),
            plan: account
                .id_token
                .as_deref()
                .and_then(jwt_claims)
                .and_then(|v| claim(&v, "chatgpt_plan_type")),
            authenticated_at: account.authenticated_at,
            reauth_required: account.reauth_required(),
            is_default: self.default_account_id.as_deref() == Some(&account.account_id),
        }
    }
    fn account(&self, id: &str) -> Result<&AccountData, AppError> {
        self.accounts.get(id).ok_or_else(|| {
            failure(
                "codex.oauth.account_not_found",
                "账号已不存在 / Account no longer exists",
            )
        })
    }
}
#[derive(Clone)]
struct CachedAccess {
    account: AccountData,
    token: String,
    until: i64,
    obtained: i64,
}
#[derive(Clone)]
struct DeviceAttempt {
    user: String,
    deadline: i64,
    target: Option<String>,
    polling: bool,
}
#[derive(Default)]
struct Runtime {
    access: HashMap<String, CachedAccess>,
    devices: HashMap<String, DeviceAttempt>,
}
pub struct CodexOAuthManager {
    env: Env,
    transport: Option<Arc<dyn OAuthTransport>>,
    clock: Arc<dyn Clock>,
    serial: Mutex<()>,
    runtime: Mutex<Runtime>,
}
fn held<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
fn failure(key: &'static str, message: &str) -> AppError {
    let (zh, en) = message.split_once(" / ").unwrap_or((message, message));
    AppError::localized(key, zh, en)
}
fn network(error: TransportError) -> AppError {
    match error {
        TransportError::Expired => failure(
            "codex.oauth.device_expired",
            "登录码已取消或过期 / Sign-in code cancelled or expired",
        ),
        TransportError::RefreshTokenInvalid => failure(
            "codex.oauth.refresh_rejected",
            "刷新凭据失效，请重新登录 / Refresh rejected; sign in again",
        ),
        TransportError::Other(message) => AppError::Message(format!("OAuth: {message}")),
    }
}
fn uncertain() -> AppError {
    failure(
        "codex.oauth.ambiguous_generation",
        "无法确定磁盘凭据的新旧，已取消写入 / Credential generation is ambiguous; write cancelled",
    )
}
fn claim(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .or_else(|| {
            value
                .get("https://api.openai.com/auth")
                .and_then(|v| v.get(key))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
fn metadata(tokens: &TokenResponse, key: &str) -> Option<String> {
    tokens
        .id_token
        .as_deref()
        .into_iter()
        .chain(std::iter::once(tokens.access_token.as_str()))
        .filter_map(jwt_claims)
        .find_map(|v| claim(&v, key))
}
fn expiry(now: i64, seconds: i64) -> i64 {
    now.saturating_add(seconds.max(0).saturating_mul(1000))
}
impl CodexOAuthManager {
    pub fn new(env: &Env, transport: Option<Arc<dyn OAuthTransport>>) -> Result<Self, AppError> {
        Accounts::read(&env.data_file(STORE_FILENAME))?;
        Ok(Self {
            env: env.clone(),
            transport,
            clock: Arc::new(SystemClock),
            serial: Mutex::new(()),
            runtime: Mutex::new(Runtime::default()),
        })
    }
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }
    pub fn store_path(&self) -> PathBuf {
        self.env.data_file(STORE_FILENAME)
    }
    fn snapshot(&self) -> Result<Accounts, AppError> {
        Accounts::read(&self.store_path())
    }
    fn write_lock(&self) -> Result<std::fs::File, AppError> {
        let lock = lock_file(&self.env.data_file("codex_oauth_auth.json.lock"), true)?;
        // Complete a previous deletion before another account mutation reads the store.
        // High-level callers also hold the service lock, as for all client writes.
        if crate::state::pending(&self.env.device_store(), Tool::Codex.as_str())?
            .is_some_and(|pending| pending.op == "oauth-remove")
        {
            crate::switch::settle(&self.env, Tool::Codex)?;
        }
        Ok(lock)
    }
    fn save(&self, db: &mut Accounts) -> Result<(), AppError> {
        db.version = STORE_VERSION;
        db.fix_default();
        let bytes =
            serde_json::to_vec_pretty(db).map_err(|source| AppError::JsonSerialize { source })?;
        fsutil::atomic_write_private(&self.store_path(), &bytes)
    }
    fn http(&self) -> Result<&dyn OAuthTransport, AppError> {
        self.transport.as_deref().ok_or_else(|| {
            failure(
                "codex.oauth.offline",
                "当前没有网络传输 / OAuth transport is unavailable",
            )
        })
    }
    pub fn try_list_accounts(&self) -> Result<Vec<AccountInfo>, AppError> {
        let db = self.snapshot()?;
        let mut accounts: Vec<_> = db.accounts.values().map(|a| db.info(a)).collect();
        accounts.sort_by_key(|a| {
            (
                !a.is_default,
                std::cmp::Reverse(a.authenticated_at),
                a.id.clone(),
            )
        });
        Ok(accounts)
    }
    pub fn try_account(&self, id: &str) -> Result<Option<AccountData>, AppError> {
        Ok(self.snapshot()?.accounts.remove(id))
    }
    pub fn try_default_account_id(&self) -> Result<Option<String>, AppError> {
        Ok(self.snapshot()?.default_account_id)
    }
    pub fn list_accounts(&self) -> Vec<AccountInfo> {
        self.try_list_accounts().unwrap_or_default()
    }
    pub fn account(&self, id: &str) -> Option<AccountData> {
        self.try_account(id).ok().flatten()
    }
    pub fn default_account_id(&self) -> Option<String> {
        self.try_default_account_id().ok().flatten()
    }
    pub fn set_default_account(&self, id: &str) -> Result<(), AppError> {
        let _local = held(&self.serial);
        let _disk = self.write_lock()?;
        let mut db = self.snapshot()?;
        db.account(id)?;
        db.default_account_id = Some(id.into());
        self.save(&mut db)
    }
    pub fn start_device_flow(&self, target: Option<&str>) -> Result<DeviceFlow, AppError> {
        if let Some(target) = target {
            self.snapshot()?.account(target)?;
        }
        let code = self.http()?.request_device_code().map_err(network)?;
        let lifetime = if code.expires_in == 0 {
            900
        } else {
            code.expires_in
        };
        let now = self.clock.now_ms();
        let mut runtime = held(&self.runtime);
        runtime.devices.retain(|_, attempt| {
            attempt.deadline > now && (target.is_none() || attempt.target.as_deref() != target)
        });
        runtime.devices.insert(
            code.device_auth_id.clone(),
            DeviceAttempt {
                user: code.user_code.clone(),
                deadline: expiry(now, i64::try_from(lifetime).unwrap_or(i64::MAX)),
                target: target.map(str::to_string),
                polling: false,
            },
        );
        Ok(DeviceFlow {
            device_code: code.device_auth_id,
            user_code: code.user_code,
            verification_uri: transport::endpoints::DEVICE_VERIFICATION_URL.into(),
            expires_in: lifetime,
            interval: code.interval.max(1),
        })
    }
    pub fn cancel_device_flow(&self, code: &str) -> bool {
        held(&self.runtime).devices.remove(code).is_some()
    }
    pub fn poll_device_flow(&self, code: &str) -> Result<DeviceFlowPoll, AppError> {
        self.poll_device_flow_with_guard(code, || Ok(()))
    }
    /// Poll/exchange first; take the caller's guard only when tokens are ready
    /// to persist, before the account-store lock.
    pub fn poll_device_flow_with_guard<G>(
        &self,
        code: &str,
        before_store: impl FnOnce() -> Result<G, AppError>,
    ) -> Result<DeviceFlowPoll, AppError> {
        let attempt = {
            let mut runtime = held(&self.runtime);
            let current = runtime
                .devices
                .get_mut(code)
                .filter(|p| p.deadline > self.clock.now_ms())
                .ok_or_else(|| network(TransportError::Expired))?;
            if current.polling {
                return Ok(DeviceFlowPoll::Pending);
            }
            current.polling = true;
            current.clone()
        };
        let response = (|| {
            let transport = self.http()?;
            match transport
                .poll_device_code(code, &attempt.user)
                .map_err(network)?
            {
                DevicePoll::Pending => Ok(None),
                DevicePoll::Authorized {
                    authorization_code,
                    code_verifier,
                } => transport
                    .exchange_code(&authorization_code, &code_verifier)
                    .map(Some)
                    .map_err(network),
            }
        })();
        let authorized = response.as_ref().is_ok_and(|tokens| tokens.is_some());
        let (response, _guard) = match response {
            Ok(Some(tokens)) => match before_store() {
                Ok(guard) => (Ok(Some(tokens)), Some(guard)),
                Err(error) => (Err(error), None),
            },
            other => (other, None),
        };
        {
            let mut runtime = held(&self.runtime);
            let Some(current) = runtime
                .devices
                .get_mut(code)
                .filter(|p| p.deadline > self.clock.now_ms())
            else {
                return Err(network(TransportError::Expired));
            };
            current.polling = false;
            if authorized {
                runtime.devices.remove(code);
            }
        }
        match response? {
            None => Ok(DeviceFlowPoll::Pending),
            Some(tokens) => self
                .add_account_from_tokens(&tokens, attempt.target.as_deref())
                .map(DeviceFlowPoll::Added),
        }
    }
    pub fn add_account_from_tokens(
        &self,
        tokens: &TokenResponse,
        target: Option<&str>,
    ) -> Result<AccountInfo, AppError> {
        let _local = held(&self.serial);
        let _disk = self.write_lock()?;
        let mut db = self.snapshot()?;
        let workspace = metadata(tokens, "chatgpt_account_id").ok_or_else(|| {
            failure(
                "codex.oauth.missing_workspace",
                "登录结果缺少 workspace / Missing workspace",
            )
        })?;
        let refresh = tokens
            .refresh_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                failure(
                    "codex.oauth.missing_refresh",
                    "登录结果缺少刷新凭据 / Missing refresh token",
                )
            })?;
        if tokens.access_token.trim().is_empty() {
            return Err(failure(
                "codex.oauth.missing_access",
                "登录结果缺少访问凭据 / Missing access token",
            ));
        }
        let email = metadata(tokens, "email");
        let owner = tokens
            .id_token
            .as_deref()
            .and_then(extract_codex_id_token_user_identity);
        let id = if let Some(target) = target {
            let existing = db.account(target)?;
            if existing
                .chatgpt_account_id
                .as_deref()
                .unwrap_or(&existing.account_id)
                != workspace
            {
                return Err(failure(
                    "codex.oauth.reauth_workspace_mismatch",
                    "重新登录的 workspace 不匹配 / Workspace mismatch",
                ));
            }
            let same = match (existing.owner(), &owner) {
                (Some(old), Some(new)) => old == *new,
                (None, Some(_)) => existing
                    .email
                    .as_deref()
                    .zip(email.as_deref())
                    .is_some_and(|(a, b)| a.trim().eq_ignore_ascii_case(b.trim())),
                _ => false,
            };
            if !same {
                return Err(failure(
                    "codex.oauth.reauth_user_mismatch",
                    "重新登录的用户不匹配 / User mismatch",
                ));
            }
            target.to_string()
        } else {
            let mut bytes = [0u8; 16];
            getrandom::fill(&mut bytes).map_err(|_| {
                AppError::Message("无法生成账号 ID / Cannot generate account ID".into())
            })?;
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        };
        if owner.is_some()
            && db.accounts.values().any(|a| {
                a.account_id != id
                    && a.chatgpt_account_id.as_deref() == Some(&workspace)
                    && a.owner() == owner
            })
        {
            return Err(failure(
                "codex.oauth.duplicate_account",
                "此用户和 workspace 已添加 / Account already exists",
            ));
        }
        let now = self.clock.now_ms();
        let account = AccountData {
            account_id: id.clone(),
            chatgpt_account_id: Some(workspace),
            email,
            refresh_token: refresh.into(),
            authenticated_at: now / 1000,
            id_token: tokens.id_token.clone(),
            token_updated_at_ms: now,
        };
        db.accounts.insert(id.clone(), account.clone());
        if db.default_account_id.is_none() {
            db.default_account_id = Some(id.clone());
        }
        self.save(&mut db)?;
        held(&self.runtime).access.insert(
            id,
            CachedAccess {
                account: account.clone(),
                token: tokens.access_token.clone(),
                until: expiry(now, tokens.expires_in.unwrap_or(3600)),
                obtained: now,
            },
        );
        Ok(db.info(&account))
    }
    pub fn import_accounts(
        &self,
        accounts: Vec<AccountData>,
        default: Option<String>,
    ) -> Result<Vec<String>, AppError> {
        let _local = held(&self.serial);
        let _disk = self.write_lock()?;
        let mut db = self.snapshot()?;
        let mut added = Vec::new();
        for account in accounts {
            if account.account_id.trim().is_empty()
                || account.refresh_token.trim().is_empty()
                || db.accounts.contains_key(&account.account_id)
            {
                continue;
            }
            added.push(account.account_id.clone());
            db.accounts.insert(account.account_id.clone(), account);
        }
        if db.default_account_id.is_none() {
            db.default_account_id = default.filter(|id| db.accounts.contains_key(id));
        }
        if !added.is_empty() {
            self.save(&mut db)?;
        }
        Ok(added)
    }
    pub fn read_store_file(path: &Path) -> Result<(Vec<AccountData>, Option<String>), AppError> {
        let db = Accounts::read(path)?;
        Ok((db.accounts.into_values().collect(), db.default_account_id))
    }
    fn remove_accounts(&self, id: Option<&str>) -> Result<(), AppError> {
        let _local = held(&self.serial);
        let _disk = self.write_lock()?;
        let commit =
            |target: &PendingTarget| crate::switch::commit_target(&self.env, Tool::Codex, target);
        let transaction = AppWrite::begin(self.env.device_store(), Tool::Codex.as_str(), &commit)?;
        let before = read_current(&self.store_path())?;
        let mut db = self.snapshot()?;
        let ids = match id {
            Some(id) => {
                db.account(id)?;
                vec![id.to_string()]
            }
            None => db.accounts.keys().cloned().collect(),
        };
        for id in &ids {
            db.accounts.remove(id);
        }
        db.version = STORE_VERSION;
        db.fix_default();
        let store_patch = Guarded {
            expected_pre: digest(before.as_deref()),
            then: if id.is_none() {
                WholeFile::Delete
            } else {
                WholeFile::Write(
                    serde_json::to_vec_pretty(&db)
                        .map_err(|source| AppError::JsonSerialize { source })?,
                )
            },
        };
        let live = live::prepare_account_removal(&self.env, &ids)?;
        // Stage all files before publishing any. The account store is committed first;
        // subsequent failures retain the existing journal for startup/retry recovery.
        let mut changes = vec![FileChange {
            file: LiveFile::private(self.store_path()),
            patch: &store_patch,
        }];
        changes.extend(live.iter().map(|(file, patch)| FileChange {
            file: file.clone(),
            patch,
        }));
        transaction.run("oauth-remove", &changes, PendingTarget::default())?;
        let mut runtime = held(&self.runtime);
        runtime.access.retain(|account, _| !ids.contains(account));
        runtime.devices.retain(|_, attempt| {
            id.is_some()
                && !attempt
                    .target
                    .as_ref()
                    .is_some_and(|target| ids.contains(target))
        });
        Ok(())
    }
    pub fn remove_account(&self, id: &str) -> Result<(), AppError> {
        self.remove_accounts(Some(id))
    }
    pub fn clear_all(&self) -> Result<(), AppError> {
        self.remove_accounts(None)
    }
    fn observed(&self, db: &Accounts, id: &str) -> Result<Option<live::LiveRefresh>, AppError> {
        let account = db.account(id)?;
        let observed = live::read_live_refresh(&self.env, id)?;
        if let Some(live) = &observed {
            if account.chatgpt_account_id.as_deref() != Some(&live.workspace) {
                return Err(failure(
                    "codex.oauth.workspace_mismatch",
                    "磁盘 workspace 不匹配 / Live workspace mismatch",
                ));
            }
            if account.owner().is_some()
                && live
                    .id_token
                    .as_deref()
                    .and_then(extract_codex_id_token_user_identity)
                    != account.owner()
            {
                return Err(failure(
                    "codex.oauth.user_mismatch",
                    "磁盘用户不匹配 / Live user mismatch",
                ));
            }
        }
        Ok(observed)
    }
    fn reconcile(
        &self,
        db: &mut Accounts,
        id: &str,
        observed: &live::LiveRefresh,
        rejected: bool,
    ) -> Result<AdoptOutcome, AppError> {
        let previous = db.account(id)?.clone();
        let mut next = previous.clone();
        let differs = previous.refresh_token != observed.refresh_token
            || observed
                .id_token
                .as_ref()
                .is_some_and(|t| previous.id_token.as_ref() != Some(t));
        let dated = previous.token_updated_at_ms > 0;
        let order = observed
            .last_refresh_ms
            .map(|t| t.cmp(&previous.token_updated_at_ms));
        let outcome = if !differs {
            next.token_updated_at_ms = if dated {
                previous
                    .token_updated_at_ms
                    .max(observed.last_refresh_ms.unwrap_or(0))
            } else {
                observed
                    .last_refresh_ms
                    .filter(|n| *n > 0)
                    .unwrap_or_else(|| self.clock.now_ms())
            };
            AdoptOutcome::Synchronized {
                state_changed: next != previous,
            }
        } else if rejected || (dated && order == Some(std::cmp::Ordering::Greater)) {
            next.refresh_token = observed.refresh_token.clone();
            if observed.id_token.is_some() {
                next.id_token = observed.id_token.clone();
            }
            next.token_updated_at_ms = if rejected {
                observed
                    .last_refresh_ms
                    .filter(|n| *n > previous.token_updated_at_ms)
                    .unwrap_or_else(|| {
                        self.clock
                            .now_ms()
                            .max(previous.token_updated_at_ms.saturating_add(1))
                    })
            } else {
                observed.last_refresh_ms.unwrap()
            };
            AdoptOutcome::Adopted
        } else if dated && order == Some(std::cmp::Ordering::Less) {
            AdoptOutcome::ProvablyOlder
        } else {
            AdoptOutcome::Ambiguous
        };
        if outcome.changed() {
            db.accounts.insert(id.into(), next);
            self.save(db)?;
            if differs {
                held(&self.runtime).access.remove(id);
            }
        }
        Ok(outcome)
    }
    pub fn adopt_live_refresh(&self, id: &str) -> Result<AdoptOutcome, AppError> {
        let _local = held(&self.serial);
        let _disk = self.write_lock()?;
        let mut db = self.snapshot()?;
        match self.observed(&db, id)? {
            Some(observed) => self.reconcile(&mut db, id, &observed, false),
            None => Ok(AdoptOutcome::Synchronized {
                state_changed: false,
            }),
        }
    }
    fn ready<'a>(&self, db: &'a Accounts, id: &str) -> Result<&'a AccountData, AppError> {
        let account = db.account(id)?;
        if account.reauth_required() {
            Err(failure(
                "codex.oauth.reauth_required",
                "账号信息不完整，请重新登录 / Sign in again to complete account identity",
            ))
        } else {
            Ok(account)
        }
    }
    fn resolve(&self, db: &mut Accounts, id: &str, force: bool) -> Result<CachedAccess, AppError> {
        self.ready(db, id)?;
        if let Some(observed) = self.observed(db, id)? {
            self.reconcile(db, id, &observed, false)?;
        }
        if !force {
            if let Some(cached) = held(&self.runtime)
                .access
                .get(id)
                .filter(|cache| {
                    Some(&cache.account) == db.accounts.get(id)
                        && cache.until.saturating_sub(self.clock.now_ms())
                            >= TOKEN_REFRESH_BUFFER_MS
                })
                .cloned()
            {
                return Ok(cached);
            }
        }
        let mut previous = db.account(id)?.clone();
        let response = match self.http()?.refresh(&previous.refresh_token) {
            Err(TransportError::RefreshTokenInvalid) => {
                let observed = self
                    .observed(db, id)?
                    .filter(|v| v.refresh_token != previous.refresh_token)
                    .ok_or_else(|| network(TransportError::RefreshTokenInvalid))?;
                self.reconcile(db, id, &observed, true)?;
                previous = db.account(id)?.clone();
                self.http()?
                    .refresh(&previous.refresh_token)
                    .map_err(network)?
            }
            result => result.map_err(network)?,
        };
        // Never accept a refresh response for a different person or workspace.
        let new_owner = response
            .id_token
            .as_deref()
            .and_then(extract_codex_id_token_user_identity);
        if new_owner
            .as_ref()
            .is_some_and(|id| Some(id) != previous.owner().as_ref())
            || metadata(&response, "chatgpt_account_id")
                .as_ref()
                .is_some_and(|w| Some(w) != previous.chatgpt_account_id.as_ref())
        {
            return Err(failure(
                "codex.oauth.refresh_identity_mismatch",
                "刷新响应的身份不匹配 / Refresh response identity mismatch",
            ));
        }
        if response.access_token.trim().is_empty() {
            return Err(failure(
                "codex.oauth.missing_access",
                "刷新响应缺少访问凭据 / Refresh response has no access token",
            ));
        }
        let now = self.clock.now_ms();
        let mut next = previous.clone();
        if let Some(token) = response.refresh_token.filter(|s| !s.trim().is_empty()) {
            next.refresh_token = token;
        }
        if let Some(token) = response.id_token.filter(|s| !s.trim().is_empty()) {
            next.id_token = Some(token);
        }
        next.token_updated_at_ms = now;
        db.accounts.insert(id.into(), next.clone());
        self.save(db)?;
        let cached = CachedAccess {
            account: next,
            token: response.access_token,
            until: expiry(now, response.expires_in.unwrap_or(3600)),
            obtained: now,
        };
        held(&self.runtime).access.insert(id.into(), cached.clone());
        let bundle = self.bundle(&cached, None);
        live::sync_live_after_refresh(&self.env,id,&previous.refresh_token,&bundle.auth_value()).map_err(|_|failure("codex.oauth.live_sync_failed","新凭据已保存，但客户端同步失败；请重新切换账号 / New credentials saved; client sync failed. Switch the account again"))?;
        Ok(cached)
    }
    fn bundle(&self, cache: &CachedAccess, observed: Option<String>) -> TokenBundle {
        TokenBundle {
            workspace: cache.account.chatgpt_account_id.clone().unwrap_or_default(),
            access_token: cache.token.clone(),
            id_token: cache.account.id_token.clone(),
            refresh_token: cache.account.refresh_token.clone(),
            last_refresh: live::format_last_refresh(cache.obtained),
            observed_live_refresh: observed,
        }
    }
    pub fn token_bundle(&self, id: &str) -> Result<TokenBundle, AppError> {
        let _local = held(&self.serial);
        let _disk = self.write_lock()?;
        let mut db = self.snapshot()?;
        let cache = self.resolve(&mut db, id, false)?;
        let observed = self.observed(&db, id)?;
        if let Some(observed) = &observed {
            match self.reconcile(&mut db, id, observed, false)? {
                AdoptOutcome::Ambiguous => return Err(uncertain()),
                AdoptOutcome::Adopted => return Err(failure(
                    "codex.oauth.refreshed_while_preparing",
                    "准备期间凭据已刷新，请重试 / Credentials refreshed during preparation; retry",
                )),
                _ => {}
            }
        }
        Ok(self.bundle(&cache, observed.map(|v| v.refresh_token)))
    }
    pub fn prepare_switch_away(&self, id: &str) -> Result<SwitchAwayGuard, AppError> {
        let _local = held(&self.serial);
        let _disk = self.write_lock()?;
        let mut db = self.snapshot()?;
        if !db.accounts.contains_key(id) {
            return Ok(SwitchAwayGuard::MissingAccount);
        }
        let observed = self.observed(&db, id)?;
        if let Some(observed) = &observed {
            if self.reconcile(&mut db, id, observed, false)? == AdoptOutcome::Ambiguous {
                return Err(uncertain());
            }
        }
        Ok(SwitchAwayGuard::ExistingAccount(
            observed.map(|v| v.refresh_token),
        ))
    }
    pub fn refresh_account(&self, id: &str) -> Result<(), AppError> {
        let _local = held(&self.serial);
        let _disk = self.write_lock()?;
        let mut db = self.snapshot()?;
        self.resolve(&mut db, id, true).map(|_| ())
    }
    pub fn refresh_stale(&self, max_age_ms: i64) -> Result<RefreshResults, AppError> {
        let due: Vec<_> = self
            .snapshot()?
            .accounts
            .into_values()
            .filter(|a| {
                !a.reauth_required()
                    && self.clock.now_ms().saturating_sub(a.token_updated_at_ms) >= max_age_ms
            })
            .map(|a| a.account_id)
            .collect();
        Ok(due
            .into_iter()
            .map(|id| {
                let result = (|| {
                    let _local = held(&self.serial);
                    let _disk = self.write_lock()?;
                    let mut db = self.snapshot()?;
                    if db.accounts.get(&id).is_none_or(|a| {
                        self.clock.now_ms().saturating_sub(a.token_updated_at_ms) < max_age_ms
                    }) {
                        return Ok(false);
                    }
                    self.resolve(&mut db, &id, true).map(|_| true)
                })();
                (id, result)
            })
            .collect())
    }
    #[doc(hidden)]
    pub fn test_cache_access_token(&self, id: &str, token: &str, expires_at_ms: i64) {
        if let Some(account) = self.account(id) {
            held(&self.runtime).access.insert(
                id.into(),
                CachedAccess {
                    account,
                    token: token.into(),
                    until: expires_at_ms,
                    obtained: self.clock.now_ms(),
                },
            );
        }
    }
    #[doc(hidden)]
    pub fn test_forget_access_tokens(&self) {
        held(&self.runtime).access.clear();
    }
}

#[cfg(test)]
mod removal_tests;
