//! 集成测试的沙箱：每个测试一个临时 HOME，绝不碰真实的 `~/.claude`、`~/.codex`。
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use leanflip_core::live::engine::digest;
use leanflip_core::{store, Env, Provider, Tool};
use serde_json::Value;

pub struct Sandbox {
    _dir: tempfile::TempDir,
    pub env: Env,
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = Env::with_home(dir.path());
        // 防呆：沙箱必须在临时目录里。
        let real_home = dirs::home_dir();
        assert!(real_home.as_deref() != Some(dir.path()));
        assert!(env.claude_dir.starts_with(dir.path()));
        assert!(env.codex_dir.starts_with(dir.path()));
        for path in [
            &env.gemini_dir,
            &env.grok_dir,
            &env.claude_desktop_root,
            &env.opencode_dir,
            &env.openclaw_dir,
            &env.hermes_dir,
            &env.pi_agent_dir,
            &env.mcode_dir,
        ] {
            assert!(path.starts_with(dir.path()), "{}", path.display());
        }
        Self { _dir: dir, env }
    }

    pub fn home(&self) -> &Path {
        &self.env.home
    }

    pub fn write(&self, path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    pub fn read(&self, path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    pub fn json(&self, path: &Path) -> Value {
        serde_json::from_str(&self.read(path)).unwrap()
    }

    pub fn add(&self, tool: Tool, provider: Provider) {
        store::add_provider(&self.env, tool, provider).unwrap();
    }

    pub fn current(&self, tool: Tool) -> Option<String> {
        store::load(&self.env)
            .unwrap()
            .app(tool)
            .and_then(|app| app.current.clone())
    }

    pub fn set_preserve(&self, preserve: bool) {
        store::update(&self.env, |data| {
            data.settings.preserve_codex_official_auth_on_switch = preserve;
            Ok(())
        })
        .unwrap();
    }

    /// 首写备份：`<data_dir>/backups/live-first-write/<hash12>-<文件名>`。
    pub fn first_write_backup(&self, path: &Path) -> PathBuf {
        let key = digest(Some(path.to_string_lossy().as_bytes())).unwrap();
        let name = path.file_name().unwrap().to_string_lossy();
        self.env
            .data_dir
            .join("backups/live-first-write")
            .join(format!("{}-{name}", &key[..12]))
    }

    /// 沙箱里所有残留的临时文件（原子写的 `*.tmp.*`）。
    pub fn temp_files(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        walk(self.home(), &mut found);
        found
            .into_iter()
            .filter(|path| path.to_string_lossy().contains(".tmp."))
            .collect()
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[cfg(unix)]
pub fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
pub fn inode(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).unwrap().ino()
}
