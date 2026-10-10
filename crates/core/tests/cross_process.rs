//! 跨进程锁：供应商库的读改写、应用写锁。另一个进程用测试二进制自己扮演。

mod common;

use std::fs::OpenOptions;
use std::sync::mpsc;
use std::time::Duration;

use ccsl_core::{coexist, store, Env, Provider, Tool};
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
