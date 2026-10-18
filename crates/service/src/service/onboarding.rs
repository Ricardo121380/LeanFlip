use super::*;
use leanflip_core::provider_import::{self, Preview, Selection, Source};
use std::sync::atomic::Ordering;

const MARKER: &str = "onboarding-v1.json";

pub(super) fn needed(env: &Env) -> Result<bool, AppError> {
    // Existing installations (including CLI users) keep their normal startup.
    for path in [
        env.data_dir.join(MARKER),
        settings::path(&env.data_dir),
        env.store_path(),
    ] {
        if path.try_exists().map_err(|e| AppError::io(&path, e))? {
            return Ok(false);
        }
    }
    Ok(true)
}

impl Service {
    pub fn complete_onboarding(&self) -> ApiResult<()> {
        let _guard = self.write_guard()?;
        leanflip_core::fsutil::atomic_write_private(
            &self.env().data_dir.join(MARKER),
            b"{\"completed\":true,\"version\":1}\n",
        )
        .map_err(|e| self.err(e))?;
        self.onboarding_needed.store(false, Ordering::Relaxed);
        self.provider_import_discard();
        Ok(())
    }

    pub fn provider_import_scan(
        &self,
        source: Source,
        directory: Option<PathBuf>,
    ) -> ApiResult<Preview> {
        let mut staged = self
            .staged_provider_import
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        staged.0 += 1;
        staged.1 = None; // A failed scan must never leave the previous source importable.
        let scan =
            provider_import::scan(&self.env(), source, directory).map_err(|e| self.err(e))?;
        let preview = scan.preview(staged.0);
        staged.1 = Some(scan);
        Ok(preview)
    }

    pub fn provider_import_apply(
        &self,
        preview_id: u64,
        selected: Vec<Selection>,
    ) -> ApiResult<ccswitch_import::Report> {
        let _guard = self.write_guard()?;
        let staged = self
            .staged_provider_import
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let scan = staged
            .1
            .as_ref()
            .filter(|_| staged.0 == preview_id)
            .ok_or_else(|| {
                ApiError::message("预览已失效，请重新扫描 / Preview expired; scan again".into())
            })?;
        // Use exactly the private snapshot the user previewed, not a second read of the source.
        provider_import::apply(&self.env(), scan, &selected).map_err(|e| self.err(e))
    }

    pub fn provider_import_discard(&self) {
        self.staged_provider_import
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .1 = None;
    }
}
