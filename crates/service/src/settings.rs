//! 桌面应用自己的设置（`<数据目录>/app-settings.json`）：语言、目录覆盖、开机自启、
//! 后台刷新。和供应商库分开：核心库只关心切换行为（「切到第三方时保留 Codex 登录」
//! 存在供应商库里，见 `Service::settings`）。

use std::path::{Path, PathBuf};

use leanflip_core::env::DirOverrides;
use leanflip_core::AppError;
use serde::{Deserialize, Serialize};

pub const FILENAME: &str = "app-settings.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    /// `zh` / `en`；`None` 跟随系统。
    pub language: Option<String>,
    /// 界面外观：`light` / `dark`；`None` 跟随系统。
    pub theme: Option<String>,
    pub dir_overrides: DirOverrides,
    pub autostart: bool,
    /// 后台刷新托管 ChatGPT 账号：上次取得 token 超过这么多小时就刷新（0 = 关闭）。
    pub background_refresh_hours: u32,
    /// 关闭主窗口时留在托盘。
    pub close_to_tray: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: None,
            theme: None,
            dir_overrides: DirOverrides::default(),
            autostart: false,
            background_refresh_hours: 24 * 7,
            close_to_tray: true,
        }
    }
}

/// 只保留认识的语言和外观值，其余回到跟随系统。
pub fn normalize(settings: &mut AppSettings) {
    if !matches!(settings.language.as_deref(), None | Some("zh" | "en")) {
        settings.language = None;
    }
    if !matches!(settings.theme.as_deref(), None | Some("light" | "dark")) {
        settings.theme = None;
    }
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILENAME)
}

pub fn load(data_dir: &Path) -> Result<AppSettings, AppError> {
    let path = path(data_dir);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| AppError::json(&path, e)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(err) => Err(AppError::io(&path, err)),
    }
}

pub fn save(data_dir: &Path, settings: &AppSettings) -> Result<(), AppError> {
    let bytes =
        serde_json::to_vec_pretty(settings).map_err(|source| AppError::JsonSerialize { source })?;
    leanflip_core::fsutil::atomic_write_private(&path(data_dir), &bytes)
}
