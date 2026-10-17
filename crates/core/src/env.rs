//! 路径解析：用户目录、各工具的配置目录、LeanFlip 自己的数据目录。
//!
//! 不用进程级的全局 home，而是把所有路径收进一个 [`Env`] 值显式传下去：测试各用自己的
//! 临时目录，互不干扰，也不可能误碰真实的 `~/.claude`、`~/.codex`。

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::live::engine::DeviceStore;

/// LeanFlip 的数据目录名（和 CC Switch 的 `~/.cc-switch` 分开，两者可以并存）。
pub const DATA_DIR_NAME: &str = ".leanflip";
/// 0.1.x 版本使用的数据目录名。没迁移前继续使用；[`migrate_legacy_data_dir`] 把它改名为
/// [`DATA_DIR_NAME`]，并在原处留一个符号链接。
pub const LEGACY_DATA_DIR_NAME: &str = ".cc-switch-lite";

/// `home` 下实际使用的数据目录：新目录存在就用它；只有旧目录（还没迁移）时继续用旧目录，
/// 只读查询因此不需要先迁移；都没有时是新目录。
pub fn data_dir_in(home: &Path) -> PathBuf {
    let current = home.join(DATA_DIR_NAME);
    let legacy = home.join(LEGACY_DATA_DIR_NAME);
    if std::fs::symlink_metadata(&current).is_err()
        && std::fs::symlink_metadata(&legacy).is_ok_and(|meta| meta.is_dir())
    {
        return legacy;
    }
    current
}

/// 把旧数据目录改名为新目录（同一文件系统内的原子改名，锁、journal 和备份一起搬走），
/// 然后在旧位置放一个指向新目录的相对符号链接，仍在运行或仍被使用的旧版本继续读写同一份数据。
/// 新目录已存在、旧目录不存在或旧位置本身是链接时什么都不做；可以重复调用。
/// 返回是否发生了迁移。
pub fn migrate_legacy_data_dir(home: &Path) -> Result<bool, AppError> {
    let current = home.join(DATA_DIR_NAME);
    let legacy = home.join(LEGACY_DATA_DIR_NAME);
    match std::fs::symlink_metadata(&current) {
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(AppError::io(&current, error)),
    }
    match std::fs::symlink_metadata(&legacy) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(AppError::io(&legacy, error)),
    }
    // Prepare the compatibility link first, so the old path is missing only between two renames.
    #[cfg(unix)]
    let link = {
        let link = home.join(format!("{LEGACY_DATA_DIR_NAME}.leanflip-link"));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(DATA_DIR_NAME, &link).map_err(|e| AppError::io(&link, e))?;
        link
    };
    if let Err(error) = std::fs::rename(&legacy, &current) {
        #[cfg(unix)]
        let _ = std::fs::remove_file(&link);
        return Err(AppError::io(&legacy, error));
    }
    #[cfg(unix)]
    if let Err(error) = std::fs::rename(&link, &legacy) {
        // The data moved; only the convenience link for old versions is missing.
        let _ = std::fs::remove_file(&link);
        log::warn!("could not link the previous data directory to the new one: {error}");
    }
    log::info!("moved the data directory to {}", current.display());
    Ok(true)
}

