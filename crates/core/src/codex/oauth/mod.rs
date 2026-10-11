//! 托管的 ChatGPT 账号（多账号）：设备码登录、refresh token 轮换和与 Codex CLI 共享
//! 同一条 token 链时的代数判定。
//!
//! 移植自 cc-switch `proxy/providers/codex_oauth_auth.rs` 的 `CodexOAuthManager`，改成
//! 同步、不联网（网络在 [`transport::OAuthTransport`] 后面），保留的保护：
//! - **refresh token 采纳**（[`AdoptOutcome`]）：托管账号写进 `auth.json` 后 Codex CLI 会
//!   自己刷新、轮换 refresh token。切走或再次写入前先把盘上的那一代读回来：内容不同时，
//!   只有 live 的 `last_refresh` *严格晚于* 管理器记的那一代才采纳；严格更早算「可证明
//!   更旧」；相等、缺时间戳、管理器没有时间戳的都是「不确定」，要覆盖或删除 live 的
//!   调用方必须停下，不猜；
//! - 服务端拒绝管理器的 refresh token 时，盘上同账号的另一代是唯一可能有效的，例外地
//!   直接采纳再试一次；
//! - 刷新响应回来时管理器的 refresh token 已经变了（并发采纳），丢弃这个响应；
//! - 生成写入 bundle 时在同一把代数锁里再核对一次盘上的代数：期间被 CLI 刷新了就停下，
//!   绝不写出「A0 的 access + R1 的 refresh」这种混合 bundle；
//! - 切走前记下采纳时盘上的 refresh token（[`SwitchAwayGuard`]），写锁里再比一次；
//! - 删除账号时连同它在 live 的登录和标记一起清掉（按标记认，不按 access token 指纹）；
//! - 同一 workspace + 同一用户不能添加两次；同一个人的 Plus 和 Team 是两个账号。
//!
//! 和原版的差异：账号库坏了直接报错，不当成空库（原版会在下次保存时覆盖掉）。

pub mod live;
#[doc(hidden)]
pub mod testing;
pub mod transport;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codex::auth::{
    extract_codex_id_token_subject, extract_codex_id_token_user_identity, jwt_claims,
};
use crate::env::Env;
use crate::error::AppError;
use crate::live::engine::read_current;
use transport::{Clock, DevicePoll, OAuthTransport, SystemClock, TokenResponse, TransportError};

/// 账号库文件（和 cc-switch 同名、同格式 v2，放在 ccswitch-lite 的数据目录里）。
pub const STORE_FILENAME: &str = "codex_oauth_auth.json";
pub const STORE_VERSION: u32 = 2;
/// access token 离过期不到这么久就刷新。
pub const TOKEN_REFRESH_BUFFER_MS: i64 = 60_000;
const DEFAULT_EXPIRES_IN_SECS: i64 = 3600;
const DEVICE_CODE_DEFAULT_EXPIRES_IN: u64 = 900;

/// 持久化的账号。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountData {
    /// 本地稳定账号 id（也是库里的 key，卡上 `authBinding.accountId` 引用它）。
    pub account_id: String,
    /// ChatGPT workspace id（写进 `auth.json` 的 `tokens.account_id`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub refresh_token: String,
    /// 登录时间（秒）。
    pub authenticated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    /// 最近一次取得或采纳这组 token 的时间（毫秒），代数判定用。
    #[serde(default)]
    pub token_updated_at_ms: i64,
}

impl AccountData {
    fn user_identity(&self) -> Option<String> {
        self.id_token
            .as_deref()
            .and_then(extract_codex_id_token_user_identity)
    }

    fn workspace(&self) -> &str {
        self.chatgpt_account_id
            .as_deref()
            .unwrap_or(self.account_id.as_str())
    }

    /// 缺少能证明身份的 id_token 或 workspace：要重新登录才能参与托管写入。
    pub fn reauth_required(&self) -> bool {
        self.user_identity().is_none() || self.chatgpt_account_id.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    accounts: BTreeMap<String, AccountData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_account_id: Option<String>,
}

/// 给界面看的账号信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: String,
    pub email: Option<String>,
    pub workspace: Option<String>,
    /// id_token 里的套餐（plus、team、pro……）。
    pub plan: Option<String>,
    pub authenticated_at: i64,
    pub reauth_required: bool,
    pub is_default: bool,
}

/// 设备码登录开始后给界面看的东西。
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

/// 采纳盘上那一代 refresh token 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptOutcome {
    /// 盘上和管理器是同一代（`state_changed` 只反映时间戳记账）。
    Synchronized { state_changed: bool },
    /// 盘上不同、且可证明更新：采纳了。
    Adopted,
    /// 盘上不同、且可证明更旧：可以覆盖或删除。
    ProvablyOlder,
    /// 盘上不同、分不出新旧：要覆盖或删除 live 的调用方必须停下。
    Ambiguous,
    /// 不是这个管理器的账号。
    NotManaged,
}

