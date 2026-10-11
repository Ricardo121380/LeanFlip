//! Network and clock interfaces; endpoint strings describe the Codex device protocol.
use serde::{Deserialize, Serialize};

/// Public protocol endpoints used by the HTTP adapter.
pub mod endpoints {
    pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
    pub const DEVICE_AUTH_USERCODE_URL: &str =
        "https://auth.openai.com/api/accounts/deviceauth/usercode";
    pub const DEVICE_AUTH_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
    pub const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
    pub const DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
    pub const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
    pub const REFRESH_SCOPE: &str = "openid profile email";
    /// 刷新返回这些错误码（或 401/403）时，实现方应返回 `TransportError::RefreshTokenInvalid`。
    pub const INVALID_REFRESH_CODES: &[&str] = &[
        "refresh_token_expired",
        "refresh_token_reused",
        "refresh_token_invalidated",
    ];
}

/// 设备码申请的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCode {
    pub device_auth_id: String,
    pub user_code: String,
    /// 有效期（秒）。
    pub expires_in: u64,
    /// 轮询间隔（秒）。
    pub interval: u64,
}

/// 轮询设备码的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll {
    /// 用户还没在浏览器里确认。
    Pending,
    Authorized {
        authorization_code: String,
        code_verifier: String,
    },
}

/// OAuth token 响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    /// access_token 有效期（秒）。
    #[serde(default)]
    pub expires_in: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// 服务端明确拒绝了这个 refresh token（过期、已被轮换、已作废）。
    #[error("refresh token rejected")]
    RefreshTokenInvalid,
    /// 设备码过期或被取消。
    #[error("device code expired")]
    Expired,
    #[error("{0}")]
    Other(String),
}

/// OpenAI OAuth 的网络调用。实现方负责超时和 User-Agent。
pub trait OAuthTransport: Send + Sync {
    fn request_device_code(&self) -> Result<DeviceCode, TransportError>;
    fn poll_device_code(
        &self,
        device_auth_id: &str,
        user_code: &str,
    ) -> Result<DevicePoll, TransportError>;
    fn exchange_code(
        &self,
        authorization_code: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse, TransportError>;
    fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, TransportError>;
}

/// 时钟（毫秒），测试里换成手动拨的。
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}