/// 一台设备上的路径集合。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    pub home: PathBuf,
    /// LeanFlip 自己的数据（供应商库、`live-state.json`、首写备份、登录暂存）。
    pub data_dir: PathBuf,
    /// Claude Code 配置目录（默认 `~/.claude`）。
    pub claude_dir: PathBuf,
    /// Codex 配置目录（默认 `~/.codex`）。
    pub codex_dir: PathBuf,
    /// Gemini CLI 配置目录（默认 `~/.gemini`）。
    pub gemini_dir: PathBuf,
    /// Grok Build 配置目录（默认 `~/.grok`）。
    pub grok_dir: PathBuf,
    /// Claude Desktop 第三方部署目录（`Claude-3p`）的父目录：
    /// macOS `~/Library/Application Support`，Windows `%LOCALAPPDATA%`，Linux
    /// `$XDG_CONFIG_HOME`（默认 `~/.config`）。
    pub claude_desktop_root: PathBuf,
    /// 普通 `Claude` 配置的父目录。Windows 使用 `%APPDATA%`，其他平台与 3p 相同。
    pub claude_desktop_normal_root: PathBuf,
    /// OpenCode 配置目录（所有平台都是 `~/.config/opencode`，OpenCode 不按平台约定）。
    pub opencode_dir: PathBuf,
    /// OpenClaw 配置目录（`~/.openclaw`）。
    pub openclaw_dir: PathBuf,
    /// Hermes 配置目录（`HERMES_HOME`；默认 `~/.hermes`，Windows `%LOCALAPPDATA%\hermes`）。
    pub hermes_dir: PathBuf,
    /// Pi 的 agent 目录（`PI_CODING_AGENT_DIR`；默认 `~/.pi/agent`）。
    pub pi_agent_dir: PathBuf,
    /// MiniMax Code 数据目录（`MINIMAX_DATA_DIR` / `MAVIS_DATA_DIR`；默认 `~/.minimax`）。
    pub mcode_dir: PathBuf,
}

impl Env {
    /// 以 `home` 为根的默认布局。
    pub fn with_home(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        Self {
            data_dir: data_dir_in(&home),
            claude_dir: home.join(".claude"),
            codex_dir: home.join(".codex"),
            gemini_dir: home.join(".gemini"),
            grok_dir: home.join(".grok"),
            claude_desktop_root: default_desktop_root(&home),
            claude_desktop_normal_root: if cfg!(windows) {
                home.join("AppData").join("Roaming")
            } else {
                default_desktop_root(&home)
            },
            opencode_dir: home.join(".config").join("opencode"),
            openclaw_dir: home.join(".openclaw"),
            hermes_dir: if cfg!(windows) {
                home.join("AppData").join("Local").join("hermes")
            } else {
                home.join(".hermes")
            },
            pi_agent_dir: home.join(".pi").join("agent"),
            mcode_dir: home.join(".minimax"),
            home,
        }
    }

