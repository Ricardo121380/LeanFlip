//! 测试用的假 OAuth 服务端和手动时钟（单元测试和集成测试共用，不属于公开 API）。
//!
//! 假服务端按真实服务端的规则发 token：每次刷新都轮换 refresh token，旧的立即作废
//! （再用就是 `refresh_token_reused`）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use serde_json::json;

use super::transport::{
    Clock, DeviceCode, DevicePoll, OAuthTransport, TokenResponse, TransportError,
};
use crate::codex::auth::test_id_token_with_claims;

/// 一个 ChatGPT 登录身份（用户 + workspace）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub user: String,
    pub workspace: String,
    pub email: String,
    pub plan: String,
}

impl Identity {
    pub fn new(user: &str, workspace: &str, plan: &str) -> Self {
        Self {
            user: user.into(),
            workspace: workspace.into(),
            email: format!("{user}@example.com"),
            plan: plan.into(),
        }
    }

    pub fn id_token(&self) -> String {
        test_id_token_with_claims(&json!({
            "sub": self.user,
            "email": self.email,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": self.workspace,
                "chatgpt_plan_type": self.plan,
            },
        }))
    }
}

type Hook = Box<dyn FnMut(&str) + Send>;

#[derive(Default)]
struct State {
    next: u64,
    valid: HashMap<String, Identity>,
    devices: HashMap<String, Option<Identity>>,
    refresh_calls: Vec<String>,
    on_refresh: Option<Hook>,
}

#[derive(Default)]
pub struct FakeTransport {
    state: Mutex<State>,
}

impl FakeTransport {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 给这个身份发一组新 token（登录）。
    pub fn issue(&self, identity: &Identity) -> TokenResponse {
        let mut state = self.lock();
        state.next += 1;
        let n = state.next;
        let refresh = format!("rt-{}-{}-{n}", identity.user, identity.workspace);
        state.valid.insert(refresh.clone(), identity.clone());
        TokenResponse {
            access_token: format!("at-{}-{}-{n}", identity.user, identity.workspace),
            refresh_token: Some(refresh),
            id_token: Some(identity.id_token()),
            expires_in: Some(3600),
        }
    }

    /// 让设备码登录完成为这个身份。
    pub fn approve(&self, device_code: &str, identity: &Identity) {
        self.lock()
            .devices
            .insert(device_code.to_string(), Some(identity.clone()));
    }

    pub fn is_valid(&self, refresh: &str) -> bool {
        self.lock().valid.contains_key(refresh)
    }

    pub fn invalidate(&self, refresh: &str) {
        self.lock().valid.remove(refresh);
    }

    /// 管理器发起过的刷新（用的 refresh token）。
    pub fn refresh_calls(&self) -> Vec<String> {
        self.lock().refresh_calls.clone()
    }

    /// 每次管理器刷新前调用（模拟 Codex CLI 恰好在这时刷新）。
    pub fn set_on_refresh(&self, hook: Option<Hook>) {
        self.lock().on_refresh = hook;
    }

    /// Codex CLI 自己刷新：和管理器刷新走同一个服务端规则，不记进 `refresh_calls`。
    pub fn cli_refresh(&self, refresh: &str) -> Result<TokenResponse, TransportError> {
        self.rotate(refresh)
    }

    fn rotate(&self, refresh: &str) -> Result<TokenResponse, TransportError> {
        let identity = self
            .lock()
            .valid
            .remove(refresh)
            .ok_or(TransportError::RefreshTokenInvalid)?;
        Ok(self.issue(&identity))
    }
}

impl OAuthTransport for FakeTransport {
    fn request_device_code(&self) -> Result<DeviceCode, TransportError> {
        let mut state = self.lock();
        state.next += 1;
        let code = format!("device-{}", state.next);
        state.devices.insert(code.clone(), None);
        Ok(DeviceCode {
            device_auth_id: code,
            user_code: "ABCD-EFGH".into(),
            expires_in: 900,
            interval: 5,
        })
    }

    fn poll_device_code(
        &self,
        device_auth_id: &str,
        _user_code: &str,
    ) -> Result<DevicePoll, TransportError> {
        match self.lock().devices.get(device_auth_id) {
            None => Err(TransportError::Expired),
            Some(None) => Ok(DevicePoll::Pending),
            Some(Some(_)) => Ok(DevicePoll::Authorized {
                authorization_code: format!("code:{device_auth_id}"),
                code_verifier: "verifier".into(),
            }),
        }
    }

    fn exchange_code(
        &self,
        authorization_code: &str,
        _code_verifier: &str,
    ) -> Result<TokenResponse, TransportError> {
        let device = authorization_code.trim_start_matches("code:");
        let identity = self
            .lock()
            .devices
            .remove(device)
            .flatten()
            .ok_or(TransportError::Expired)?;
        Ok(self.issue(&identity))
    }

    fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, TransportError> {
        let hook = self.lock().on_refresh.take();
        if let Some(mut hook) = hook {
            hook(refresh_token);
            let mut state = self.lock();
            if state.on_refresh.is_none() {
                state.on_refresh = Some(hook);
            }
        }
        self.lock().refresh_calls.push(refresh_token.to_string());
        self.rotate(refresh_token)
    }
}

/// 手动拨的时钟。
pub struct ManualClock(AtomicI64);

impl ManualClock {
    pub fn new(start_ms: i64) -> Self {
        Self(AtomicI64::new(start_ms))
    }

    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
