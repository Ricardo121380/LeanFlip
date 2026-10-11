//! Regression cases for LeanFlip's credential ownership and device-flow boundaries.
use ccsl_core::{
    codex::oauth::{
        testing::{FakeTransport, Identity},
        transport::{DeviceCode, DevicePoll, OAuthTransport, TokenResponse, TransportError},
        CodexOAuthManager, DeviceFlowPoll,
    },
    Env,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc, Mutex,
};
struct ResponseTransport(TokenResponse);
impl OAuthTransport for ResponseTransport {
    fn request_device_code(&self) -> Result<DeviceCode, TransportError> {
        Err(TransportError::Expired)
    }
    fn poll_device_code(&self, _: &str, _: &str) -> Result<DevicePoll, TransportError> {
        Err(TransportError::Expired)
    }
    fn exchange_code(&self, _: &str, _: &str) -> Result<TokenResponse, TransportError> {
        Err(TransportError::Expired)
    }
    fn refresh(&self, _: &str) -> Result<TokenResponse, TransportError> {
        Ok(self.0.clone())
    }
}
#[test]
fn refresh_identity_mismatch_preserves_the_account_database() {
    let directory = tempfile::tempdir().unwrap();
    let env = Env::with_home(directory.path());
    let issuer = FakeTransport::new();
    let original = issuer.issue(&Identity::new("original", "workspace", "team"));
    let wrong = issuer.issue(&Identity::new("different-person", "workspace", "team"));
    let manager = CodexOAuthManager::new(&env, Some(Arc::new(ResponseTransport(wrong)))).unwrap();
    let id = manager.add_account_from_tokens(&original, None).unwrap().id;
    let before = std::fs::read(manager.store_path()).unwrap();
    let error = manager.refresh_account(&id).unwrap_err();
    assert!(matches!(
        error,
        ccsl_core::AppError::Localized {
            key: "codex.oauth.refresh_identity_mismatch",
            ..
        }
    ));
    assert_eq!(std::fs::read(manager.store_path()).unwrap(), before);
    assert!(!env.codex_auth_path().exists());
}
struct PausedExchange {
    started: mpsc::Sender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
    calls: AtomicUsize,
    tokens: TokenResponse,
}
impl OAuthTransport for PausedExchange {
    fn request_device_code(&self) -> Result<DeviceCode, TransportError> {
        Ok(DeviceCode {
            device_auth_id: "fixture-device".into(),
            user_code: "TEST".into(),
            expires_in: 900,
            interval: 5,
        })
    }
    fn poll_device_code(&self, _: &str, _: &str) -> Result<DevicePoll, TransportError> {
        Ok(DevicePoll::Authorized {
            authorization_code: "fixture-code".into(),
            code_verifier: "fixture-verifier".into(),
        })
    }
    fn exchange_code(&self, _: &str, _: &str) -> Result<TokenResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.send(()).unwrap();
        self.resume
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        Ok(self.tokens.clone())
    }
    fn refresh(&self, _: &str) -> Result<TokenResponse, TransportError> {
        Err(TransportError::Expired)
    }
}
#[test]
fn duplicate_polls_do_not_redeem_a_code_twice_and_cancellation_wins() {
    let directory = tempfile::tempdir().unwrap();
    let env = Env::with_home(directory.path());
    let issuer = FakeTransport::new();
    let (started, started_rx) = mpsc::channel();
    let (resume, resume_rx) = mpsc::channel();
    let transport = Arc::new(PausedExchange {
        started,
        resume: Mutex::new(resume_rx),
        calls: AtomicUsize::new(0),
        tokens: issuer.issue(&Identity::new("fixture-user", "fixture-workspace", "plus")),
    });
    let manager = Arc::new(CodexOAuthManager::new(&env, Some(transport.clone())).unwrap());
    let flow = manager.start_device_flow(None).unwrap();
    let other = manager.clone();
    let code = flow.device_code.clone();
    let worker = std::thread::spawn(move || other.poll_device_flow(&code));
    started_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        manager.poll_device_flow(&flow.device_code).unwrap(),
        DeviceFlowPoll::Pending
    );
    assert!(manager.cancel_device_flow(&flow.device_code));
    resume.send(()).unwrap();
    assert!(worker.join().unwrap().is_err());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert!(manager.try_list_accounts().unwrap().is_empty());
    assert!(!manager.store_path().exists());
}

#[test]
fn switch_away_does_not_erase_a_login_that_appeared_after_preparation() {
    use ccsl_core::codex::oauth::{live, SwitchAwayGuard};
    let directory = tempfile::tempdir().unwrap();
    let env = Env::with_home(directory.path());
    let issuer = FakeTransport::new();
    let tokens = issuer.issue(&Identity::new("fixture-user", "fixture-workspace", "plus"));
    let manager = CodexOAuthManager::new(&env, None).unwrap();
    let id = manager.add_account_from_tokens(&tokens, None).unwrap().id;
    let guard = manager.prepare_switch_away(&id).unwrap();
    assert_eq!(guard, SwitchAwayGuard::ExistingAccount(None));
    let auth = live::managed_auth_value(
        "fixture-workspace",
        &tokens.access_token,
        tokens.id_token.as_deref(),
        tokens.refresh_token.as_deref().unwrap(),
        &live::format_last_refresh(1),
    );
    ccsl_core::fsutil::atomic_write_private(
        &env.codex_auth_path(),
        &serde_json::to_vec(&auth).unwrap(),
    )
    .unwrap();
    ccsl_core::fsutil::atomic_write_private(
        &live::marker_path(&env),
        &live::marker_bytes(&auth, &id).unwrap().unwrap(),
    )
    .unwrap();
    assert!(guard.ensure_unchanged(&env, &id).is_err());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(env.codex_auth_path()).unwrap())
            .unwrap(),
        auth
    );
}