    /// 真实用户的 home：`LEANFLIP_HOME` 可以显式覆盖（调试、演示用），否则是 `dirs::home_dir()`。
    pub fn system_home() -> PathBuf {
        std::env::var_os("LEANFLIP_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// 真实用户环境（home 见 [`Env::system_home`]）。Codex 目录遵循 Codex 自己的 `CODEX_HOME`。
    pub fn from_system() -> Self {
        let home = Self::system_home();
        let mut env = Self::with_home(home);
        if let Some(codex_home) = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
            env.codex_dir = PathBuf::from(codex_home);
        }
        if cfg!(windows) {
            if let Some(roaming) = std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
            {
                env.claude_desktop_normal_root = roaming;
            }
            if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
                env.claude_desktop_root = PathBuf::from(&local);
                let trimmed = local.to_string_lossy().trim().to_string();
                if !trimmed.is_empty() {
                    env.hermes_dir = PathBuf::from(trimmed).join("hermes");
                }
            }
        } else if cfg!(target_os = "linux") && !std::path::Path::new("/.flatpak-info").is_file() {
            // Flatpak 里的 XDG_CONFIG_HOME 是沙箱私有的，宿主上的 Claude Desktop 用 ~/.config。
            if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
            {
                env.claude_desktop_root = xdg;
                env.claude_desktop_normal_root = env.claude_desktop_root.clone();
            }
        }
        let trimmed_var = |name: &str| {
            std::env::var_os(name)
                .map(|v| v.to_string_lossy().trim().to_string())
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        if let Some(dir) = trimmed_var("HERMES_HOME") {
            env.hermes_dir = dir;
        }
        if let Some(dir) = trimmed_var("PI_CODING_AGENT_DIR").filter(|p| p.is_absolute()) {
            env.pi_agent_dir = dir;
        }
        if let Some(dir) = trimmed_var("MINIMAX_DATA_DIR").or_else(|| trimmed_var("MAVIS_DATA_DIR"))
        {
            env.mcode_dir = dir;
        }
        env
    }

    /// OpenCode 配置文件：已有 `opencode.jsonc` 就用它，否则 `opencode.json`。
    pub fn opencode_config_path(&self) -> PathBuf {
        let jsonc = self.opencode_dir.join("opencode.jsonc");
        if jsonc.exists() {
            return jsonc;
        }
        self.opencode_dir.join("opencode.json")
    }

    pub fn openclaw_config_path(&self) -> PathBuf {
        self.openclaw_dir.join("openclaw.json")
    }

    pub fn hermes_config_path(&self) -> PathBuf {
        self.hermes_dir.join("config.yaml")
    }

    pub fn pi_models_path(&self) -> PathBuf {
        self.pi_agent_dir.join("models.json")
    }

    pub fn mcode_config_path(&self) -> PathBuf {
        self.mcode_dir.join("config.yaml")
    }

    /// `Claude/claude_desktop_config.json`。
    pub fn claude_desktop_config_path(&self) -> PathBuf {
        self.claude_desktop_normal_root
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

/// 用户在设置里覆盖的目录（没填的用默认值）。桌面应用存在自己的设置文件里，
/// 启动时套到 [`Env::from_system`] 上。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DirOverrides {
    pub claude_dir: Option<PathBuf>,
    pub codex_dir: Option<PathBuf>,
    pub gemini_dir: Option<PathBuf>,
    pub grok_dir: Option<PathBuf>,
    pub claude_desktop_root: Option<PathBuf>,
    pub opencode_dir: Option<PathBuf>,
    pub openclaw_dir: Option<PathBuf>,
    pub hermes_dir: Option<PathBuf>,
    pub pi_agent_dir: Option<PathBuf>,
    pub mcode_dir: Option<PathBuf>,
}

impl Env {
    /// 套上目录覆盖。空路径和相对路径忽略（避免把配置写到当前目录下）。
    pub fn with_overrides(mut self, overrides: &DirOverrides) -> Self {
        // An explicit common root retains the existing custom-layout contract.
        if let Some(root) = overrides
            .claude_desktop_root
            .as_ref()
            .filter(|p| p.is_absolute())
        {
            self.claude_desktop_normal_root = root.clone();
        }
        let pairs: [(&Option<PathBuf>, &mut PathBuf); 10] = [
            (&overrides.claude_dir, &mut self.claude_dir),
            (&overrides.codex_dir, &mut self.codex_dir),
            (&overrides.gemini_dir, &mut self.gemini_dir),
            (&overrides.grok_dir, &mut self.grok_dir),
            (
                &overrides.claude_desktop_root,
                &mut self.claude_desktop_root,
            ),
            (&overrides.opencode_dir, &mut self.opencode_dir),
            (&overrides.openclaw_dir, &mut self.openclaw_dir),
            (&overrides.hermes_dir, &mut self.hermes_dir),
            (&overrides.pi_agent_dir, &mut self.pi_agent_dir),
            (&overrides.mcode_dir, &mut self.mcode_dir),
        ];
        for (value, slot) in pairs {
            if let Some(path) = value.as_ref().filter(|p| p.is_absolute()) {
                *slot = path.clone();
            }
        }
        self
    }

    /// 各工具的客户端配置文件（给文件监听和界面显示用）。
    pub fn live_paths(&self, tool: crate::provider::Tool) -> Vec<PathBuf> {
        use crate::provider::Tool;
        match tool {
            Tool::Claude => vec![self.claude_settings_path()],
            Tool::Codex => vec![self.codex_config_path(), self.codex_auth_path()],
            Tool::Gemini => vec![self.gemini_env_path(), self.gemini_settings_path()],
            Tool::GrokBuild => vec![self.grok_config_path()],
            Tool::ClaudeDesktop => vec![
                self.claude_desktop_config_path(),
                self.claude_desktop_3p_config_path(),
                self.claude_desktop_library_dir().join("_meta.json"),
            ],
            Tool::OpenCode => vec![self.opencode_config_path()],
            Tool::OpenClaw => vec![self.openclaw_config_path()],
            Tool::Hermes => vec![self.hermes_config_path()],
            Tool::Pi => vec![self.pi_models_path()],
            Tool::Mcode => vec![self.mcode_config_path()],
        }
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
        assert_eq!(env.data_dir, dir.path().join(".leanflip"));
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

    #[test]
    fn an_unmigrated_legacy_directory_stays_in_use_until_it_is_moved() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_DATA_DIR_NAME);
        std::fs::create_dir_all(legacy.join("backups")).unwrap();
        std::fs::write(legacy.join("providers.json"), r#"{"version":1}"#).unwrap();
        assert_eq!(Env::with_home(dir.path()).data_dir, legacy);

        assert!(migrate_legacy_data_dir(dir.path()).unwrap());
        let current = dir.path().join(DATA_DIR_NAME);
        assert_eq!(Env::with_home(dir.path()).data_dir, current);
        assert_eq!(
            std::fs::read_to_string(current.join("providers.json")).unwrap(),
            r#"{"version":1}"#
        );
        assert!(current.join("backups").is_dir());
        #[cfg(unix)]
        {
            assert!(std::fs::symlink_metadata(&legacy)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(
                std::fs::read_link(&legacy).unwrap(),
                PathBuf::from(DATA_DIR_NAME)
            );
            // Old versions reading the previous path see the same files.
            assert_eq!(
                std::fs::read_to_string(legacy.join("providers.json")).unwrap(),
                r#"{"version":1}"#
            );
        }
        assert!(!migrate_legacy_data_dir(dir.path()).unwrap(), "idempotent");
    }

    #[test]
    fn migration_never_replaces_an_existing_new_directory() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_DATA_DIR_NAME);
        let current = dir.path().join(DATA_DIR_NAME);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("providers.json"), "old").unwrap();
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("providers.json"), "new").unwrap();
        assert!(!migrate_legacy_data_dir(dir.path()).unwrap());
        assert_eq!(
            std::fs::read_to_string(legacy.join("providers.json")).unwrap(),
            "old"
        );
        assert_eq!(
            std::fs::read_to_string(current.join("providers.json")).unwrap(),
            "new"
        );
        assert_eq!(Env::with_home(dir.path()).data_dir, current);

