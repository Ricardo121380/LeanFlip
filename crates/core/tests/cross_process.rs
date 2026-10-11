//! 跨进程锁：供应商库的读改写、应用写锁。另一个进程用测试二进制自己扮演。

mod common;

use std::fs::OpenOptions;
use std::sync::mpsc;
use std::time::Duration;

use ccsl_core::codex::oauth::{live as managed_live, testing::Identity};
use ccsl_core::{coexist, state, store, switch, AppError, Env, Provider, Tool};
use common::Sandbox;
use serde_json::json;

const CHILD_HOME: &str = "CCSL_TEST_CHILD_HOME";
const ROUNDS: usize = 40;

fn add_cards(env: &Env, prefix: &str) {
    for i in 0..ROUNDS {
        store::add_provider(
            env,
            Tool::Pi,
            Provider::new(
                format!("{prefix}-{i}"),
                "x",
                json!({"baseUrl": "https://x"}),
            ),
        )
        .unwrap();
    }
}

/// 两个进程同时往库里加卡：每一张都在（没有「读旧库 → 覆盖」的丢失更新）。
#[test]
fn concurrent_processes_do_not_lose_store_updates() {
    if let Some(home) = std::env::var_os(CHILD_HOME) {
        add_cards(&Env::with_home(home), "child");
        return;
    }
    let sb = Sandbox::new();
    std::fs::create_dir_all(&sb.env.data_dir).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "concurrent_processes_do_not_lose_store_updates",
            "--test-threads=1",
        ])
        .env(CHILD_HOME, sb.home())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    add_cards(&sb.env, "parent");
    assert!(child.wait().unwrap().success());
    let data = store::load(&sb.env).unwrap();
    assert_eq!(data.providers(Tool::Pi).len(), 2 * ROUNDS);
}

fn hold(path: &std::path::Path) -> std::fs::File {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .unwrap();
    file.lock().unwrap();
    file
}

/// 别的进程拿着库锁时，读改写等它放手。
#[test]
fn store_updates_wait_for_another_holder() {
    let sb = Sandbox::new();
    let lock = hold(&sb.env.data_dir.join("providers.json.lock"));
    let env = sb.env.clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        store::add_provider(&env, Tool::Pi, Provider::new("a", "a", json!({}))).unwrap();
        tx.send(()).unwrap();
    });
    assert!(
        rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "must wait"
    );
    drop(lock);
    rx.recv_timeout(Duration::from_secs(10)).unwrap();
    worker.join().unwrap();
}

/// 别的进程拿着某个应用的写锁时，这个应用的客户端文件写入等它放手。
#[test]
fn live_writes_wait_for_another_holder_of_the_app_lock() {
    let sb = Sandbox::new();
    sb.add(
        Tool::Pi,
        Provider::new("relay", "r", json!({"baseUrl": "https://r"})),
    );
    let lock = hold(&sb.env.data_dir.join("locks").join("pi.lock"));
    let env = sb.env.clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        coexist::write_to_live(&env, Tool::Pi, "relay").unwrap();
        tx.send(()).unwrap();
    });
    assert!(
        rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "must wait"
    );
    assert!(!sb.env.pi_models_path().exists());
    drop(lock);
    rx.recv_timeout(Duration::from_secs(10)).unwrap();
    worker.join().unwrap();
    assert!(sb.env.pi_models_path().exists());
}

/// 锁路径不能打开时，切换必须在改客户端、指针、pending 或备份之前停止。
#[test]
fn unavailable_app_lock_stops_switch_without_changing_files() {
    let sb = Sandbox::new();
    sb.add(
        Tool::Claude,
        Provider::new(
            "relay",
            "Relay",
            json!({"env": {"ANTHROPIC_AUTH_TOKEN": "test-new"}}),
        ),
    );
    sb.write(
        &sb.env.claude_settings_path(),
        r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"test-old"}}"#,
    );
    let before_live = sb.read(&sb.env.claude_settings_path());
    let before_store = sb.read(&sb.env.store_path());
    let lock_path = sb.env.data_dir.join("locks/claude.lock");
    std::fs::create_dir_all(&lock_path).unwrap();

    assert!(matches!(
        switch::switch(&sb.env, Tool::Claude, "relay"),
        Err(AppError::Io { .. })
    ));
    assert_eq!(sb.read(&sb.env.claude_settings_path()), before_live);
    assert_eq!(sb.read(&sb.env.store_path()), before_store);
    assert!(!sb.env.device_store().state_path().exists());
    assert!(!sb.env.device_store().first_write_backup_dir().exists());
    assert!(sb.temp_files().is_empty());

    // 失败必须释放已经取得的进程内锁；修好锁路径后同一进程可以重试。
    std::fs::remove_dir(lock_path).unwrap();
    switch::switch(&sb.env, Tool::Claude, "relay").unwrap();
    assert_eq!(sb.current(Tool::Claude).as_deref(), Some("relay"));
}

