use super::*;
use crate::operation::failpoint;
use std::fs;

struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        failpoint::crash_at(None);
        failpoint::on_before_publish(None);
    }
}
fn fixture() -> (tempfile::TempDir, Env, CodexOAuthManager) {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    let manager = CodexOAuthManager::new(&env, None).unwrap();
    let identity = testing::Identity::new("fixture", "workspace", "plus");
    manager
        .import_accounts(
            vec![AccountData {
                account_id: "fixture".into(),
                chatgpt_account_id: Some("workspace".into()),
                email: None,
                refresh_token: "fixture-refresh".into(),
                authenticated_at: 1,
                id_token: Some(identity.id_token()),
                token_updated_at_ms: 1,
            }],
            Some("fixture".into()),
        )
        .unwrap();
    let auth = live::managed_auth_value(
        "workspace",
        "fixture-access",
        Some(&identity.id_token()),
        "fixture-refresh",
        "2026-01-01T00:00:00Z",
    );
    fsutil::atomic_write_private(&env.codex_auth_path(), &serde_json::to_vec(&auth).unwrap())
        .unwrap();
    fsutil::atomic_write_private(
        &live::marker_path(&env),
        &live::marker_bytes(&auth, "fixture").unwrap().unwrap(),
    )
    .unwrap();
    (dir, env, manager)
}

#[test]
fn removal_before_publication_keeps_account_and_live_credentials() {
    let _reset = Reset;
    for point in ["staged", "pending"] {
        let (_dir, env, manager) = fixture();
        let auth = fs::read(env.codex_auth_path()).unwrap();
        failpoint::crash_at(Some(point));
        assert!(manager.remove_account("fixture").is_err());
        failpoint::crash_at(None);
        crate::switch::settle(&env, Tool::Codex).unwrap();
        assert!(manager.try_account("fixture").unwrap().is_some());
        assert_eq!(fs::read(env.codex_auth_path()).unwrap(), auth);
        assert!(live::marker_path(&env).exists());
    }
}

#[test]
fn interrupted_removal_rolls_forward_from_each_publication_boundary() {
    let _reset = Reset;
    for all in [false, true] {
        for point in [
            "marked",
            "published:0",
            "published:1",
            "published:2",
            "target",
        ] {
            let (_dir, env, manager) = fixture();
            failpoint::crash_at(Some(point));
            assert!(
                if all {
                    manager.clear_all()
                } else {
                    manager.remove_account("fixture")
                }
                .is_err(),
                "{point}"
            );
            failpoint::crash_at(None);
            crate::switch::settle(&env, Tool::Codex).unwrap();
            assert!(manager.try_account("fixture").unwrap().is_none(), "{point}");
            assert!(!env.codex_auth_path().exists(), "{point}");
            assert!(!live::marker_path(&env).exists(), "{point}");
            assert!(crate::state::pending(&env.device_store(), "codex")
                .unwrap()
                .is_none());
        }
    }
}

#[test]
fn filesystem_failure_before_account_publish_does_not_clear_live_auth() {
    let (_dir, env, manager) = fixture();
    let auth = fs::read(env.codex_auth_path()).unwrap();
    let store = fs::read(manager.store_path()).unwrap();
    let fingerprint = digest(Some(manager.store_path().to_string_lossy().as_bytes())).unwrap();
    let blocked = env.data_dir.join("backups/live-first-write").join(format!(
        "{}-{}",
        &fingerprint[..12],
        STORE_FILENAME
    ));
    fs::create_dir_all(&blocked).unwrap(); // A real rename-to-directory failure while preparing the store backup.
    assert!(manager.remove_account("fixture").is_err());
    assert_eq!(fs::read(manager.store_path()).unwrap(), store);
    assert_eq!(fs::read(env.codex_auth_path()).unwrap(), auth);
    assert!(live::marker_path(&env).exists());
    fs::remove_dir(blocked).unwrap();
    manager.remove_account("fixture").unwrap();
    assert!(manager.try_account("fixture").unwrap().is_none());
    assert!(!env.codex_auth_path().exists());
}

#[test]
fn recovery_preserves_external_credentials_and_mutation_cannot_resurrect_removed_account() {
    let _reset = Reset;
    let (_dir, env, manager) = fixture();
    failpoint::crash_at(Some("published:0"));
    assert!(manager.remove_account("fixture").is_err());
    failpoint::crash_at(None);
    let external = br#"{"OPENAI_API_KEY":"external-fixture"}"#;
    fs::write(env.codex_auth_path(), external).unwrap();
    // An unrelated account mutation must settle the pending deletion before reading the store.
    manager
        .import_accounts(
            vec![AccountData {
                account_id: "new".into(),
                chatgpt_account_id: None,
                email: None,
                refresh_token: "new-fixture".into(),
                authenticated_at: 2,
                id_token: None,
                token_updated_at_ms: 2,
            }],
            None,
        )
        .unwrap();
    assert!(manager.try_account("fixture").unwrap().is_none());
    assert!(manager.try_account("new").unwrap().is_some());
    assert_eq!(fs::read(env.codex_auth_path()).unwrap(), external);
    assert!(!live::marker_path(&env).exists());
}
