use super::*;
use crate::{codex::oauth::AccountData, operation::failpoint};
use serde_json::json;
use std::fs;

struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        failpoint::crash_at(None);
        failpoint::on_before_publish(None);
    }
}
fn fixture() -> (tempfile::TempDir, Env, PathBuf, CodexOAuthManager) {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    let source_env = Env::with_home(dir.path().join("source"));
    let source = CodexOAuthManager::new(&source_env, None).unwrap();
    source
        .import_accounts(
            vec![AccountData {
                account_id: "fixture".into(),
                chatgpt_account_id: Some("workspace".into()),
                email: None,
                refresh_token: "fixture-refresh".into(),
                authenticated_at: 1,
                id_token: None,
                token_updated_at_ms: 1,
            }],
            Some("fixture".into()),
        )
        .unwrap();
    fs::write(source_env.data_dir.join("config.json"), json!({"claude":{"providers":{"imported":{"name":"Imported","settingsConfig":{"env":{}}}}}}).to_string()).unwrap();
    let manager = CodexOAuthManager::new(&env, None).unwrap();
    (dir, env, source_env.data_dir, manager)
}

#[test]
fn interrupted_import_is_discarded_before_publication_and_completed_afterward() {
    let _reset = Reset;
    for point in ["staged", "pending", "marked", "published:0", "target"] {
        let (_dir, env, source, manager) = fixture();
        let source_before = fs::read(source.join(ACCOUNTS_FILENAME)).unwrap();
        failpoint::crash_at(Some(point));
        assert!(apply(&env, &source, &[Tool::Claude, Tool::Codex], Some(&manager)).is_err());
        failpoint::crash_at(None);
        crate::switch::settle(&env, Tool::Codex).unwrap();
        let committed = !matches!(point, "staged" | "pending");
        assert_eq!(manager.try_account("fixture").unwrap().is_some(), committed);
        assert_eq!(
            store::load(&env)
                .unwrap()
                .provider(Tool::Claude, "imported")
                .is_some(),
            committed
        );
        // A retry imports at most one copy and never overwrites the source account store.
        apply(&env, &source, &[Tool::Claude, Tool::Codex], Some(&manager)).unwrap();
        assert_eq!(manager.try_list_accounts().unwrap().len(), 1);
        assert_eq!(
            store::load(&env)
                .unwrap()
                .app(Tool::Claude)
                .unwrap()
                .providers
                .len(),
            1
        );
        assert_eq!(
            fs::read(source.join(ACCOUNTS_FILENAME)).unwrap(),
            source_before
        );
    }
}

#[test]
fn card_store_failure_after_account_publish_is_recovered_before_retry() {
    let _reset = Reset;
    let (_dir, env, source, manager) = fixture();
    let lock = env.data_file("providers.json.lock");
    let blocked = lock.clone();
    failpoint::on_before_publish(Some(Box::new(move |_, _| {
        fs::remove_file(&blocked).unwrap();
        fs::create_dir(&blocked).unwrap();
    })));
    assert!(apply(&env, &source, &[Tool::Claude, Tool::Codex], Some(&manager)).is_err());
    failpoint::on_before_publish(None);
    assert!(manager.try_account("fixture").unwrap().is_some());
    assert!(
        crate::state::pending(&env.device_store(), Tool::Codex.as_str())
            .unwrap()
            .is_some()
    );
    fs::remove_dir(lock).unwrap();
    let report = apply(&env, &source, &[Tool::Claude, Tool::Codex], Some(&manager)).unwrap();
    assert!(report.created.is_empty());
    assert!(report.accounts_added.is_empty());
    assert!(store::load(&env)
        .unwrap()
        .provider(Tool::Claude, "imported")
        .is_some());
    assert_eq!(manager.try_list_accounts().unwrap().len(), 1);
}