        let empty = tempfile::tempdir().unwrap();
        assert!(!migrate_legacy_data_dir(empty.path()).unwrap());
        assert!(
            !empty.path().join(DATA_DIR_NAME).exists(),
            "nothing is created"
        );
    }

    #[test]
    fn overrides_only_take_absolute_paths() {
        let home = tempfile::tempdir().unwrap();
        let custom = home.path().join("custom/codex");
        let env = Env::with_home(home.path()).with_overrides(&DirOverrides {
            codex_dir: Some(custom.clone()),
            hermes_dir: Some("relative".into()),
            ..DirOverrides::default()
        });
        assert_eq!(env.codex_dir, custom);
        assert_eq!(env.hermes_dir, Env::with_home(home.path()).hermes_dir);
        assert_eq!(env.data_dir, home.path().join(DATA_DIR_NAME));
    }

    #[test]
    fn desktop_roots_follow_the_platform_and_explicit_override() {
        let home = tempfile::tempdir().unwrap();
        let env = Env::with_home(home.path());
        if cfg!(windows) {
            assert_eq!(
                env.claude_desktop_config_path(),
                home.path()
                    .join("AppData/Roaming/Claude/claude_desktop_config.json")
            );
            assert_eq!(
                env.claude_desktop_3p_config_path(),
                home.path()
                    .join("AppData/Local/Claude-3p/claude_desktop_config.json")
            );
        } else {
            assert_eq!(env.claude_desktop_normal_root, env.claude_desktop_root);
        }
        let root = home.path().join("custom");
        let env = env.with_overrides(&DirOverrides {
            claude_desktop_root: Some(root.clone()),
            ..Default::default()
        });
        assert_eq!(
            env.claude_desktop_config_path(),
            root.join("Claude/claude_desktop_config.json")
        );
        assert_eq!(
            env.claude_desktop_3p_config_path(),
            root.join("Claude-3p/claude_desktop_config.json")
        );
    }
}
