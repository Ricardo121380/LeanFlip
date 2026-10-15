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
    service.backup_load(&bytes, PASSWORD).unwrap();
    service
        .backup_apply(RestoreOptions {
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
