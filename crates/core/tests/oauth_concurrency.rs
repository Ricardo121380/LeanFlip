//! OAuth accounts shared by an App and a CLI, with no network or real user files.
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use ccsl_core::codex::oauth::testing::{FakeTransport, Identity, ManualClock};
use ccsl_core::codex::oauth::{AccountData, CodexOAuthManager};
use ccsl_core::{store, Env, Provider, Tool};

const NOW: i64 = 1_790_000_000_000;

fn account(id: &str) -> AccountData {
    AccountData {
        account_id: id.into(),
        chatgpt_account_id: Some(format!("workspace-{id}")),
        email: Some(format!("{id}@example.com")),
        refresh_token: format!("test-refresh-{id}"),
        authenticated_at: NOW / 1000,
        id_token: Some(Identity::new(id, &format!("workspace-{id}"), "plus").id_token()),
        token_updated_at_ms: NOW,
    }
}

fn import(manager: &CodexOAuthManager, id: &str) {
    manager.import_accounts(vec![account(id)], None).unwrap();
}

#[test]
fn stale_managers_preserve_additions_defaults_and_deletions() {
    let home = tempfile::tempdir().unwrap();
    let env = Env::with_home(home.path());
    let app = CodexOAuthManager::new(&env, None).unwrap();
    let cli = CodexOAuthManager::new(&env, None).unwrap();
    import(&app, "a");
    import(&cli, "b");
    app.set_default_account("b").unwrap();
    import(&cli, "c");
    assert_eq!(cli.try_default_account_id().unwrap().as_deref(), Some("b"));
    assert_eq!(app.try_list_accounts().unwrap().len(), 3);

    app.remove_account("a").unwrap();
    import(&cli, "d");
    assert!(cli.try_account("a").unwrap().is_none());
    assert_eq!(app.try_list_accounts().unwrap().len(), 3);
    assert!(cli.set_default_account("a").is_err());
    app.clear_all().unwrap();
    assert!(cli.try_list_accounts().unwrap().is_empty());
}

#[test]
fn queries_do_not_create_files_and_report_corrupt_snapshots() {
    let home = tempfile::tempdir().unwrap();
    let env = Env::with_home(home.path());
    let manager = CodexOAuthManager::new(&env, None).unwrap();
    assert!(manager.try_list_accounts().unwrap().is_empty());
    assert!(manager.try_account("absent").unwrap().is_none());
    assert!(manager.try_default_account_id().unwrap().is_none());
    assert!(manager.refresh_stale(1000).unwrap().is_empty());
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
    import(&manager, "a");
    std::fs::write(manager.store_path(), "broken-json").unwrap();
    assert!(manager.try_list_accounts().is_err());
    assert!(manager.try_account("a").is_err());
    assert!(manager.refresh_stale(0).is_err());
    assert!(manager.set_default_account("a").is_err());
    assert_eq!(std::fs::read(manager.store_path()).unwrap(), b"broken-json");
}

#[test]
fn failure_to_lock_accounts_refuses_the_write() {
    let home = tempfile::tempdir().unwrap();
    let env = Env::with_home(home.path());
    let manager = CodexOAuthManager::new(&env, None).unwrap();
    std::fs::create_dir_all(env.data_file("codex_oauth_auth.json.lock")).unwrap();
    assert!(manager.import_accounts(vec![account("a")], None).is_err());
    assert!(!manager.store_path().exists());
}

