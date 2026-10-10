//! 路径解析：用户目录、各工具的配置目录、ccswitch-lite 自己的数据目录。
//!
//! 和 cc-switch 不同，这里不用进程级的全局 home（`CC_SWITCH_TEST_HOME` 那一套），
//! 而是把所有路径收进一个 [`Env`] 值显式传下去：测试各用自己的临时目录，互不干扰，
//! 也不可能误碰真实的 `~/.claude`、`~/.codex`。

use std::path::PathBuf;

use crate::live::engine::DeviceStore;

/// ccswitch-lite 的数据目录名（和 cc-switch 的 `~/.cc-switch` 分开，两者可以并存）。
pub const DATA_DIR_NAME: &str = ".cc-switch-lite";

/// 一台设备上的路径集合。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    pub home: PathBuf,
    /// ccswitch-lite 自己的数据（供应商库、`live-state.json`、首写备份、登录暂存）。
    pub data_dir: PathBuf,
    /// Claude Code 配置目录（默认 `~/.claude`）。
    pub claude_dir: PathBuf,
    /// Codex 配置目录（默认 `~/.codex`）。
    pub codex_dir: PathBuf,
    /// Gemini CLI 配置目录（默认 `~/.gemini`）。
    pub gemini_dir: PathBuf,
    /// Grok Build 配置目录（默认 `~/.grok`）。
    pub grok_dir: PathBuf,
    /// Claude Desktop 的普通配置目录（`Claude`）和第三方部署目录（`Claude-3p`）的父目录：
    /// macOS `~/Library/Application Support`，Windows `%LOCALAPPDATA%`，Linux
    /// `$XDG_CONFIG_HOME`（默认 `~/.config`）。
    pub claude_desktop_root: PathBuf,
}

impl Env {
    /// 以 `home` 为根的默认布局。
    pub fn with_home(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        Self {
            data_dir: home.join(DATA_DIR_NAME),
            claude_dir: home.join(".claude"),
            codex_dir: home.join(".codex"),
            gemini_dir: home.join(".gemini"),
            grok_dir: home.join(".grok"),
            claude_desktop_root: default_desktop_root(&home),
            home,
        }
    }

    /// 真实用户环境：`dirs::home_dir()`；`CCSL_HOME` 可以显式覆盖（调试用）。
    /// Codex 目录遵循 Codex 自己的 `CODEX_HOME`。
    pub fn from_system() -> Self {
        let home = std::env::var_os("CCSL_HOME")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let mut env = Self::with_home(home);
        if let Some(codex_home) = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
            env.codex_dir = PathBuf::from(codex_home);
        }
        if cfg!(windows) {
            if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
                env.claude_desktop_root = PathBuf::from(local);
            }
        } else if cfg!(target_os = "linux") && !std::path::Path::new("/.flatpak-info").is_file() {
            // Flatpak 里的 XDG_CONFIG_HOME 是沙箱私有的，宿主上的 Claude Desktop 用 ~/.config。
            if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
            {
                env.claude_desktop_root = xdg;
            }
        }
        env
    }

    /// `Claude/claude_desktop_config.json`。
    pub fn claude_desktop_config_path(&self) -> PathBuf {
        self.claude_desktop_root
            .join("Claude")
            .join("claude_desktop_config.json")
    }

    /// `Claude-3p/claude_desktop_config.json`。
    pub fn claude_desktop_3p_config_path(&self) -> PathBuf {
        self.claude_desktop_root
            .join("Claude-3p")
            .join("claude_desktop_config.json")
    }

    /// `Claude-3p/configLibrary`（第三方 profile 和 `_meta.json`）。
    pub fn claude_desktop_library_dir(&self) -> PathBuf {
        self.claude_desktop_root
            .join("Claude-3p")
            .join("configLibrary")
    }

    pub fn device_store(&self) -> DeviceStore {
        DeviceStore::at(&self.data_dir)
    }

    /// Claude Code 的 `settings.json`；旧安装若只有 `claude.json` 则继续用它。
    pub fn claude_settings_path(&self) -> PathBuf {
        let settings = self.claude_dir.join("settings.json");
        if settings.exists() {
            return settings;
        }
        let legacy = self.claude_dir.join("claude.json");
        if legacy.exists() {
            return legacy;
        }
        settings
    }

    pub fn codex_config_path(&self) -> PathBuf {
        self.codex_dir.join("config.toml")
    }

    pub fn codex_auth_path(&self) -> PathBuf {
        self.codex_dir.join("auth.json")
    }

    pub fn gemini_env_path(&self) -> PathBuf {
        self.gemini_dir.join(".env")
    }

    pub fn gemini_settings_path(&self) -> PathBuf {
        self.gemini_dir.join("settings.json")
    }

    pub fn grok_config_path(&self) -> PathBuf {
        self.grok_dir.join("config.toml")
    }

    /// 供应商库文件。
    pub fn store_path(&self) -> PathBuf {
        self.data_dir.join("providers.json")
    }

    pub fn data_file(&self, name: &str) -> PathBuf {
        self.data_dir.join(name)
    }
}

fn default_desktop_root(home: &std::path::Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else if cfg!(windows) {
        home.join("AppData").join("Local")
    } else {
        home.join(".config")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_legacy_claude_file() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        assert_eq!(env.data_dir, dir.path().join(".cc-switch-lite"));
        assert_eq!(
            env.claude_settings_path(),
            dir.path().join(".claude/settings.json")
        );
        std::fs::create_dir_all(&env.claude_dir).unwrap();
        std::fs::write(env.claude_dir.join("claude.json"), "{}").unwrap();
        assert_eq!(
            env.claude_settings_path(),
            dir.path().join(".claude/claude.json")
        );
        std::fs::write(env.claude_dir.join("settings.json"), "{}").unwrap();
        assert_eq!(
            env.claude_settings_path(),
            dir.path().join(".claude/settings.json")
        );
    }
}
