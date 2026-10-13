//! App and CLI use independent Service instances in independent processes.
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use leanflip_core::{store, Env, Provider, Tool};
use leanflip_service::Service;
use serde_json::json;

const CHILD_HOME: &str = "LEANFLIP_SERVICE_TEST_HOME";

#[test]
fn independent_processes_keep_live_config_and_pointer_consistent() {
    if let Some(home) = std::env::var_os(CHILD_HOME) {
        let service = Service::open(Env::with_home(home), None).unwrap();
        for _ in 0..20 {
            service.switch_provider(Tool::Claude, "child").unwrap();
        }
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let env = Env::with_home(temp.path());
    for id in ["parent", "child"] {
        store::add_provider(&env, Tool::Claude, Provider::new(id, id, json!({
            "env": {"ANTHROPIC_BASE_URL": format!("https://{id}.example"), "ANTHROPIC_AUTH_TOKEN": "test-only"}
        }))).unwrap();
    }
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "independent_processes_keep_live_config_and_pointer_consistent",
            "--test-threads=1",
        ])
        .env(CHILD_HOME, temp.path())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let service = Service::open(env.clone(), None).unwrap();
    for _ in 0..20 {
        service.switch_provider(Tool::Claude, "parent").unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("concurrent switch process did not finish");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let data = store::load_readonly(&env).unwrap();
    let current = data.current(Tool::Claude).unwrap();
    let live: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env.claude_settings_path()).unwrap()).unwrap();
    assert_eq!(
        live["env"]["ANTHROPIC_BASE_URL"],
        format!("https://{}.example", current.id)
    );
    assert!(leanflip_core::state::pending(&env.device_store(), "claude")
        .unwrap()
        .is_none());
}