impl AdoptOutcome {
    fn state_changed(self) -> bool {
        matches!(
            self,
            Self::Synchronized {
                state_changed: true
            } | Self::Adopted
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdoptMode {
    TimestampChecked,
    /// 服务端刚拒绝了管理器的 refresh token：盘上不同的那一代是唯一的出路。
    RejectedManagerToken,
}

/// 切走托管账号前准备好的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchAwayGuard {
    /// 账号还在；带着采纳时盘上的 refresh token（盘上不是它的登录时为 `None`）。
    ExistingAccount(Option<String>),
    /// 账号已经删了：它不再拥有 live，留下的标记要清掉。
    MissingAccount,
}

impl SwitchAwayGuard {
    /// 写锁里再核对一次：采纳之后 Codex CLI 又刷新了就停下。
    pub fn ensure_unchanged(&self, env: &Env, account_id: &str) -> Result<(), AppError> {
        if let Self::ExistingAccount(Some(token)) = self {
            live::ensure_live_unchanged(env, account_id, token)?;
        }
        Ok(())
    }
}

/// 写进托管 `auth.json` 的完整 token 束。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBundle {
    pub workspace: String,
    pub access_token: String,
    pub id_token: Option<String>,
    pub refresh_token: String,
    /// access token 真正取得的时间（RFC3339），不是写盘时间。
    pub last_refresh: String,
    /// 生成 bundle 时（代数锁里最后一次核对）live 里这个账号的 refresh token（live 不是
    /// 它的登录时为 `None`）。写锁里再比一次，变了就停下。
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

#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    expires_at_ms: i64,
    obtained_at_ms: i64,
}

#[derive(Debug, Clone)]
struct PendingDevice {
    user_code: String,
    expires_at_ms: i64,
    target_account_id: Option<String>,
}

#[derive(Default)]
struct Inner {
    accounts: BTreeMap<String, AccountData>,
    default_account_id: Option<String>,
    tokens: HashMap<String, CachedToken>,
    pending: HashMap<String, PendingDevice>,
}

/// 批量保活结果：每个账号是否实际刷新，或该账号自己的失败原因。
pub type RefreshResults = Vec<(String, Result<bool, AppError>)>;

/// 托管 ChatGPT 账号管理器。
pub struct CodexOAuthManager {
    env: Env,
    transport: Option<Arc<dyn OAuthTransport>>,
    clock: Arc<dyn Clock>,
    /// 锁顺序：进程内 generation → 账号库文件锁 → Codex live 写锁。
    /// 刷新持有前两把锁跨网络调用，确保 App 和 CLI 不会消费同一代 refresh token。
    generation: Mutex<()>,
    /// 内存状态，只在短时间内持有，不跨网络调用。
    inner: Mutex<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn not_found(account_id: &str) -> AppError {
    AppError::localized(
        "codex.oauth.account_not_found",
        format!("找不到托管的 ChatGPT 账号 {account_id}（可能已被删除），请在账号管理里重新登录"),
        format!("Managed ChatGPT account {account_id} was not found (it may have been removed). Sign in again in account management"),
    )
}

fn transport_error(err: TransportError) -> AppError {
    match err {
        TransportError::RefreshTokenInvalid => AppError::localized(
            "codex.oauth.refresh_rejected",
            "ChatGPT 拒绝了这个账号的 refresh token（已过期或已在别处轮换），请重新登录这个账号",
            "ChatGPT rejected this account's refresh token (expired or rotated elsewhere). Sign in to this account again",
        ),
        TransportError::Expired => AppError::localized(
            "codex.oauth.device_expired",
            "登录码已过期或已取消，请重新开始登录",
            "The sign-in code expired or was cancelled. Start again",
        ),
        TransportError::Other(message) => AppError::Message(format!("ChatGPT OAuth: {message}")),
    }
}

fn ambiguous(account_id: &str) -> AppError {
    AppError::localized(
        "codex.oauth.ambiguous_generation",
        format!("Codex CLI 账号 {account_id} 的磁盘凭据已变化，但无法安全判断 refresh token 新旧；为避免覆盖或删除有效登录，本次操作已取消。请重新登录这个账号"),
        format!("The on-disk credentials of Codex account {account_id} changed, but it cannot be told safely whether they are newer; to avoid overwriting or deleting a valid login nothing was changed. Sign in to this account again"),
    )
}

/// 从 id_token（没有时 access_token）里取 workspace 和邮箱。
fn token_metadata(tokens: &TokenResponse) -> (Option<String>, Option<String>) {
    let claims_of = |token: &str| jwt_claims(token).unwrap_or(Value::Null);
    let workspace_of = |claims: &Value| {
        claims
            .get("chatgpt_account_id")
            .or_else(|| claims.pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let email_of = |claims: &Value| {
        claims
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let id_claims = tokens
        .id_token
        .as_deref()
        .map(claims_of)
        .unwrap_or(Value::Null);
    let access_claims = claims_of(&tokens.access_token);
    (
        workspace_of(&id_claims).or_else(|| workspace_of(&access_claims)),
        email_of(&id_claims).or_else(|| email_of(&access_claims)),
    )
}

fn plan_of(id_token: Option<&str>) -> Option<String> {
    let claims = jwt_claims(id_token?)?;
    claims
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_plan_type")
        .or_else(|| claims.get("chatgpt_plan_type"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn random_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // 几乎不会发生；退回时间戳，仍然只在本机唯一即可。
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        bytes.copy_from_slice(&nanos.to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl CodexOAuthManager {
    /// 读账号库。`transport` 为 `None` 时不能登录和刷新（核心库测试、离线工具用）。
    pub fn new(env: &Env, transport: Option<Arc<dyn OAuthTransport>>) -> Result<Self, AppError> {
        let manager = Self {
            env: env.clone(),
            transport,
            clock: Arc::new(SystemClock),
            generation: Mutex::new(()),
            inner: Mutex::new(Inner::default()),
        };
        manager.reload()?;
        Ok(manager)
    }

    /// 换时钟（测试用）。
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn store_path(&self) -> std::path::PathBuf {
        self.env.data_file(STORE_FILENAME)
    }

    /// 调用方先持有 generation；只在要写入/刷新时创建跨进程锁文件。
    fn lock_store(&self) -> Result<std::fs::File, AppError> {
        let guard = crate::live::engine::lock_file(
            &self.env.data_file("codex_oauth_auth.json.lock"),
            true,
        )?;
        self.reload()?;
        Ok(guard)
    }

    fn read_snapshot(path: &std::path::Path) -> Result<StoreFile, AppError> {
        let Some(bytes) = read_current(path)? else {
            return Ok(StoreFile::default());
        };
        let store: StoreFile =
            serde_json::from_slice(&bytes).map_err(|source| AppError::json(path, source))?;
        if !matches!(store.version, 1 | 2)
            || store
                .accounts
                .iter()
                .any(|(key, account)| key.trim().is_empty() || *key != account.account_id)
        {
            return Err(AppError::Config(format!(
                "Codex 账号库 {} 的版本或索引无效，没有加载也不会覆盖它",
                path.display()
            )));
        }
        Ok(store)
    }

    /// 原子文件读快照，不创建目录或锁文件。调用方持有 generation（或尚未发布实例）。
    fn reload(&self) -> Result<(), AppError> {
        let store = Self::read_snapshot(&self.store_path())?;
        let mut inner = lock(&self.inner);
        let Inner {
            accounts, tokens, ..
        } = &mut *inner;
        tokens.retain(|id, _| {
            store
                .accounts
                .get(id)
                .is_some_and(|account| accounts.get(id) == Some(account))
        });
        inner.default_account_id = store
            .default_account_id
            .filter(|id| store.accounts.contains_key(id))
            .or_else(|| Self::fallback_default(&store.accounts));
        inner.accounts = store.accounts;
        Ok(())
    }

    fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    fn transport(&self) -> Result<&dyn OAuthTransport, AppError> {
        self.transport.as_deref().ok_or_else(|| {
            AppError::localized(
                "codex.oauth.offline",
                "没有可用的网络通道，无法登录或刷新 ChatGPT 账号",
                "No network transport is available to sign in or refresh ChatGPT accounts",
            )
        })
    }

    fn fallback_default(accounts: &BTreeMap<String, AccountData>) -> Option<String> {
        accounts
            .values()
            .max_by(|a, b| {
                a.authenticated_at
                    .cmp(&b.authenticated_at)
                    .then_with(|| b.account_id.cmp(&a.account_id))
            })
            .map(|account| account.account_id.clone())
    }

    fn save(&self, inner: &Inner) -> Result<(), AppError> {
        Self::save_snapshot(
            &self.store_path(),
            &inner.accounts,
            inner.default_account_id.clone(),
        )
    }

    fn save_snapshot(
        path: &std::path::Path,
        accounts: &BTreeMap<String, AccountData>,
        default_account_id: Option<String>,
    ) -> Result<(), AppError> {
        let store = StoreFile {
            version: STORE_VERSION,
            accounts: accounts.clone(),
            default_account_id,
        };
        let bytes = serde_json::to_vec_pretty(&store)
            .map_err(|source| AppError::JsonSerialize { source })?;
        crate::fsutil::atomic_write_private(path, &bytes)
    }

    fn info(&self, inner: &Inner, account: &AccountData) -> AccountInfo {
        AccountInfo {
            id: account.account_id.clone(),
            email: account.email.clone(),
            workspace: account.chatgpt_account_id.clone(),
            plan: plan_of(account.id_token.as_deref()),
            authenticated_at: account.authenticated_at,
            reauth_required: account.reauth_required(),
            is_default: inner.default_account_id.as_deref() == Some(account.account_id.as_str()),
        }
    }

    /// 最新的账号摘要；读盘失败时保留错误，不返回旧缓存。
    pub fn try_list_accounts(&self) -> Result<Vec<AccountInfo>, AppError> {
        let _generation = lock(&self.generation);
        self.reload()?;
        Ok(self.list_accounts())
    }

    pub fn try_account(&self, account_id: &str) -> Result<Option<AccountData>, AppError> {
        let _generation = lock(&self.generation);
        self.reload()?;
        Ok(self.account(account_id))
    }

    pub fn try_default_account_id(&self) -> Result<Option<String>, AppError> {
        let _generation = lock(&self.generation);
        self.reload()?;
        Ok(self.default_account_id())
    }

    /// 当前进程的缓存快照。用户可见查询应使用 [`Self::try_list_accounts`]。
    pub fn list_accounts(&self) -> Vec<AccountInfo> {
        let inner = lock(&self.inner);
        let mut accounts: Vec<AccountInfo> = inner
            .accounts
            .values()
            .map(|account| self.info(&inner, account))
            .collect();
        accounts.sort_by(|a, b| {
            b.is_default
                .cmp(&a.is_default)
                .then(b.authenticated_at.cmp(&a.authenticated_at))
                .then(a.id.cmp(&b.id))
        });
        accounts
    }

    /// 缓存快照；需要读取其他进程的更新时使用 [`Self::try_account`]。
    pub fn account(&self, account_id: &str) -> Option<AccountData> {
        lock(&self.inner).accounts.get(account_id).cloned()
    }

    /// 缓存快照；用户可见查询应使用 [`Self::try_default_account_id`]。
    pub fn default_account_id(&self) -> Option<String> {
        lock(&self.inner).default_account_id.clone()
    }

    pub fn set_default_account(&self, account_id: &str) -> Result<(), AppError> {
        let _generation = lock(&self.generation);
        let _store = self.lock_store()?;
        let mut inner = lock(&self.inner);
        if !inner.accounts.contains_key(account_id) {
            return Err(not_found(account_id));
        }
        let default = Some(account_id.to_string());
        Self::save_snapshot(&self.store_path(), &inner.accounts, default.clone())?;
        inner.default_account_id = default;
        Ok(())
    }

    // ==================== 设备码登录 ====================

    /// 开始登录一个新账号。`target_account_id` 为重新登录已有账号（原位更新，卡上的
    /// 绑定不变；workspace 和用户必须对得上）。
    pub fn start_device_flow(
        &self,
        target_account_id: Option<&str>,
    ) -> Result<DeviceFlow, AppError> {
        if let Some(target) = target_account_id {
            if self.try_account(target)?.is_none() {
                return Err(not_found(target));
            }
        }
        let code = self
            .transport()?
            .request_device_code()
            .map_err(transport_error)?;
        let expires_in = if code.expires_in == 0 {
            DEVICE_CODE_DEFAULT_EXPIRES_IN
        } else {
            code.expires_in
        };
        let now = self.now_ms();
        let mut inner = lock(&self.inner);
        inner
            .pending
            .retain(|_, pending| pending.expires_at_ms > now);
        // 同一个目标账号只认最新的一次重新登录。
        if let Some(target) = target_account_id {
            inner
                .pending
                .retain(|_, pending| pending.target_account_id.as_deref() != Some(target));
        }
        inner.pending.insert(
            code.device_auth_id.clone(),
            PendingDevice {
                user_code: code.user_code.clone(),
                expires_at_ms: now + expires_in as i64 * 1000,
                target_account_id: target_account_id.map(str::to_string),
            },
        );
        Ok(DeviceFlow {
            device_code: code.device_auth_id,
            user_code: code.user_code,
            verification_uri: transport::endpoints::DEVICE_VERIFICATION_URL.to_string(),
            expires_in,
            interval: code.interval.max(1),
        })
    }

    pub fn cancel_device_flow(&self, device_code: &str) -> bool {
        lock(&self.inner).pending.remove(device_code).is_some()
    }

    /// 轮询一次。用户确认后换 token、加账号。
    pub fn poll_device_flow(&self, device_code: &str) -> Result<DeviceFlowPoll, AppError> {
        let pending = {
            let inner = lock(&self.inner);
            inner.pending.get(device_code).cloned()
        }
        .filter(|pending| pending.expires_at_ms > self.now_ms())
        .ok_or_else(|| transport_error(TransportError::Expired))?;
        let transport = self.transport()?;
        let (code, verifier) = match transport
            .poll_device_code(device_code, &pending.user_code)
            .map_err(transport_error)?
        {
            DevicePoll::Pending => return Ok(DeviceFlowPoll::Pending),
            DevicePoll::Authorized {
                authorization_code,
                code_verifier,
            } => (authorization_code, code_verifier),
        };
        let tokens = transport
            .exchange_code(&code, &verifier)
            .map_err(transport_error)?;
        // 换 token 期间被取消、过期或被更新的一次重新登录取代：不提交。
        {
            let mut inner = lock(&self.inner);
            let still = inner
                .pending
                .get(device_code)
                .is_some_and(|p| p.expires_at_ms > self.now_ms());
            if !still {
                return Err(transport_error(TransportError::Expired));
            }
            inner.pending.remove(device_code);
        }
        self.add_account_from_tokens(&tokens, pending.target_account_id.as_deref())
            .map(DeviceFlowPoll::Added)
    }

    /// 用一组刚取得的 token 加账号（或原位更新 `target_account_id`）。
    pub fn add_account_from_tokens(
        &self,
        tokens: &TokenResponse,
        target_account_id: Option<&str>,
    ) -> Result<AccountInfo, AppError> {
        let _generation = lock(&self.generation);
        let _store = self.lock_store()?;
        let (workspace, email) = token_metadata(tokens);
        let workspace = workspace.ok_or_else(|| {
            AppError::localized(
                "codex.oauth.missing_workspace",
                "登录结果里没有 ChatGPT workspace，无法添加这个账号",
                "The sign-in result has no ChatGPT workspace, so the account cannot be added",
            )
        })?;
        let refresh_token = tokens
            .refresh_token
            .clone()
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| AppError::Message("登录结果里没有 refresh token".into()))?;
        let id_token = tokens.id_token.clone().filter(|t| !t.trim().is_empty());
        let now_ms = self.now_ms();
        let mut inner = lock(&self.inner);

        let account_id = match target_account_id {
            Some(target) => {
                let existing = inner
                    .accounts
                    .get(target)
                    .ok_or_else(|| not_found(target))?;
                if existing.workspace() != workspace {
                    return Err(AppError::localized(
                        "codex.oauth.reauth_workspace_mismatch",
                        format!("重新登录的 ChatGPT workspace 和账号 {target} 不一致，原账号保持不变"),
                        format!("The workspace signed in does not match account {target}; the account was left unchanged"),
                    ));
                }
                let new_subject = id_token.as_deref().and_then(extract_codex_id_token_subject);
                let old_subject = existing
                    .id_token
                    .as_deref()
                    .and_then(extract_codex_id_token_subject);
                let same_user = match (&old_subject, &new_subject) {
                    (Some(old), Some(new)) => old == new,
                    (None, Some(_)) => matches!(
                        (existing.email.as_deref(), email.as_deref()),
                        (Some(a), Some(b)) if a.trim().eq_ignore_ascii_case(b.trim())
                    ),
                    _ => false,
                };
                if !same_user {
                    return Err(AppError::localized(
                        "codex.oauth.reauth_user_mismatch",
                        format!("无法确认重新登录的 ChatGPT 用户属于账号 {target}，原账号保持不变"),
                        format!("Cannot confirm the user signed in belongs to account {target}; the account was left unchanged"),
                    ));
                }
                target.to_string()
            }
            None => random_id(),
        };

        let data = AccountData {
            account_id: account_id.clone(),
            chatgpt_account_id: Some(workspace.clone()),
            email,
            refresh_token,
            authenticated_at: now_ms / 1000,
            id_token,
            token_updated_at_ms: now_ms,
        };
        if let Some(identity) = data.user_identity() {
            let duplicate = inner.accounts.values().any(|existing| {
                existing.account_id != account_id
                    && existing.workspace() == workspace
                    && existing.user_identity().as_deref() == Some(identity.as_str())
            });
            if duplicate {
                return Err(AppError::localized(
                    "codex.oauth.duplicate_account",
                    "这个 ChatGPT 账号（同一用户、同一 workspace）已经添加过了",
                    "This ChatGPT account (same user and workspace) has already been added",
                ));
            }
        }
        // 先写盘再对外可见：写失败时内存和缓存都不变。
        let mut accounts = inner.accounts.clone();
        accounts.insert(account_id.clone(), data.clone());
        let default = inner
            .default_account_id
            .clone()
            .or_else(|| Some(account_id.clone()));
        Self::save_snapshot(&self.store_path(), &accounts, default.clone())?;
        inner.accounts = accounts;
        inner.default_account_id = default;
        inner.tokens.insert(
            account_id.clone(),
            CachedToken {
                token: tokens.access_token.clone(),
                expires_at_ms: now_ms + tokens.expires_in.unwrap_or(DEFAULT_EXPIRES_IN_SECS) * 1000,
                obtained_at_ms: now_ms,
            },
        );
        let account = inner.accounts.get(&account_id).expect("just inserted");
        Ok(self.info(&inner, account))
    }

    // ==================== 删除 ====================

    /// 删除一个账号，连同它在 live 的登录和标记。live 清理失败时账号保留。
    pub fn remove_account(&self, account_id: &str) -> Result<(), AppError> {
        let _generation = lock(&self.generation);
        let _store = self.lock_store()?;
        self.remove_locked(account_id)
    }

    fn remove_locked(&self, account_id: &str) -> Result<(), AppError> {
        if !lock(&self.inner).accounts.contains_key(account_id) {
            return Err(not_found(account_id));
        }
        // 先补完 Codex 未完成的切换，再按落定的 live 清理。
        crate::switch::settle(&self.env, crate::provider::Tool::Codex)?;
        live::clear_live_for_account(&self.env, account_id)?;
        let mut inner = lock(&self.inner);
        let mut accounts = inner.accounts.clone();
        accounts.remove(account_id);
        let default = match inner.default_account_id.as_deref() {
            Some(id) if id == account_id => Self::fallback_default(&accounts),
            other => other.map(str::to_string),
        };
        Self::save_snapshot(&self.store_path(), &accounts, default.clone())?;
        inner.accounts = accounts;
        inner.default_account_id = default;
        inner.tokens.remove(account_id);
        inner
            .pending
            .retain(|_, pending| pending.target_account_id.as_deref() != Some(account_id));
        Ok(())
    }

    /// 退出全部账号。
    pub fn clear_all(&self) -> Result<(), AppError> {
        let _generation = lock(&self.generation);
        let _store = self.lock_store()?;
        let ids: Vec<String> = lock(&self.inner).accounts.keys().cloned().collect();
        for id in ids {
            self.remove_locked(&id)?;
        }
        crate::fsutil::delete_file(&self.store_path())?;
        lock(&self.inner).pending.clear();
        Ok(())
    }

    // ==================== 代数判定 ====================

    fn ensure_ready(&self, account_id: &str) -> Result<(), AppError> {
        let inner = lock(&self.inner);
        let account = inner
            .accounts
            .get(account_id)
            .ok_or_else(|| not_found(account_id))?;
        if account.reauth_required() {
            return Err(AppError::localized(
                "codex.oauth.reauth_required",
                format!("账号 {account_id} 缺少可证明的用户身份或 workspace，请重新登录"),
                format!("Account {account_id} has no provable user identity or workspace. Sign in again"),
            ));
        }
        Ok(())
    }

    /// 读回 live 里属于这个账号的那一代（workspace 对不上就停下）。
    fn live_generation(&self, account_id: &str) -> Result<Option<live::LiveRefresh>, AppError> {
        let workspace = {
            let inner = lock(&self.inner);
            let account = inner
                .accounts
                .get(account_id)
                .ok_or_else(|| not_found(account_id))?;
            account.chatgpt_account_id.clone()
        };
        let Some(live) = live::read_live_refresh(&self.env, account_id)? else {
            return Ok(None);
        };
        if workspace.as_deref() != Some(live.workspace.as_str()) {
            return Err(AppError::localized(
                "codex.oauth.workspace_mismatch",
                format!("Codex 托管账号 {account_id} 的 workspace 和磁盘凭据不一致，本次操作已取消"),
                format!("The workspace of managed Codex account {account_id} does not match the on-disk credentials; nothing was changed"),
            ));
        }
        Ok(Some(live))
    }

    /// 把盘上那一代采纳进管理器（调用方持有代数锁）。
    fn adopt_locked(
        &self,
        account_id: &str,
        live: &live::LiveRefresh,
        mode: AdoptMode,
    ) -> Result<AdoptOutcome, AppError> {
        let refresh_token = live.refresh_token.trim().to_string();
        let incoming_id = live.id_token.clone().filter(|t| !t.trim().is_empty());
        let observed = live.last_refresh_ms;
        let mut inner = lock(&self.inner);
        let Some(account) = inner.accounts.get_mut(account_id) else {
            return Ok(AdoptOutcome::NotManaged);
        };
        let refresh_changed = account.refresh_token != refresh_token;
        let id_changed = incoming_id
            .as_ref()
            .is_some_and(|token| account.id_token.as_deref() != Some(token.as_str()));
        let material_changed = refresh_changed || id_changed;
        let undated = account.token_updated_at_ms <= 0;
        let order = observed.map(|o| o.cmp(&account.token_updated_at_ms));
        let should_adopt = material_changed
            && (mode == AdoptMode::RejectedManagerToken
                || (!undated && order == Some(std::cmp::Ordering::Greater)));

        let mut changed = false;
        let mut replaced = false;
        let mut outcome = if !material_changed {
            AdoptOutcome::Synchronized {
                state_changed: false,
            }
        } else if should_adopt {
            if refresh_changed {
                account.refresh_token = refresh_token;
                changed = true;
                replaced = true;
            }
            if let Some(id) = incoming_id {
                if account.id_token.as_deref() != Some(id.as_str()) {
                    account.id_token = Some(id);
                    changed = true;
                    replaced = true;
                }
            }
            AdoptOutcome::Adopted
        } else if !undated && order == Some(std::cmp::Ordering::Less) {
            AdoptOutcome::ProvablyOlder
        } else {
            AdoptOutcome::Ambiguous
        };

        match outcome {
            AdoptOutcome::Adopted if mode == AdoptMode::RejectedManagerToken => {
                let at = observed
                    .filter(|o| *o > account.token_updated_at_ms)
                    .unwrap_or_else(|| {
                        self.now_ms()
                            .max(account.token_updated_at_ms.saturating_add(1))
                    });
                if account.token_updated_at_ms != at {
                    account.token_updated_at_ms = at;
                    changed = true;
                }
            }
            AdoptOutcome::Adopted => {
                if let Some(o) = observed {
                    if account.token_updated_at_ms != o {
                        account.token_updated_at_ms = o;
                        changed = true;
                    }
                }
            }
            AdoptOutcome::Synchronized { .. } => {
                if undated {
                    // 内容相同确立了同一代：补上时间戳，下次分得出新旧。
                    account.token_updated_at_ms =
                        observed.filter(|o| *o > 0).unwrap_or_else(|| self.now_ms());
                    changed = true;
                } else if let Some(o) = observed {
                    if o > account.token_updated_at_ms {
                        account.token_updated_at_ms = o;
                        changed = true;
                    }
                }
            }
            _ => {}
        }
        if replaced {
            // 旧 access token 可能已被服务端作废，下次按新 refresh token 重取。
            inner.tokens.remove(account_id);
        }
        if let AdoptOutcome::Synchronized { .. } = outcome {
            outcome = AdoptOutcome::Synchronized {
                state_changed: changed,
            };
        }
        if changed {
            self.save(&inner)?;
        }
        Ok(outcome)
    }

    /// 采纳 Codex CLI 轮换过的 refresh token。返回管理器状态是否变了。
    pub fn adopt_live_refresh(&self, account_id: &str) -> Result<AdoptOutcome, AppError> {
        let _generation = lock(&self.generation);
        let _store = self.lock_store()?;
        match self.live_generation(account_id)? {
            Some(live) => self.adopt_locked(account_id, &live, AdoptMode::TimestampChecked),
            None => Ok(AdoptOutcome::Synchronized {
                state_changed: false,
            }),
        }
    }

    /// 有效的 access token（持代数锁调用）：先采纳盘上的新一代，缓存够新就用，否则刷新。
    fn resolve_locked(&self, account_id: &str) -> Result<CachedToken, AppError> {
        if let Some(live) = self.live_generation(account_id)? {
            self.adopt_locked(account_id, &live, AdoptMode::TimestampChecked)?;
        }
        let mut refresh_token = {
            let inner = lock(&self.inner);
            let account = inner
                .accounts
                .get(account_id)
                .ok_or_else(|| not_found(account_id))?;
            if let Some(cached) = inner.tokens.get(account_id) {
                if cached.expires_at_ms - self.now_ms() >= TOKEN_REFRESH_BUFFER_MS {
                    return Ok(cached.clone());
                }
            }
            account.refresh_token.clone()
        };
        let transport = self.transport()?;
        let response = match transport.refresh(&refresh_token) {
            Err(TransportError::RefreshTokenInvalid) => {
                // 请求期间 Codex CLI 刷新了：盘上同账号的另一代是唯一的出路，采纳后只重试一次。
                let Some(live) = self
                    .live_generation(account_id)?
                    .filter(|live| live.refresh_token.trim() != refresh_token)
                else {
                    return Err(transport_error(TransportError::RefreshTokenInvalid));
                };
                if self.adopt_locked(account_id, &live, AdoptMode::RejectedManagerToken)?
                    != AdoptOutcome::Adopted
                {
                    return Err(transport_error(TransportError::RefreshTokenInvalid));
                }
                refresh_token = live.refresh_token.trim().to_string();
                transport.refresh(&refresh_token).map_err(transport_error)?
            }
            other => other.map_err(transport_error)?,
        };
        let obtained = self.now_ms();
        let (stored_refresh, stored_id, workspace) = {
            let mut inner = lock(&self.inner);
            let account = inner
                .accounts
                .get_mut(account_id)
                .ok_or_else(|| not_found(account_id))?;
            if account.refresh_token != refresh_token {
                return Err(AppError::Message(
                    "账号凭据已更新，已丢弃旧的刷新响应".into(),
                ));
            }
            let (new_workspace, _) = token_metadata(&response);
            if let (Some(new), Some(_)) = (new_workspace, account.chatgpt_account_id.as_ref()) {
                account.chatgpt_account_id = Some(new);
            }
            if let Some(new) = response
                .refresh_token
                .clone()
                .filter(|t| !t.trim().is_empty())
            {
                account.refresh_token = new;
            }
            if let Some(new) = response.id_token.clone().filter(|t| !t.trim().is_empty()) {
                account.id_token = Some(new);
            }
            account.token_updated_at_ms = obtained;
            let snapshot = (
                account.refresh_token.clone(),
                account.id_token.clone(),
                account.chatgpt_account_id.clone(),
            );
            self.save(&inner)?;
            snapshot
        };
        let workspace = workspace
            .ok_or_else(|| AppError::Message("刷新后的 token 里没有 ChatGPT workspace".into()))?;
        let cached = CachedToken {
            token: response.access_token.clone(),
            expires_at_ms: obtained + response.expires_in.unwrap_or(DEFAULT_EXPIRES_IN_SECS) * 1000,
            obtained_at_ms: obtained,
        };
        let refreshed = live::managed_auth_value(
            &workspace,
            &cached.token,
            stored_id.as_deref(),
            &stored_refresh,
            &live::format_last_refresh(obtained),
        );
        // 新一代已持久化，先保留 access 缓存；同步失败也不能回滚已消费的 refresh token。
        lock(&self.inner)
            .tokens
            .insert(account_id.to_string(), cached.clone());
        live::sync_live_after_refresh(&self.env, account_id, &refresh_token, &refreshed)
            .map_err(|err| {
                AppError::localized(
                    "codex.oauth.live_sync_failed",
                    format!("账号 {account_id} 的凭据已刷新并保存，但同步 Codex 客户端失败；新凭据已保留，没有回滚。请修复后重新切换该账号。{err}"),
                    format!("Credentials for account {account_id} were refreshed and saved, but syncing the Codex client failed. The new credentials were retained, not rolled back. Fix the issue and switch to this account again. {err}"),
                )
            })?;
        Ok(cached)
    }

    /// 写托管 `auth.json` 用的完整 token 束（必要时刷新）。在同一把代数锁里解析 token、
    /// 再核对一次盘上的代数：期间被 CLI 刷新了（采纳了新一代）或分不出新旧就停下。
    pub fn token_bundle(&self, account_id: &str) -> Result<TokenBundle, AppError> {
        let _generation = lock(&self.generation);
        let _store = self.lock_store()?;
        self.ensure_ready(account_id)?;
        let cached = self.resolve_locked(account_id)?;
        let observed = self.live_generation(account_id)?;
        if let Some(live) = &observed {
            match self.adopt_locked(account_id, live, AdoptMode::TimestampChecked)? {
                AdoptOutcome::Synchronized { .. } | AdoptOutcome::ProvablyOlder => {}
                AdoptOutcome::Ambiguous => return Err(ambiguous(account_id)),
                AdoptOutcome::Adopted => {
                    return Err(AppError::localized(
                        "codex.oauth.refreshed_while_preparing",
                        format!("Codex CLI 账号 {account_id} 的磁盘凭据在准备写入期间刷新了；为避免写出混合的 token，本次操作已取消，请重试"),
                        format!("The on-disk credentials of Codex account {account_id} were refreshed while preparing the write; to avoid a mixed token bundle nothing was changed. Please try again"),
                    ))
                }
                AdoptOutcome::NotManaged => return Err(not_found(account_id)),
            }
        }
        let inner = lock(&self.inner);
        let account = inner
            .accounts
            .get(account_id)
            .ok_or_else(|| not_found(account_id))?;
        Ok(TokenBundle {
            workspace: account
                .chatgpt_account_id
                .clone()
                .ok_or_else(|| not_found(account_id))?,
            access_token: cached.token,
            id_token: account.id_token.clone(),
            refresh_token: account.refresh_token.clone(),
            last_refresh: live::format_last_refresh(cached.obtained_at_ms),
            observed_live_refresh: observed.map(|live| live.refresh_token.trim().to_string()),
        })
    }

    /// 切走托管账号前：把盘上 CLI 轮换过的 refresh token 采纳回来，记下这一代给写锁里
    /// 核对。分不出新旧就停下（不删可能更新的登录）。
    pub fn prepare_switch_away(&self, account_id: &str) -> Result<SwitchAwayGuard, AppError> {
        let _generation = lock(&self.generation);
        let _store = self.lock_store()?;
        if !lock(&self.inner).accounts.contains_key(account_id) {
            return Ok(SwitchAwayGuard::MissingAccount);
        }
        let Some(live) = self.live_generation(account_id)? else {
            return Ok(SwitchAwayGuard::ExistingAccount(None));
        };
        match self.adopt_locked(account_id, &live, AdoptMode::TimestampChecked)? {
            AdoptOutcome::Synchronized { .. }
            | AdoptOutcome::Adopted
            | AdoptOutcome::ProvablyOlder => Ok(SwitchAwayGuard::ExistingAccount(Some(
                live.refresh_token.trim().to_string(),
            ))),
            AdoptOutcome::Ambiguous => Err(ambiguous(account_id)),
            AdoptOutcome::NotManaged => Err(not_found(account_id)),
        }
    }

    /// 从 cc-switch 的账号库（同格式 v2）读出的账号。id 不变（卡上的绑定照样能用）；
    /// 已有的 id 不覆盖。不联网。
    pub fn import_accounts(
        &self,
        accounts: Vec<AccountData>,
        default_account_id: Option<String>,
    ) -> Result<Vec<String>, AppError> {
        let _generation = lock(&self.generation);
        let _store = self.lock_store()?;
        let mut inner = lock(&self.inner);
        let mut added = Vec::new();
        for account in accounts {
            if account.refresh_token.trim().is_empty()
                || inner.accounts.contains_key(&account.account_id)
            {
                continue;
            }
            added.push(account.account_id.clone());
            inner.accounts.insert(account.account_id.clone(), account);
        }
        if inner.default_account_id.is_none() {
            inner.default_account_id = default_account_id
                .filter(|id| inner.accounts.contains_key(id))
                .or_else(|| inner.accounts.keys().next().cloned());
        }
        if !added.is_empty() {
            self.save(&inner)?;
        }
        Ok(added)
    }

    /// 读一个账号库文件（只读）。不存在时为空。
    pub fn read_store_file(
        path: &std::path::Path,
    ) -> Result<(Vec<AccountData>, Option<String>), AppError> {
        let file = Self::read_snapshot(path)?;
        Ok((
            file.accounts.into_values().collect(),
            file.default_account_id,
        ))
    }

    /// 后台保活：上次取得 token 超过 `max_age_ms` 的账号各刷新一次（换一代 refresh
    /// token，live 是这个账号的旧一代时一并同步）。需要重新登录的账号跳过。返回
    /// `(账号, 是否刷新了)`；单个账号失败不影响其他账号。
    pub fn refresh_stale(&self, max_age_ms: i64) -> Result<RefreshResults, AppError> {
        let due: Vec<String> = {
            let _generation = lock(&self.generation);
            self.reload()?;
            let inner = lock(&self.inner);
            let now = self.now_ms();
            inner
                .accounts
                .values()
                .filter(|account| !account.reauth_required())
                .filter(|account| now - account.token_updated_at_ms >= max_age_ms)
                .map(|account| account.account_id.clone())
                .collect()
        };
        Ok(due
            .into_iter()
            .map(|account_id| {
                let result = (|| {
                    let _generation = lock(&self.generation);
                    let _store = self.lock_store()?;
                    let still_due = lock(&self.inner)
                        .accounts
                        .get(&account_id)
                        .is_some_and(|a| self.now_ms() - a.token_updated_at_ms >= max_age_ms);
                    if !still_due {
                        return Ok(false);
                    }
                    lock(&self.inner).tokens.remove(&account_id);
                    self.resolve_locked(&account_id).map(|_| true)
                })();
                (account_id, result)
            })
            .collect())
    }

    /// 显式刷新一个账号，即使本进程还有有效的 access token 缓存。
    pub fn refresh_account(&self, account_id: &str) -> Result<(), AppError> {
        let _generation = lock(&self.generation);
        let _store = self.lock_store()?;
        self.ensure_ready(account_id)?;
        lock(&self.inner).tokens.remove(account_id);
        self.resolve_locked(account_id).map(|_| ())
    }

    /// 测试用：直接放一个缓存的 access token。
    #[doc(hidden)]
    pub fn test_cache_access_token(&self, account_id: &str, token: &str, expires_at_ms: i64) {
        lock(&self.inner).tokens.insert(
            account_id.to_string(),
            CachedToken {
                token: token.to_string(),
                expires_at_ms,
                obtained_at_ms: self.now_ms(),
            },
        );
    }

    /// 测试用：清掉内存里的 access token（模拟重启）。
    #[doc(hidden)]
    pub fn test_forget_access_tokens(&self) {
        lock(&self.inner).tokens.clear();
    }
}

impl AdoptOutcome {
    /// 管理器状态有没有变（给界面刷新用）。
    pub fn changed(self) -> bool {
        self.state_changed()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{FakeTransport, Identity, ManualClock};
    use super::*;

    const T: i64 = 1_790_000_000_000;

    fn manager() -> (tempfile::TempDir, CodexOAuthManager, String) {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        let fake = Arc::new(FakeTransport::new());
        let mgr = CodexOAuthManager::new(&env, Some(fake.clone()))
            .unwrap()
            .with_clock(Arc::new(ManualClock::new(T)));
        let id = mgr
            .add_account_from_tokens(&fake.issue(&Identity::new("u", "ws", "plus")), None)
            .unwrap()
            .id;
        (dir, mgr, id)
    }

    fn live(
        mgr: &CodexOAuthManager,
        id: &str,
        refresh: Option<&str>,
        at: Option<i64>,
    ) -> live::LiveRefresh {
        let account = mgr.account(id).unwrap();
        live::LiveRefresh {
            refresh_token: refresh.map(str::to_string).unwrap_or(account.refresh_token),
            id_token: account.id_token,
            last_refresh_ms: at,
            workspace: "ws".into(),
        }
    }

    /// 采纳判定的真值表（和 cc-switch 原版一致）。
    #[test]
    fn adoption_truth_table() {
        use std::cmp::Ordering::*;
        let cases: &[(Option<std::cmp::Ordering>, bool, AdoptOutcome)] = &[
            (
                Some(Greater),
                false,
                AdoptOutcome::Synchronized {
                    state_changed: true,
                },
            ),
            (
                Some(Equal),
                false,
                AdoptOutcome::Synchronized {
                    state_changed: false,
                },
            ),
            (Some(Greater), true, AdoptOutcome::Adopted),
            (Some(Less), true, AdoptOutcome::ProvablyOlder),
            (Some(Equal), true, AdoptOutcome::Ambiguous),
            (None, true, AdoptOutcome::Ambiguous),
        ];
        for (order, different, expected) in cases {
            let (_dir, mgr, id) = manager();
            let at = order.map(|o| match o {
                Greater => T + 1,
                Equal => T,
                Less => T - 1,
            });
            let refresh = different.then_some("rt-other");
            let before = mgr.account(&id).unwrap();
            let outcome = mgr
                .adopt_locked(
                    &id,
                    &live(&mgr, &id, refresh, at),
                    AdoptMode::TimestampChecked,
                )
                .unwrap();
            assert_eq!(outcome, *expected, "{order:?} different={different}");
            let after = mgr.account(&id).unwrap();
            match outcome {
                AdoptOutcome::Adopted => {
                    assert_eq!(after.refresh_token, "rt-other");
                    assert_eq!(after.token_updated_at_ms, T + 1);
                }
                AdoptOutcome::Synchronized {
                    state_changed: true,
                } => {
                    assert_eq!(after.token_updated_at_ms, T + 1);
                }
                _ => assert_eq!(after, before, "{order:?} different={different}"),
            }
        }
    }

    #[test]
    fn an_undated_manager_only_dates_matching_material() {
        let (_dir, mgr, id) = manager();
        lock(&mgr.inner)
            .accounts
            .get_mut(&id)
            .unwrap()
            .token_updated_at_ms = 0;
        let outcome = mgr
            .adopt_locked(
                &id,
                &live(&mgr, &id, Some("rt-other"), Some(T + 10)),
                AdoptMode::TimestampChecked,
            )
            .unwrap();
        assert_eq!(
            outcome,
            AdoptOutcome::Ambiguous,
            "never guess for an undated manager"
        );
        let outcome = mgr
            .adopt_locked(
                &id,
                &live(&mgr, &id, None, Some(T + 10)),
                AdoptMode::TimestampChecked,
            )
            .unwrap();
        assert_eq!(
            outcome,
            AdoptOutcome::Synchronized {
                state_changed: true
            }
        );
        assert_eq!(mgr.account(&id).unwrap().token_updated_at_ms, T + 10);
    }

    #[test]
    fn a_server_rejection_adopts_the_other_generation_even_without_a_timestamp() {
        let (_dir, mgr, id) = manager();
        mgr.test_cache_access_token(&id, "old-access", T + 10 * 3_600_000);
        let outcome = mgr
            .adopt_locked(
                &id,
                &live(&mgr, &id, Some("rt-other"), None),
                AdoptMode::RejectedManagerToken,
            )
            .unwrap();
        assert_eq!(outcome, AdoptOutcome::Adopted);
        let account = mgr.account(&id).unwrap();
        assert_eq!(account.refresh_token, "rt-other");
        assert!(account.token_updated_at_ms > T);
        assert!(
            !lock(&mgr.inner).tokens.contains_key(&id),
            "stale access token dropped"
        );
        assert_eq!(
            mgr.adopt_locked(
                "nope",
                &live(&mgr, &id, None, None),
                AdoptMode::TimestampChecked
            )
            .unwrap(),
            AdoptOutcome::NotManaged
        );
    }

    #[test]
    fn without_a_transport_nothing_is_refreshed() {
        let (dir, mgr, id) = manager();
        drop(mgr);
        let offline = CodexOAuthManager::new(&Env::with_home(dir.path()), None).unwrap();
        let err = offline.token_bundle(&id).unwrap_err();
        assert!(err.to_string().contains("网络"), "{err}");
    }

    #[test]
    fn refresh_stale_only_touches_old_generations() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        let fake = Arc::new(FakeTransport::new());
        let clock = Arc::new(ManualClock::new(T));
        let mgr = CodexOAuthManager::new(&env, Some(fake.clone()))
            .unwrap()
            .with_clock(clock.clone());
        let id = mgr
            .add_account_from_tokens(&fake.issue(&Identity::new("u", "ws", "plus")), None)
            .unwrap()
            .id;
        let day = 24 * 3600 * 1000;
        assert!(
            mgr.refresh_stale(7 * day).unwrap().is_empty(),
            "fresh account"
        );
        let before = mgr.account(&id).unwrap().refresh_token;
        clock.advance(8 * day);
        let results = mgr.refresh_stale(7 * day).unwrap();
        assert!(
            matches!(results[..], [(ref a, Ok(true))] if *a == id),
            "{results:?}"
        );
        let after = mgr.account(&id).unwrap().refresh_token;
        assert_ne!(before, after, "rotated");
        assert_eq!(fake.refresh_calls().len(), 1);
        assert!(
            mgr.refresh_stale(7 * day).unwrap().is_empty(),
            "now fresh again"
        );
        // 服务端作废了：报错，不影响下次。
        clock.advance(8 * day);
        fake.invalidate(&after);
        let results = mgr.refresh_stale(7 * day).unwrap();
        assert!(matches!(results[..], [(_, Err(_))]), "{results:?}");
    }
}
