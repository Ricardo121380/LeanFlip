use super::*;
use crate::operation::failpoint;
use serde_json::json;
use std::fs;

struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        failpoint::crash_at(None);
        failpoint::on_before_publish(None);
    }
}

fn fixture() -> (tempfile::TempDir, Env, Provider) {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    let card = Provider::new(
        "relay",
        "Relay",
        json!({"base_url":"https://before.invalid", "models":[{"id":"m"}]}),
    );
    add_provider(&env, Tool::Hermes, card.clone(), true).unwrap();
    (dir, env, card)
}

#[test]
fn interrupted_additions_recover_the_card_with_the_client_entry() {
    let _reset = Reset;
    for point in ["staged", "pending", "marked", "published:0", "target"] {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        let card = Provider::new(
            "new",
            "New",
            json!({"base_url":"https://new.invalid", "models":[{"id":"m"}]}),
        );
        failpoint::crash_at(Some(point));
        assert!(add_provider(&env, Tool::Hermes, card.clone(), true).is_err());
        failpoint::crash_at(None);
        crate::switch::settle(&env, Tool::Hermes).unwrap();
        let committed = !matches!(point, "staged" | "pending");
        assert_eq!(
            store::load(&env)
                .unwrap()
                .provider(Tool::Hermes, &card.id)
                .is_some(),
            committed
        );
        assert_eq!(
            live_entries(&env, Tool::Hermes)
                .unwrap()
                .contains_key(&card.id),
            committed
        );
    }
}

#[test]
fn interrupted_card_updates_and_deletions_recover_with_the_client() {
    let _reset = Reset;
    for delete in [false, true] {
        for point in ["staged", "pending", "marked", "published:0", "target"] {
            let (_dir, env, old) = fixture();
            let before_live = fs::read(env.hermes_config_path()).unwrap();
            let mut next = old.clone();
            next.settings_config["base_url"] = json!("https://after.invalid");
            failpoint::crash_at(Some(point));
            if delete {
                assert!(delete_provider(&env, Tool::Hermes, &old.id).is_err());
            } else {
                assert!(update_provider(&env, Tool::Hermes, next.clone()).is_err());
            }
            failpoint::crash_at(None);
            crate::switch::settle(&env, Tool::Hermes).unwrap();
            let data = store::load(&env).unwrap();
            let live = live_entries(&env, Tool::Hermes).unwrap();
            if matches!(point, "staged" | "pending") {
                assert_eq!(data.provider(Tool::Hermes, &old.id), Some(&old));
                assert_eq!(fs::read(env.hermes_config_path()).unwrap(), before_live);
            } else if delete {
                assert!(data.provider(Tool::Hermes, &old.id).is_none());
                assert!(!live.contains_key(&old.id));
            } else {
                assert_eq!(data.provider(Tool::Hermes, &old.id), Some(&next));
                assert_eq!(live[&old.id]["base_url"], "https://after.invalid");
            }
            assert!(
                crate::state::pending(&env.device_store(), Tool::Hermes.as_str())
                    .unwrap()
                    .is_none()
            );
        }
    }
}

#[test]
fn failed_card_store_commit_keeps_a_recoverable_intent() {
    let _reset = Reset;
    for delete in [false, true] {
        let (_dir, env, old) = fixture();
        let before = fs::read(env.store_path()).unwrap();
        let lock = env.data_file("providers.json.lock");
        let blocked = lock.clone();
        failpoint::on_before_publish(Some(Box::new(move |_, _| {
            fs::remove_file(&blocked).unwrap();
            fs::create_dir(&blocked).unwrap();
        })));
        let mut next = old.clone();
        next.settings_config["base_url"] = json!("https://after.invalid");
        if delete {
            assert!(delete_provider(&env, Tool::Hermes, &old.id).is_err());
        } else {
            assert!(update_provider(&env, Tool::Hermes, next.clone()).is_err());
        }
        failpoint::on_before_publish(None);
        assert_eq!(fs::read(env.store_path()).unwrap(), before);
        assert!(
            crate::state::pending(&env.device_store(), Tool::Hermes.as_str())
                .unwrap()
                .is_some()
        );
        fs::remove_dir(lock).unwrap();
        crate::switch::settle(&env, Tool::Hermes).unwrap();
        assert_eq!(
            store::load(&env).unwrap().provider(Tool::Hermes, &old.id),
            if delete { None } else { Some(&next) }
        );
    }
}

#[test]
fn recovery_preserves_unrelated_cards_and_refuses_conflicting_edits() {
    let _reset = Reset;
    let (_dir, env, old) = fixture();
    let mut next = old.clone();
    next.name = "Updated".into();
    next.settings_config["base_url"] = json!("https://after.invalid");
    failpoint::crash_at(Some("published:0"));
    assert!(update_provider(&env, Tool::Hermes, next.clone()).is_err());
    failpoint::crash_at(None);
    let unrelated = Provider::new("other", "Other", json!({"env":{}}));
    store::add_provider(&env, Tool::Claude, unrelated.clone()).unwrap();
    let mut external = old.clone();
    external.name = "Edited elsewhere".into();
    store::replace_provider(&env, Tool::Hermes, external.clone()).unwrap();
    assert!(crate::switch::settle(&env, Tool::Hermes).is_err());
    let data = store::load(&env).unwrap();
    assert_eq!(data.provider(Tool::Hermes, &old.id), Some(&external));
    assert_eq!(data.provider(Tool::Claude, "other"), Some(&unrelated));
    assert!(
        crate::state::pending(&env.device_store(), Tool::Hermes.as_str())
            .unwrap()
            .is_some()
    );
}
