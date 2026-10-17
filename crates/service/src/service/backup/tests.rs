use super::*;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

const PASSWORD: &str = "fixture-passphrase-only";
#[test]
fn preview_expiry_removes_plaintext_and_refuses_restore_without_writes() {
    let (_source, _, bytes) = fixture();
    let target = TempDir::new().unwrap();
    let service = open(target.path());
    service.backup_load(&bytes, PASSWORD).unwrap();
    {
        let mut cache = service.staged_backup.lock().unwrap();
        expire_preview(&mut cache.preview, Instant::now());
        assert!(cache.preview.is_some(), "a current preview remains valid");
        cache.preview.as_mut().unwrap().expires_at = Instant::now() - Duration::from_secs(1);
    }
    assert_eq!(
        service
            .backup_apply(RestoreOptions::default())
            .unwrap_err()
            .code(),
        "backup.none"
    );
    assert!(service.staged_backup.lock().unwrap().preview.is_none());
    assert!(!service.env().data_dir.exists());
}

#[test]
fn sensitive_json_scrubbing_clears_nested_buffers() {
    let mut value =
        json!({"env":{"ANTHROPIC_AUTH_TOKEN":"fixture-private"},"nested":["fixture-token"]});
    wipe_value(&mut value);
    assert_eq!(value, json!({}));
}
fn open(home: &Path) -> Service {
    Service::open(Env::with_home(home), None).unwrap()
}
fn card(id: &str, secret: &str) -> Provider {
    serde_json::from_value(json!({"id":id,"name":"Work account", "settingsConfig":{"env":{"ANTHROPIC_AUTH_TOKEN":secret,"ANTHROPIC_BASE_URL":"https://example.com"}}})).unwrap()
}
fn account(token: &str) -> AccountData {
    AccountData {
        account_id: "account-fixture".into(),
        chatgpt_account_id: Some("workspace-fixture".into()),
        email: Some("test@example.com".into()),
        refresh_token: token.into(),
        authenticated_at: 1,
        id_token: Some("fixture-private-id-token".into()),
        token_updated_at_ms: 1000,
    }
}
fn fixture() -> (TempDir, Service, Vec<u8>) {
    let source = TempDir::new().unwrap();
    let service = open(source.path());
    service
        .add_provider(Tool::Claude, card("work", "fixture-private-api-key"), false)
        .unwrap();
    service
        .runtime()
        .oauth
        .import_accounts(
            vec![account("fixture-private-refresh")],
            Some("account-fixture".into()),
        )
        .unwrap();
    let mut settings = AppSettings {
        language: Some("en".into()),
        close_to_tray: false,
        autostart: true,
        ..Default::default()
    };
    settings.dir_overrides.codex_dir = Some(source.path().join("custom-codex"));
    service
        .save_settings(SettingsInput {
            app: settings,
            keep_codex_login: true,
        })
        .unwrap();
    let bytes = service.backup_export(PASSWORD).unwrap();
    (source, service, bytes)
}

#[test]
fn encrypted_roundtrip_restores_cards_accounts_and_portable_preferences() {
    let (_source, _, bytes) = fixture();
    assert!(bytes.starts_with(b"age-encryption.org/v1"));
    for secret in [
        "fixture-private-api-key",
        "fixture-private-refresh",
        "fixture-private-id-token",
        PASSWORD,
    ] {
        assert!(!bytes
            .windows(secret.len())
            .any(|window| window == secret.as_bytes()));
    }
    let target = TempDir::new().unwrap();
    let service = open(target.path());
    let preview = service.backup_load(&bytes, PASSWORD).unwrap();
    assert_eq!(preview.items.len(), 1);
    assert_eq!(preview.accounts, 1);
    assert!(preview.has_paths);
    assert!(!serde_json::to_string(&preview)
        .unwrap()
        .contains("fixture-private"));
    assert!(
        !service.env().data_dir.exists(),
        "decrypt/preview must be read-only"
    );
    let report = service.backup_apply(RestoreOptions::default()).unwrap();
    assert!(report.failures.is_empty());
    assert_eq!(report.providers_added, 1);
    assert_eq!(report.accounts_added, 1);
    assert!(report.settings_restored);
    let data = store::load_readonly(&service.env()).unwrap();
    assert_eq!(
        data.providers(Tool::Claude)[0].settings_config["env"]["ANTHROPIC_AUTH_TOKEN"],
        "fixture-private-api-key"
    );
    assert!(data.current(Tool::Claude).is_none());
    assert!(data.settings.preserve_codex_official_auth_on_switch);
    assert_eq!(
        service
            .runtime()
            .oauth
            .try_account("account-fixture")
            .unwrap()
            .unwrap()
            .refresh_token,
        "fixture-private-refresh"
    );
    assert_eq!(service.app_settings().language.as_deref(), Some("en"));
    assert!(!service.app_settings().close_to_tray);
    assert!(!service.app_settings().autostart);
    assert_eq!(
        service.app_settings().dir_overrides,
        DirOverrides::default()
    );
    assert!(!service.env().claude_settings_path().exists());
    assert!(!service.env().codex_auth_path().exists());
}