#[test]
fn independent_managers_refresh_one_chain_and_drop_stale_access_caches() {
    let home = tempfile::tempdir().unwrap();
    let env = Env::with_home(home.path());
    let transport = Arc::new(FakeTransport::new());
    let clock = Arc::new(ManualClock::new(NOW));
    let app = CodexOAuthManager::new(&env, Some(transport.clone()))
        .unwrap()
        .with_clock(clock.clone());
    let id = app
        .add_account_from_tokens(
            &transport.issue(&Identity::new("user", "workspace", "plus")),
            None,
        )
        .unwrap()
        .id;
    let cli = CodexOAuthManager::new(&env, Some(transport.clone()))
        .unwrap()
        .with_clock(clock);
    let initial = app.token_bundle(&id).unwrap();
    cli.refresh_account(&id).unwrap();
    let latest = app.token_bundle(&id).unwrap();
    assert_ne!(initial.access_token, latest.access_token);
    assert!(transport.is_valid(&latest.refresh_token));
    assert_eq!(transport.refresh_calls().len(), 2);

    app.test_forget_access_tokens();
    cli.test_forget_access_tokens();
    let barrier = Barrier::new(2);
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            barrier.wait();
            app.token_bundle(&id).unwrap();
        });
        barrier.wait();
        cli.token_bundle(&id).unwrap();
        worker.join().unwrap();
    });
    let calls = transport.refresh_calls();
    let distinct: std::collections::HashSet<_> = calls.iter().collect();
    assert_eq!(
        calls.len(),
        distinct.len(),
        "each refresh generation consumed once"
    );
    assert!(transport.is_valid(&app.try_account(&id).unwrap().unwrap().refresh_token));
}

#[test]
fn deleting_an_account_invalidates_another_managers_cache() {
    let home = tempfile::tempdir().unwrap();
    let env = Env::with_home(home.path());
    let transport = Arc::new(FakeTransport::new());
    let app = CodexOAuthManager::new(&env, Some(transport.clone())).unwrap();
    let id = app
        .add_account_from_tokens(
            &transport.issue(&Identity::new("user", "workspace", "plus")),
            None,
        )
        .unwrap()
        .id;
    let cli = CodexOAuthManager::new(&env, Some(transport.clone())).unwrap();
    cli.remove_account(&id).unwrap();
    assert!(app.token_bundle(&id).is_err());
    assert!(app.refresh_account(&id).is_err());
    assert!(transport.refresh_calls().is_empty());
    assert!(app.try_list_accounts().unwrap().is_empty());
}