#[test]
fn unavailable_app_lock_stops_managed_live_refresh_and_deletion() {
    let sb = Sandbox::new();
    let id_token = Identity::new("alice", "team", "team").id_token();
    let old = managed_live::managed_auth_value(
        "team",
        "access-old",
        Some(&id_token),
        "refresh-old",
        "2026-10-11T00:00:00Z",
    );
    sb.write(&sb.env.codex_auth_path(), &old.to_string());
    let marker_path = managed_live::marker_path(&sb.env);
    std::fs::create_dir_all(&sb.env.data_dir).unwrap();
    std::fs::write(
        &marker_path,
        managed_live::marker_bytes(&old, "account")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let before_auth = sb.read(&sb.env.codex_auth_path());
    let before_marker = sb.read(&marker_path);
    std::fs::create_dir_all(sb.env.data_dir.join("locks/codex.lock")).unwrap();
    let new = managed_live::managed_auth_value(
        "team",
        "access-new",
        Some(&id_token),
        "refresh-new",
        "2026-10-11T00:01:00Z",
    );

    assert!(matches!(
        managed_live::sync_live_after_refresh(&sb.env, "account", "refresh-old", &new),
        Err(AppError::Io { .. })
    ));
    assert!(matches!(
        managed_live::clear_live_for_account(&sb.env, "account"),
        Err(AppError::Io { .. })
    ));
    assert_eq!(sb.read(&sb.env.codex_auth_path()), before_auth);
    assert_eq!(sb.read(&marker_path), before_marker);
}

#[test]
fn recovery_reports_app_lock_failure_and_keeps_pending() {
    let sb = Sandbox::new();
    let pending = state::Pending {
        op: "switch".into(),
        files: Vec::new(),
        target: state::PendingTarget::pointer(Some("relay".into())),
        published: false,
    };
    state::set_pending(&sb.env.device_store(), "claude", Some(pending.clone())).unwrap();
    let before = sb.read(&sb.env.device_store().state_path());
    std::fs::create_dir_all(sb.env.data_dir.join("locks/claude.lock")).unwrap();

    let outcomes = switch::recover_on_startup(&sb.env);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].0, "claude");
    assert!(matches!(outcomes[0].1, Err(AppError::Io { .. })));
    assert_eq!(sb.read(&sb.env.device_store().state_path()), before);
    assert_eq!(
        state::pending(&sb.env.device_store(), "claude").unwrap(),
        Some(pending)
    );
    assert!(!sb.env.store_path().exists());
}

/// 不同工具持有不同的应用锁，但共同状态文件的读改写仍必须跨进程串行。
#[test]
fn concurrent_processes_preserve_other_tools_pending_and_written() {
    fn append(env: &Env, tool: Tool) {
        for i in 0..ROUNDS {
            let device = env.device_store();
            let _app = device.lock_app(tool.as_str()).unwrap();
            state::update(&device, |live| {
                // 扩大读改写重叠窗口，旧的进程内锁实现会丢失另一个工具的更新。
                std::thread::sleep(Duration::from_millis(2));
                let app = live.apps.entry(tool.as_str().into()).or_default();
                app.written
                    .get_or_insert_with(Default::default)
                    .tables
                    .push(i.to_string());
                app.pending = Some(state::Pending {
                    op: format!("round-{i}"),
                    files: Vec::new(),
                    target: state::PendingTarget::default(),
                    published: false,
                });
            })
            .unwrap();
        }
    }
    if let Some(home) = std::env::var_os(CHILD_HOME) {
        append(&Env::with_home(home), Tool::Codex);
        return;
    }
    let sb = Sandbox::new();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "concurrent_processes_preserve_other_tools_pending_and_written",
            "--test-threads=1",
        ])
        .env(CHILD_HOME, sb.home())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    append(&sb.env, Tool::Claude);
    assert!(child.wait().unwrap().success());
    let live = state::load(&sb.env.device_store()).unwrap();
    for tool in [Tool::Claude, Tool::Codex] {
        let app = &live.apps[tool.as_str()];
        assert_eq!(
            app.written.as_ref().unwrap().tables,
            (0..ROUNDS).map(|i| i.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            app.pending.as_ref().unwrap().op,
            format!("round-{}", ROUNDS - 1)
        );
    }
}

#[test]
fn unavailable_state_lock_stops_switch_and_preserves_state() {
    let sb = Sandbox::new();
    sb.add(
        Tool::Claude,
        Provider::new(
            "relay",
            "Relay",
            json!({"env": {"ANTHROPIC_AUTH_TOKEN": "test-new"}}),
        ),
    );
    sb.write(&sb.env.claude_settings_path(), "{}");
    sb.write(
        &sb.env.device_store().state_path(),
        r#"{"version":1,"apps":{}}"#,
    );
    let before_store = sb.read(&sb.env.store_path());
    let before_state = sb.read(&sb.env.device_store().state_path());
    std::fs::create_dir(sb.env.data_dir.join("live-state.json.lock")).unwrap();

    assert!(matches!(
        switch::switch(&sb.env, Tool::Claude, "relay"),
        Err(AppError::Io { .. })
    ));
    assert_eq!(sb.read(&sb.env.claude_settings_path()), "{}");
    assert_eq!(sb.read(&sb.env.store_path()), before_store);
    assert_eq!(sb.read(&sb.env.device_store().state_path()), before_state);
    assert!(!sb.env.device_store().first_write_backup_dir().exists());
    assert!(sb.temp_files().is_empty());
}