#[test]
fn wrong_password_and_tampering_clear_previous_preview_without_writing() {
    let (_source, _, mut bytes) = fixture();
    let target = TempDir::new().unwrap();
    let service = open(target.path());
    service.backup_load(&bytes, PASSWORD).unwrap();
    assert_eq!(
        service
            .backup_load(&bytes, "incorrect-password")
            .unwrap_err()
            .code(),
        "backup.decrypt"
    );
    assert_eq!(
        service
            .backup_apply(RestoreOptions::default())
            .unwrap_err()
            .code(),
        "backup.none"
    );
    *bytes.last_mut().unwrap() ^= 1;
    assert_eq!(
        service.backup_load(&bytes, PASSWORD).unwrap_err().code(),
        "backup.decrypt"
    );
    assert!(!service.env().data_dir.exists());
}

#[test]
fn scopes_and_duplicate_merges_preserve_existing_credentials() {
    let (_source, _, bytes) = fixture();
    let target = TempDir::new().unwrap();
    let service = open(target.path());
    service
        .add_provider(Tool::Claude, card("work", "local-newer-api-key"), false)
        .unwrap();
    service
        .runtime()
        .oauth
        .import_accounts(vec![account("local-newer-refresh")], None)
        .unwrap();
    for expected in [1, 0] {
        service.backup_load(&bytes, PASSWORD).unwrap();
        let report = service
            .backup_apply(RestoreOptions {
                settings: false,
                duplicate: ccswitch_export::DuplicatePolicy::Rename,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(report.providers_added, expected);
        assert_eq!(report.accounts_added, 0);
        assert!(!report.settings_restored);
    }
    let data = store::load_readonly(&service.env()).unwrap();
    assert_eq!(data.providers(Tool::Claude).len(), 2);
    assert_eq!(
        data.provider(Tool::Claude, "work").unwrap().settings_config["env"]["ANTHROPIC_AUTH_TOKEN"],
        "local-newer-api-key"
    );
    assert_eq!(
        service
            .runtime()
            .oauth
            .try_account("account-fixture")
            .unwrap()
            .unwrap()
            .refresh_token,
        "local-newer-refresh"
    );
    assert_eq!(service.app_settings().language, None);
    let preview = service.backup_load(&bytes, PASSWORD).unwrap();
    service
        .backup_apply(RestoreOptions {
            paths_confirmation: Some(preview.paths_fingerprint),
            tools: vec![],
            accounts: false,
            settings: true,
            paths: true,
            ..Default::default()
        })
        .unwrap();
    assert!(service.app_settings().dir_overrides.codex_dir.is_some());
    assert!(!service.env().codex_auth_path().exists());
}

#[test]
fn private_export_never_overwrites_files_or_follows_destination_symlinks() {
    let (dir, service, _) = fixture();
    let path = dir.path().join("backup.leanflip");
    service.backup_export_file(&path, PASSWORD).unwrap();
    let original = fs::read(&path).unwrap();
    assert_eq!(
        service
            .backup_export_file(&path, PASSWORD)
            .unwrap_err()
            .code(),
        "backup.exists"
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = dir.path().join("link.leanflip");
        symlink(&path, &link).unwrap();
        assert_eq!(
            service
                .backup_export_file(&link, PASSWORD)
                .unwrap_err()
                .code(),
            "backup.exists"
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}

#[test]
fn partial_restore_reports_completed_accounts_and_can_retry_idempotently() {
    let (_source, _, bytes) = fixture();
    let target = TempDir::new().unwrap();
    let service = open(target.path());
    service.backup_load(&bytes, PASSWORD).unwrap();
    // A directory at the provider-lock path deterministically blocks only the
    // provider stage, after the independent account file has been published.
    let blocked = service.env().data_dir.join("providers.json.lock");
    fs::create_dir_all(&blocked).unwrap();
    let report = service.backup_apply(RestoreOptions::default()).unwrap();
    assert_eq!(report.accounts_added, 1);
    assert_eq!(report.providers_added, 0);
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures[0].starts_with("providers:"));
    assert!(!service.env().claude_settings_path().exists());
    fs::remove_dir(blocked).unwrap();
    let retried = service.backup_apply(RestoreOptions::default()).unwrap();
    assert!(retried.failures.is_empty());
    assert_eq!(retried.accounts_added, 0);
    assert_eq!(retried.providers_added, 1);
}

#[test]
fn scenes_restore_once_and_follow_renamed_cards() {
    let source = TempDir::new().unwrap();
    let exporter = open(source.path());
    exporter
        .add_provider(Tool::Claude, card("work", "fixture-private-api-key"), false)
        .unwrap();
    let mut office = Scene {
        name: "Office".into(),
        ..Scene::default()
    };
    office
        .tools
        .insert("claude".into(), scene::SceneTool::current("work"));
    let office = exporter.save_scene(office).unwrap();
    scene::set_last(&exporter.env(), Some(&office.id)).unwrap();
    let bytes = exporter.backup_export(PASSWORD).unwrap();

    let target = TempDir::new().unwrap();
    let service = open(target.path());
    service
        .add_provider(Tool::Claude, card("work", "local-newer-api-key"), false)
        .unwrap();
    assert_eq!(service.backup_load(&bytes, PASSWORD).unwrap().scenes, 1);
    let report = service
        .backup_apply(RestoreOptions {
            scenes: false,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(report.scenes_added, 0);
    assert!(store::load_readonly(&service.env())
        .unwrap()
        .scenes
        .is_empty());

    for (added, skipped) in [(1, 0), (0, 1)] {
        service.backup_load(&bytes, PASSWORD).unwrap();
        let report = service
            .backup_apply(RestoreOptions {
                duplicate: ccswitch_export::DuplicatePolicy::Rename,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            (report.scenes_added, report.scenes_skipped),
            (added, skipped)
        );
    }
    let data = store::load_readonly(&service.env()).unwrap();
    assert_eq!(data.scenes.len(), 1);
    assert_eq!(data.scenes[0].id, office.id);
    assert_eq!(
        data.scenes[0].tools["claude"].current.as_deref(),
        Some("work-2"),
        "the scene follows the restored card, not the local one"
    );
    assert_eq!(
        data.last_scene, None,
        "the last-applied scene is device-local"
    );
}

#[test]
fn restored_scenes_never_point_at_a_different_local_card() {
    let source = TempDir::new().unwrap();
    let exporter = open(source.path());
    exporter
        .add_provider(Tool::Claude, card("work", "fixture-private-api-key"), false)
        .unwrap();
    let mut office = Scene {
        name: "Office".into(),
        ..Scene::default()
    };
    office
        .tools
        .insert("claude".into(), scene::SceneTool::current("work"));
    exporter.save_scene(office).unwrap();
    let bytes = exporter.backup_export(PASSWORD).unwrap();

    // Skip keeps the unrelated local "work"; the scene must not be retargeted to it.
    let target = TempDir::new().unwrap();
    let service = open(target.path());
    service
        .add_provider(Tool::Claude, card("work", "local-other-api-key"), false)
        .unwrap();
    service.backup_load(&bytes, PASSWORD).unwrap();
    let report = service.backup_apply(RestoreOptions::default()).unwrap();
    assert_eq!((report.scenes_added, report.scenes_skipped), (0, 1));
    assert!(store::load_readonly(&service.env())
        .unwrap()
        .scenes
        .is_empty());

    // Scenes restored without their cards are skipped too.
    let empty = TempDir::new().unwrap();
    let bare = open(empty.path());
    bare.backup_load(&bytes, PASSWORD).unwrap();
    let report = bare
        .backup_apply(RestoreOptions {
            tools: vec![],
            ..Default::default()
        })
        .unwrap();
    assert_eq!((report.scenes_added, report.scenes_skipped), (0, 1));

    // An identical local card is a match even when the duplicate is skipped.
    let same = TempDir::new().unwrap();
    let matching = open(same.path());
    matching
        .add_provider(Tool::Claude, card("work", "fixture-private-api-key"), false)
        .unwrap();
    matching.backup_load(&bytes, PASSWORD).unwrap();
    let report = matching.backup_apply(RestoreOptions::default()).unwrap();
    assert_eq!((report.scenes_added, report.scenes_skipped), (1, 0));
    assert_eq!(
        store::load_readonly(&matching.env()).unwrap().scenes[0].tools["claude"]
            .current
            .as_deref(),
        Some("work")
    );
}

#[test]
fn path_restore_requires_exact_current_preview_and_detects_stale_destinations() {
    let (_source, _, bytes) = fixture();
    let target = TempDir::new().unwrap();
    let service = open(target.path());
    let preview = service.backup_load(&bytes, PASSWORD).unwrap();
    let codex = preview
        .paths
        .iter()
        .find(|p| p.tool == Tool::Codex)
        .unwrap();
    assert_eq!(codex.current, service.env().live_paths(Tool::Codex));
    assert_ne!(codex.current, codex.target);
    assert!(codex.outside_home);
    for confirmation in [None, Some("incorrect".into())] {
        let error = service
            .backup_apply(RestoreOptions {
                paths: true,
                paths_confirmation: confirmation,
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(error.code(), "backup.paths_confirmation_required");
        assert!(store::load_readonly(&service.env())
            .unwrap()
            .providers(Tool::Claude)
            .is_empty());
        assert_eq!(
            service.app_settings().dir_overrides,
            DirOverrides::default()
        );
    }
    let mut app = service.app_settings();
    app.dir_overrides.codex_dir = Some(target.path().join("changed-since-preview"));
    service
        .save_settings(SettingsInput {
            app,
            keep_codex_login: false,
        })
        .unwrap();
    let error = service
        .backup_apply(RestoreOptions {
            paths: true,
            paths_confirmation: Some(preview.paths_fingerprint),
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(error.code(), "backup.paths_confirmation_required");
    let fresh = service.backup_load(&bytes, PASSWORD).unwrap();
    service
        .backup_apply(RestoreOptions {
            paths: true,
            paths_confirmation: Some(fresh.paths_fingerprint),
            ..Default::default()
        })
        .unwrap();
}

#[cfg(unix)]
#[test]
fn directory_consent_changes_when_a_symlink_points_somewhere_else() {
    let dir = TempDir::new().unwrap();
    let env = Env::with_home(dir.path());
    for name in ["one", "two"] {
        fs::create_dir(dir.path().join(name)).unwrap();
    }
    let alias = dir.path().join("alias");
    std::os::unix::fs::symlink(dir.path().join("one"), &alias).unwrap();
    let overrides = DirOverrides {
        codex_dir: Some(alias.clone()),
        ..Default::default()
    };
    let before = directory_preview(&env, &env, &overrides);
    fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(dir.path().join("two"), &alias).unwrap();
    let after = directory_preview(&env, &env, &overrides);
    assert_eq!(before[1].target, after[1].target);
    assert_ne!(
        directory_fingerprint(&before).unwrap(),
        directory_fingerprint(&after).unwrap()
    );
}
