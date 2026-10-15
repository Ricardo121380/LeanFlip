//! Password-encrypted, portable LeanFlip backups. Plaintext stays in memory;
//! preview APIs return only identity/count metadata. Restores merge cards and
//! accounts and never activate providers or replace existing login credentials.
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use age::secrecy::SecretString;
use leanflip_core::codex::oauth::AccountData;
use leanflip_core::env::DirOverrides;
use leanflip_core::scene::{self, Scene};
use leanflip_core::store::StoreData;
use zeroize::{Zeroize, Zeroizing};

use super::*;

pub const MAX_BACKUP_BYTES: usize = 32 * 1024 * 1024;
const FORMAT: &str = "leanflip-backup";
const VERSION: u32 = 1;
const WORK_FACTOR: u8 = 18;
const PREVIEW_LIFETIME: Duration = Duration::from_secs(10 * 60);

pub(super) struct StagedBackup {
    payload: Payload,
    expires_at: Instant,
}
#[derive(Default)]
pub(super) struct BackupCache {
    generation: u64,
    preview: Option<StagedBackup>,
}
impl BackupCache {
    fn discard(&mut self) -> u64 {
        self.preview = None;
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }
}
fn expire_preview(staged: &mut Option<StagedBackup>, now: Instant) {
    if staged
        .as_ref()
        .is_some_and(|backup| backup.expires_at <= now)
    {
        staged.take();
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Payload {
    format: String,
    version: u32,
    created_at: u64,
    providers: StoreData,
    accounts: Vec<AccountData>,
    default_account_id: Option<String>,
    app: AppSettings,
}

fn wipe_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => text.zeroize(),
        serde_json::Value::Array(values) => values.iter_mut().for_each(wipe_value),
        serde_json::Value::Object(fields) => wipe_fields(fields),
        _ => {}
    }
}
fn wipe_fields(fields: &mut serde_json::Map<String, serde_json::Value>) {
    for (mut key, mut value) in std::mem::take(fields) {
        key.zeroize();
        wipe_value(&mut value);
    }
}
impl Drop for Payload {
    fn drop(&mut self) {
        for app in self.providers.apps.values_mut() {
            for provider in &mut app.providers {
                wipe_value(&mut provider.settings_config);
                wipe_fields(&mut provider.meta.extra);
                wipe_fields(&mut provider.extra);
            }
            wipe_fields(&mut app.extra);
        }
        wipe_fields(&mut self.providers.extra);
        wipe_fields(&mut self.providers.settings.extra);
        for account in &mut self.accounts {
            account.refresh_token.zeroize();
            account.id_token.zeroize();
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupItem {
    pub tool: Tool,
    pub id: String,
    pub name: String,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPreview {
    pub created_at: u64,
    pub items: Vec<BackupItem>,
    pub accounts: usize,
    pub existing_accounts: usize,
    pub has_settings: bool,
    pub has_paths: bool,
    pub scenes: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RestoreOptions {
    pub tools: Vec<Tool>,
    pub accounts: bool,
    pub settings: bool,
    pub paths: bool,
    pub scenes: bool,
    pub duplicate: ccswitch_export::DuplicatePolicy,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self {
            tools: Tool::ALL.to_vec(),
            accounts: true,
            settings: true,
            paths: false,
            scenes: true,
            duplicate: ccswitch_export::DuplicatePolicy::Skip,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub providers_added: usize,
    pub providers_skipped: usize,
    pub accounts_added: usize,
    pub settings_restored: bool,
    pub dirs_changed: bool,
    pub scenes_added: usize,
    pub scenes_skipped: usize,
    /// Stable stage/error codes only; never parser contents or credentials.
    pub failures: Vec<String>,
}

fn invalid() -> AppError {
    AppError::localized(
        "backup.invalid",
        "不是有效的 LeanFlip 完整备份，或备份版本尚不受支持。",
        "Not a valid LeanFlip backup, or its version is not supported.",
    )
}

fn decrypt_error() -> AppError {
    AppError::localized(
        "backup.decrypt",
        "密码不正确，或备份文件已损坏。没有恢复任何数据。",
        "Incorrect password or damaged backup. No data was restored.",
    )
}

fn password_error() -> AppError {
    AppError::localized(
        "backup.password",
        "请使用 10–4096 个字符的备份密码。密码不会保存。",
        "Use a backup passphrase of 10–4096 characters. It is not stored.",
    )
}

fn validate(payload: &Payload) -> Result<(), AppError> {
    if payload.format != FORMAT
        || payload.version != VERSION
        || payload.providers.version != store::STORE_VERSION
    {
        return Err(invalid());
    }
    for (tool, app) in &payload.providers.apps {
        if tool.parse::<Tool>().is_err() {
            return Err(invalid());
        }
        let mut ids = HashSet::new();
        if app
            .providers
            .iter()
            .any(|provider| provider.id.trim().is_empty() || !ids.insert(&provider.id))
        {
            return Err(invalid());
        }
    }
    let mut ids = HashSet::new();
    if payload.accounts.iter().any(|account| {
        account.account_id.trim().is_empty()
            || account.refresh_token.trim().is_empty()
            || !ids.insert(&account.account_id)
    }) {
        return Err(invalid());
    }
    if payload
        .default_account_id
        .as_ref()
        .is_some_and(|id| !ids.contains(id))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Merges backed-up scenes. Scenes whose id or name already exists are skipped;
/// card references follow cards that were restored under a suffixed id. The
/// last-applied scene is a device record and is never restored.
fn restore_scenes(
    data: &mut StoreData,
    scenes: &[Scene],
    renamed: &HashMap<(Tool, String), String>,
) -> (usize, usize) {
    let (mut added, mut skipped) = (0, 0);
    for scene in scenes {
        let same = |other: &Scene| {
            other.id == scene.id
                || other.name.trim().to_lowercase() == scene.name.trim().to_lowercase()
        };
        if scene::validate(scene).is_err() || data.scenes.iter().any(same) {
            skipped += 1;
            continue;
        }
        let mut next = scene.clone();
        for (key, entry) in next.tools.iter_mut() {
            let Ok(tool) = key.parse::<Tool>() else {
                continue;
            };
            let map = |id: &mut String| {
                if let Some(local) = renamed.get(&(tool, id.clone())) {
                    *id = local.clone();
                }
            };
            entry.current.iter_mut().for_each(map);
            entry.enabled.iter_mut().flatten().for_each(map);
        }
        data.scenes.push(next);
        added += 1;
    }
    (added, skipped)
}

fn encrypt(payload: &Payload, password: &str) -> Result<Vec<u8>, AppError> {
    if !(10..=4096).contains(&password.chars().count()) {
        return Err(password_error());
    }
    let bytes = Zeroizing::new(serde_json::to_vec(payload).map_err(|_| invalid())?);
    if bytes.len() > MAX_BACKUP_BYTES - 4096 {
        return Err(invalid());
    }
    let mut recipient = age::scrypt::Recipient::new(SecretString::from(password.to_owned()));
    // Fixed, bounded cost keeps backups portable across machines. Unit tests
    // use a smaller cost only in their test-only service build.
    recipient.set_work_factor(if cfg!(test) { 10 } else { WORK_FACTOR });
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
            .map_err(|_| invalid())?;
    let mut encrypted = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut encrypted)
        .map_err(|_| invalid())?;
    writer.write_all(&bytes).map_err(|_| invalid())?;
    writer.finish().map_err(|_| invalid())?;
    Ok(encrypted)
}

fn decrypt(bytes: &[u8], password: &str) -> Result<Payload, AppError> {
    if bytes.len() > MAX_BACKUP_BYTES || password.chars().count() > 4096 {
        return Err(invalid());
    }
    let mut identity = age::scrypt::Identity::new(SecretString::from(password.to_owned()));
    identity.set_max_work_factor(WORK_FACTOR);
    let decryptor = age::Decryptor::new(bytes).map_err(|_| decrypt_error())?;
    if !decryptor.is_scrypt() {
        return Err(invalid());
    }
    let reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| decrypt_error())?;
    let mut plaintext = Zeroizing::new(Vec::new());
    reader
        .take((MAX_BACKUP_BYTES + 1) as u64)
        .read_to_end(&mut plaintext)
        .map_err(|_| decrypt_error())?;
    if plaintext.len() > MAX_BACKUP_BYTES {
        return Err(invalid());
    }
    let payload: Payload = serde_json::from_slice(&plaintext).map_err(|_| invalid())?;
    validate(&payload)?;
    Ok(payload)
}

impl Service {
    /// Collect a coherent snapshot under the same cross-process guard as writes.
    /// Encryption happens after releasing it; no plaintext file is created.
    pub fn backup_export(&self, password: &str) -> ApiResult<Vec<u8>> {
        let payload = {
            let _guard = self.write_guard()?;
            let rt = self.runtime();
            let (accounts, default_account_id) =
                CodexOAuthManager::read_store_file(&rt.oauth.store_path())
                    .map_err(|e| self.err(e))?;
            Payload {
                format: FORMAT.into(),
                version: VERSION,
                created_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                providers: store::load_readonly(&rt.env).map_err(|e| self.err(e))?,
                accounts,
                default_account_id,
                app: settings::load(&rt.env.data_dir).map_err(|e| self.err(e))?,
            }
        };
        validate(&payload).map_err(|e| self.err(e))?;
        encrypt(&payload, password).map_err(|e| self.err(e))
    }

    /// Publish a private, complete encrypted file without replacing an existing
    /// file or following a destination symlink.
    pub fn backup_export_file(&self, path: &Path, password: &str) -> ApiResult<()> {
        if path.exists() || path.symlink_metadata().is_ok() {
            return Err(self.err(AppError::localized(
                "backup.exists",
                "目标文件已存在，请选择新的备份文件名。",
                "The destination already exists. Choose a new backup filename.",
            )));
        }
        let bytes = self.backup_export(password)?;
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|e| self.err(AppError::io(path, e)))?;
        temporary
            .write_all(&bytes)
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|e| self.err(AppError::io(path, e)))?;
        temporary
            .persist_noclobber(path)
            .map_err(|e| self.err(AppError::io(path, e.error)))?;
        Ok(())
    }

    pub fn backup_load(&self, bytes: &[u8], password: &str) -> ApiResult<BackupPreview> {
        let generation = self
            .staged_backup
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .discard();
        let payload = decrypt(bytes, password).map_err(|e| self.err(e))?;
        let existing = store::load_readonly(&self.env()).map_err(|e| self.err(e))?;
        let accounts = self.codex_accounts_checked()?;
        let items = Tool::ALL
            .iter()
            .flat_map(|&tool| {
                payload
                    .providers
                    .providers(tool)
                    .iter()
                    .map(move |provider| (tool, provider))
            })
            .map(|(tool, provider)| BackupItem {
                tool,
                id: provider.id.clone(),
                name: provider.name.clone(),
                duplicate: existing.provider(tool, &provider.id).is_some(),
            })
            .collect();
        let preview = BackupPreview {
            created_at: payload.created_at,
            items,
            accounts: payload.accounts.len(),
            existing_accounts: payload
                .accounts
                .iter()
                .filter(|account| {
                    accounts
                        .iter()
                        .any(|existing| existing.id == account.account_id)
                })
                .count(),
            has_settings: true,
            has_paths: payload.app.dir_overrides != DirOverrides::default(),
            scenes: payload.providers.scenes.len(),
        };
        {
            let mut cache = self.staged_backup.lock().unwrap_or_else(|e| e.into_inner());
            if cache.generation != generation {
                return Err(self.err(AppError::localized(
                    "backup.cancelled",
                    "备份预览已取消，请重新选择文件并解密。",
                    "Backup preview was cancelled. Select and unlock the file again.",
                )));
            }
            cache.preview = Some(StagedBackup {
                payload,
                expires_at: Instant::now() + PREVIEW_LIFETIME,
            });
        }
        // Do not retain Service or plaintext in the expiry worker. A newer
        // preview has its own deadline and cannot be cleared by an older worker.
        let cache = Arc::downgrade(&self.staged_backup);
        if let Err(error) = std::thread::Builder::new()
            .name("leanflip-backup-expiry".into())
            .spawn(move || {
                std::thread::sleep(PREVIEW_LIFETIME);
                if let Some(cache) = cache.upgrade() {
                    expire_preview(
                        &mut cache.lock().unwrap_or_else(|e| e.into_inner()).preview,
                        Instant::now(),
                    );
                }
            })
        {
            self.backup_discard();
            return Err(self.err(AppError::io("backup expiry worker", error)));
        }
        Ok(preview)
    }

    pub fn backup_discard(&self) {
        self.staged_backup
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .discard();
    }

    pub fn backup_apply(&self, options: RestoreOptions) -> ApiResult<RestoreReport> {
        if options.tools.is_empty() && !options.accounts && !options.settings && !options.scenes {
            return Err(self.err(AppError::InvalidInput(
                "Choose at least one restore scope".into(),
            )));
        }
        if options.paths && !options.settings {
            return Err(self.err(AppError::InvalidInput(
                "Directory overrides require restoring application settings".into(),
            )));
        }
        let payload = {
            let mut staged = self.staged_backup.lock().unwrap_or_else(|e| e.into_inner());
            expire_preview(&mut staged.preview, Instant::now());
            staged.preview.as_ref().map(|backup| backup.payload.clone())
        }
        .ok_or_else(|| {
            self.err(AppError::localized(
                "backup.none",
                "请先解密并预览备份；预览在 10 分钟后自动清除。",
                "Unlock and preview a backup first; previews are cleared after 10 minutes.",
            ))
        })?;
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        // Validate every destination before the first write. Later I/O failures
        // still return a partial report so completed merges are never hidden.
        store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        let current_settings = if options.settings {
            Some(settings::load(&rt.env.data_dir).map_err(|e| self.err(e))?)
        } else {
            None
        };
        if options.paths {
            let paths = serde_json::to_value(&payload.app.dir_overrides)
                .map_err(|_| self.err(invalid()))?;
            if paths
                .as_object()
                .into_iter()
                .flat_map(|map| map.values())
                .filter_map(serde_json::Value::as_str)
                .any(|path| !Path::new(path).is_absolute())
            {
                return Err(self.err(AppError::InvalidInput("Backup paths do not match this operating system; restore without directory overrides".into())));
            }
        }
        let mut report = RestoreReport::default();
        // Stages are idempotent merges. If a later disk write fails, report the
        // completed stages explicitly; do not roll back refreshed credentials.
        if options.accounts {
            match rt
                .oauth
                .import_accounts(payload.accounts.clone(), payload.default_account_id.clone())
            {
                Ok(added) => report.accounts_added = added.len(),
                Err(problem) => {
                    report
                        .failures
                        .push(format!("accounts:{}", self.err(problem).code()));
                    return Ok(report);
                }
            }
        }
        let result = if options.tools.is_empty() && !options.settings && !options.scenes {
            Ok((0, 0, (0, 0)))
        } else {
            store::update(&rt.env, |data| {
                let mut added = 0;
                let mut skipped = 0;
                // Suffixed cards: (tool, backup id) -> local id, for scene references.
                let mut renamed = HashMap::new();
                for &tool in &options.tools {
                    for provider in payload.providers.providers(tool) {
                        let mut next = provider.clone();
                        next.set_codex_login_ref(None);
                        if let Some(existing) = data.provider(tool, &next.id) {
                            if existing == &next
                                || options.duplicate == ccswitch_export::DuplicatePolicy::Skip
                            {
                                skipped += 1;
                                continue;
                            }
                            let mut suffix = 2;
                            loop {
                                next.id = format!("{}-{suffix}", provider.id);
                                match data.provider(tool, &next.id) {
                                    Some(existing) if existing == &next => break,
                                    Some(_) => suffix += 1,
                                    None => break,
                                }
                            }
                            renamed.insert((tool, provider.id.clone()), next.id.clone());
                            if data.provider(tool, &next.id).is_some() {
                                skipped += 1;
                                continue;
                            }
                        }
                        // Local login-stash identities are not portable; managed
                        // account bindings keep their IDs and migrate separately.
                        data.app_mut(tool).providers.push(next);
                        added += 1;
                    }
                }
                if options.settings {
                    data.settings = payload.providers.settings.clone();
                    for (key, value) in &payload.providers.extra {
                        data.extra
                            .entry(key.clone())
                            .or_insert_with(|| value.clone());
                    }
                }
                let scenes = if options.scenes {
                    restore_scenes(data, &payload.providers.scenes, &renamed)
                } else {
                    (0, 0)
                };
                Ok((added, skipped, scenes))
            })
        };
        match result {
            Ok((added, skipped, (scenes_added, scenes_skipped))) => {
                report.providers_added = added;
                report.providers_skipped = skipped;
                report.scenes_added = scenes_added;
                report.scenes_skipped = scenes_skipped;
            }
            Err(problem) => {
                report
                    .failures
                    .push(format!("providers:{}", self.err(problem).code()));
                return Ok(report);
            }
        }
        if options.settings {
            let current = current_settings.expect("settings were preflighted");
            let mut app = payload.app.clone();
            // Autostart is an OS registration, not a portable preference.
            app.autostart = current.autostart;
            if !options.paths {
                app.dir_overrides = current.dir_overrides;
            }
            settings::normalize(&mut app);
            match settings::save(&rt.env.data_dir, &app) {
                Ok(()) => {
                    match self.apply_overrides(&app.dir_overrides) {
                        Ok(changed) => report.dirs_changed = changed,
                        Err(problem) => {
                            report
                                .failures
                                .push(format!("settings:{}", self.err(problem).code()));
                            return Ok(report);
                        }
                    }
                    *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = app;
                    report.settings_restored = true;
                }
                Err(problem) => {
                    report
                        .failures
                        .push(format!("settings:{}", self.err(problem).code()));
                    return Ok(report);
                }
            }
        }
        self.backup_discard();
        Ok(report)
    }
}

#[cfg(test)]
mod tests;