fn wait_for(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(Instant::now() < deadline, "child synchronization timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn separate_processes_preserve_concurrent_imports_and_removed_accounts() {
    const CHILD_HOME: &str = "CCSL_OAUTH_TEST_CHILD_HOME";
    const ROUNDS: usize = 20;
    if let Some(home) = std::env::var_os(CHILD_HOME) {
        let env = Env::with_home(home);
        let manager = CodexOAuthManager::new(&env, None).unwrap();
        std::fs::write(env.home.join("ready"), []).unwrap();
        wait_for(&env.home.join("continue"));
        for n in 0..ROUNDS {
            import(&manager, &format!("child-{n}"));
        }
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let env = Env::with_home(home.path());
    let manager = CodexOAuthManager::new(&env, None).unwrap();
    import(&manager, "remove-me");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "separate_processes_preserve_concurrent_imports_and_removed_accounts",
            "--test-threads=1",
        ])
        .env(CHILD_HOME, home.path())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for(&env.home.join("ready"));
    manager.remove_account("remove-me").unwrap();
    std::fs::write(env.home.join("continue"), []).unwrap();
    for n in 0..ROUNDS {
        import(&manager, &format!("parent-{n}"));
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(manager.try_list_accounts().unwrap().len(), 2 * ROUNDS);
    assert!(manager.try_account("remove-me").unwrap().is_none());
}

#[test]
fn prepared_switch_does_not_restore_an_account_removed_by_another_manager() {
    use ccsl_core::provider::AuthBinding;
    use ccsl_core::switch::codex;
    let home = tempfile::tempdir().unwrap();
    let env = Env::with_home(home.path());
    let transport = Arc::new(FakeTransport::new());
    let manager = CodexOAuthManager::new(&env, Some(transport.clone())).unwrap();
    let id = manager
        .add_account_from_tokens(
            &transport.issue(&Identity::new("user", "workspace", "plus")),
            None,
        )
        .unwrap()
        .id;
    let mut provider = Provider::new(
        "managed",
        "managed",
        serde_json::json!({"auth": {}, "config": "model = \"gpt-5\"\n"}),
    )
    .official();
    provider.meta.auth_binding = Some(AuthBinding {
        auth_provider: "codex_oauth".into(),
        account_id: Some(id.clone()),
    });
    store::add_provider(&env, Tool::Codex, provider.clone()).unwrap();
    let data = store::load(&env).unwrap();
    let prepared = codex::prepare(Some(&manager), None, &provider).unwrap();
    let planned = codex::plan(&data, None, &provider).unwrap();
    CodexOAuthManager::new(&env, None)
        .unwrap()
        .remove_account(&id)
        .unwrap();
    let error = codex::run(
        &env,
        &data,
        "switch",
        planned,
        &prepared,
        ccsl_core::state::PendingTarget::pointer(Some("managed".into())),
    )
    .unwrap_err();
    assert!(error.to_string().contains("已更新或删除"));
    assert!(!env.codex_auth_path().exists());
    assert!(!env.codex_config_path().exists());
}

/// A file-backed fake OAuth server, shared by two actual processes. It rejects
/// reuse of a refresh generation exactly like FakeTransport, without networking.
struct ProcessTransport(std::path::PathBuf);

impl ccsl_core::codex::oauth::transport::OAuthTransport for ProcessTransport {
    fn request_device_code(
        &self,
    ) -> Result<
        ccsl_core::codex::oauth::transport::DeviceCode,
        ccsl_core::codex::oauth::transport::TransportError,
    > {
        unreachable!("refresh-only fake server")
    }

    fn poll_device_code(
        &self,
        _: &str,
        _: &str,
    ) -> Result<
        ccsl_core::codex::oauth::transport::DevicePoll,
        ccsl_core::codex::oauth::transport::TransportError,
    > {
        unreachable!("refresh-only fake server")
    }

    fn exchange_code(
        &self,
        _: &str,
        _: &str,
    ) -> Result<
        ccsl_core::codex::oauth::transport::TokenResponse,
        ccsl_core::codex::oauth::transport::TransportError,
    > {
        unreachable!("refresh-only fake server")
    }

    fn refresh(
        &self,
        refresh: &str,
    ) -> Result<
        ccsl_core::codex::oauth::transport::TokenResponse,
        ccsl_core::codex::oauth::transport::TransportError,
    > {
        use ccsl_core::codex::oauth::transport::{TokenResponse, TransportError};
        let server_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.0.join("server.lock"))
            .unwrap();
        server_lock.lock().unwrap();
        let state_path = self.0.join("server-generation");
        let generation: usize = std::fs::read_to_string(&state_path)
            .unwrap()
            .parse()
            .unwrap();
        if refresh != format!("test-refresh-{generation}") {
            return Err(TransportError::RefreshTokenInvalid);
        }
        std::thread::sleep(Duration::from_millis(50));
        let next = generation + 1;
        std::fs::write(state_path, next.to_string()).unwrap();
        Ok(TokenResponse {
            access_token: format!("test-access-{next}"),
            refresh_token: Some(format!("test-refresh-{next}")),
            id_token: Some(Identity::new("a", "workspace-a", "plus").id_token()),
            expires_in: Some(3600),
        })
    }
}

#[test]
fn separate_processes_refresh_a_single_token_chain() {
    const CHILD_HOME: &str = "CCSL_OAUTH_REFRESH_CHILD_HOME";
    if let Some(home) = std::env::var_os(CHILD_HOME) {
        let env = Env::with_home(home);
        let manager =
            CodexOAuthManager::new(&env, Some(Arc::new(ProcessTransport(env.home.clone()))))
                .unwrap();
        std::fs::write(env.home.join("ready"), []).unwrap();
        wait_for(&env.home.join("continue"));
        manager.refresh_account("a").unwrap();
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let env = Env::with_home(home.path());
    let manager =
        CodexOAuthManager::new(&env, Some(Arc::new(ProcessTransport(env.home.clone())))).unwrap();
    let mut initial = account("a");
    initial.refresh_token = "test-refresh-0".into();
    manager.import_accounts(vec![initial], None).unwrap();
    std::fs::write(env.home.join("server-generation"), "0").unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "separate_processes_refresh_a_single_token_chain",
            "--test-threads=1",
        ])
        .env(CHILD_HOME, home.path())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for(&env.home.join("ready"));
    std::fs::write(env.home.join("continue"), []).unwrap();
    manager.refresh_account("a").unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(
        std::fs::read_to_string(env.home.join("server-generation")).unwrap(),
        "2"
    );
    assert_eq!(
        manager.try_account("a").unwrap().unwrap().refresh_token,
        "test-refresh-2"
    );
}
