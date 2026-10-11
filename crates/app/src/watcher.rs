//! 监听客户端配置和数据目录，变化（去抖）后刷新托盘、通知前端。
//!
//! - 配置目录不存在时监听离它最近的已存在的上级目录；目录建出来以后（比如装了 OpenCode
//!   之后才有 `~/.config/opencode`）自动换成监听它本身。
//! - 上级目录里无关文件的变化（比如 `~/.bash_history`）不触发刷新。
//! - 改目录覆盖后调用 [`WatchSet::set_env`] 换一组目录，不用重启。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ccsl_core::{Env, Tool};
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEvent, Debouncer};
use tauri::{AppHandle, Manager};

/// 合并同一批文件事件；不能按时间忽略事件，否则会漏掉其他客户端紧随其后的写入。
const DEBOUNCE: Duration = Duration::from_millis(500);

/// 需要关心的文件 / 目录。
pub(crate) fn targets(env: &Env) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Tool::ALL
        .iter()
        .flat_map(|&tool| env.live_paths(tool))
        .collect();
    out.push(env.data_dir.clone());
    out
}

fn nearest_existing(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|p| p.is_dir()).map(Path::to_path_buf)
}

/// 要监听的目录（都不递归）：文件的父目录、数据目录本身；不存在时用最近的已存在上级。
pub(crate) fn plan(env: &Env) -> BTreeSet<PathBuf> {
    let mut dirs = BTreeSet::new();
    for target in targets(env) {
        let dir = if target == env.data_dir {
            target.clone()
        } else {
            match target.parent() {
                Some(parent) => parent.to_path_buf(),
                None => continue,
            }
        };
        if let Some(existing) = nearest_existing(&dir) {
            // 不监听根目录（离配置太远，事件太多）。
            if existing.parent().is_some() {
                dirs.insert(existing);
            }
        }
    }
    dirs
}

fn is_own_temp_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".lock") || name.contains(".tmp")
}

/// 文件可能刚被删除或还没创建，只规范化已存在的上级，再拼回剩余路径。
/// macOS 的 FSEvents 会报告 `/private/var/...`，环境路径则可能仍是 `/var/...`。
fn resolve_alias(path: &Path) -> PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(real) = ancestor.canonicalize() {
            return real.join(path.strip_prefix(ancestor).expect("path ancestor"));
        }
    }
    path.to_path_buf()
}

/// 这个路径的变化需要刷新吗：是关心的文件、在数据目录里、或者是通往它们的目录。
pub(crate) fn is_relevant(env: &Env, path: &Path) -> bool {
    if is_own_temp_file(path) {
        return false;
    }
    let path = resolve_alias(path);
    if path.starts_with(resolve_alias(&env.data_dir)) {
        return true;
    }
    targets(env)
        .iter()
        .map(|target| resolve_alias(target))
        .any(|target| {
            target == path
                || target.starts_with(&path)
                || target.parent() == Some(path.as_path())
        })
}

struct Inner {
    env: Env,
    watched: BTreeSet<PathBuf>,
    debouncer: Option<Debouncer<notify::RecommendedWatcher>>,
}

impl Inner {
    /// 按当前环境重新规划，增删监听。
    fn sync(&mut self) {
        let wanted = plan(&self.env);
        let Some(debouncer) = self.debouncer.as_mut() else {
            return;
        };
        for dir in self.watched.difference(&wanted) {
            let _ = debouncer.watcher().unwatch(dir);
        }
        let mut watched = BTreeSet::new();
        for dir in &wanted {
            if self.watched.contains(dir) {
                watched.insert(dir.clone());
                continue;
            }
            match debouncer.watcher().watch(dir, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    watched.insert(dir.clone());
                }
                Err(err) => log::warn!("watch {}: {err}", dir.display()),
            }
        }
        self.watched = watched;
    }
}

/// 一组目录监听。和 Tauri 无关，便于测试。
#[derive(Clone)]
pub struct WatchSet {
    inner: Arc<Mutex<Inner>>,
}

impl WatchSet {
    pub fn new(
        env: Env,
        debounce: Duration,
        on_change: impl Fn() + Send + 'static,
    ) -> Result<Self, notify::Error> {
        let inner = Arc::new(Mutex::new(Inner {
            env,
            watched: BTreeSet::new(),
            debouncer: None,
        }));
        let weak = Arc::downgrade(&inner);
        let debouncer = new_debouncer(
            debounce,
            move |result: Result<Vec<DebouncedEvent>, notify::Error>| {
                let Ok(events) = result else { return };
                let Some(inner) = weak.upgrade() else { return };
                let relevant = {
                    let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
                    let relevant = events.iter().any(|e| is_relevant(&guard.env, &e.path));
                    if relevant {
                        // 可能有新目录建出来了。
                        guard.sync();
                    }
                    relevant
                };
                if relevant {
                    on_change();
                }
            },
        )?;
        {
            let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.debouncer = Some(debouncer);
            guard.sync();
        }
        Ok(Self { inner })
    }

    pub fn set_env(&self, env: Env) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.env != env {
            guard.env = env;
            guard.sync();
        }
    }

    #[cfg(test)]
    pub fn watched(&self) -> BTreeSet<PathBuf> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .watched
            .clone()
    }
}

pub fn start(app: &AppHandle, env: &Env) {
    let handle = app.clone();
    match WatchSet::new(env.clone(), DEBOUNCE, move || {
        crate::refresh_views(&handle);
    }) {
        Ok(set) => {
            app.manage(set);
        }
        Err(err) => log::warn!("file watcher unavailable: {err}"),
    }
}

/// 目录覆盖改了：换一组监听目录。
pub fn reconfigure(app: &AppHandle, env: Env) {
    if let Some(set) = app.try_state::<WatchSet>() {
        set.set_env(env);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    fn wait_for(mut check: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(8) {
            if check() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn sandbox() -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        (dir, Env::with_home(home))
    }

    #[test]
    fn plan_falls_back_to_the_nearest_existing_ancestor() {
        let (_dir, env) = sandbox();
        let planned = plan(&env);
        assert!(
            planned.iter().all(|d| d.starts_with(&env.home)),
            "{planned:?}"
        );
        assert!(
            planned.contains(&env.home),
            "missing dirs fall back to HOME"
        );

        std::fs::create_dir_all(&env.opencode_dir).unwrap();
        assert!(plan(&env).contains(&env.opencode_dir));
    }

    #[test]
    fn unrelated_files_do_not_trigger_refreshes() {
        let (_dir, env) = sandbox();
        assert!(!is_relevant(&env, &env.home.join(".bash_history")));
        assert!(is_relevant(&env, &env.claude_settings_path()));
        assert!(
            is_relevant(&env, &env.claude_dir),
            "creating ~/.claude matters"
        );
        assert!(
            is_relevant(&env, &env.home.join(".config")),
            "on the way to opencode"
        );
        assert!(is_relevant(&env, &env.data_dir.join("providers.json")));
        assert!(!is_relevant(
            &env,
            &env.data_dir.join("providers.json.lock")
        ));
        assert!(!is_relevant(
            &env,
            &env.claude_dir.join("settings.json.tmp123")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn canonical_events_match_aliased_paths_even_when_the_file_is_missing() {
        let (dir, env) = sandbox();
        let alias = dir.path().join("home-alias");
        std::os::unix::fs::symlink(&env.home, &alias).unwrap();
        let aliased = Env::with_home(alias);
        let real_home = env.home.canonicalize().unwrap();
        let path = real_home.join(".claude/settings.json");
        assert!(is_relevant(&aliased, &path));
        assert!(is_relevant(&aliased, &real_home.join(".claude")));
        assert!(!is_relevant(&aliased, &real_home.join("notes.txt")));

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{}").unwrap();
        assert!(is_relevant(&aliased, &path));
        std::fs::remove_file(&path).unwrap();
        assert!(is_relevant(&aliased, &path));
    }

    #[test]
    fn picks_up_config_dirs_created_after_start() {
        let (_dir, env) = sandbox();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let set = WatchSet::new(env.clone(), Duration::from_millis(100), move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        assert!(!set.watched().contains(&env.claude_dir));

        std::fs::create_dir_all(&env.claude_dir).unwrap();
        assert!(
            wait_for(|| set.watched().contains(&env.claude_dir)),
            "{:?}",
            set.watched()
        );
        let before = hits.load(Ordering::SeqCst);
        assert!(before >= 1);

        std::fs::write(env.claude_settings_path(), "{}").unwrap();
        assert!(wait_for(|| hits.load(Ordering::SeqCst) > before));

        // 无关文件不触发。
        std::thread::sleep(Duration::from_millis(400));
        let settled = hits.load(Ordering::SeqCst);
        std::fs::write(env.home.join("notes.txt"), "x").unwrap();
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(hits.load(Ordering::SeqCst), settled);
    }

    #[test]
    fn set_env_moves_the_watches() {
        let (_dir, env) = sandbox();
        let alt = env.home.join("alt-claude");
        std::fs::create_dir_all(&alt).unwrap();
        let set = WatchSet::new(env.clone(), Duration::from_millis(100), || {}).unwrap();
        assert!(!set.watched().contains(&alt));
        let overrides = ccsl_core::env::DirOverrides {
            claude_dir: Some(alt.clone()),
            ..Default::default()
        };
        set.set_env(env.clone().with_overrides(&overrides));
        assert!(set.watched().contains(&alt));
    }

    #[test]
    fn external_write_immediately_after_a_local_write_is_observed() {
        let (_dir, env) = sandbox();
        std::fs::create_dir_all(&env.claude_dir).unwrap();
        let path = env.claude_settings_path();
        let watched_path = path.clone();
        let observed = Arc::new(Mutex::new(String::new()));
        let snapshot = observed.clone();
        let _set = WatchSet::new(env, DEBOUNCE, move || {
            *snapshot.lock().unwrap() = std::fs::read_to_string(&watched_path).unwrap();
        })
        .unwrap();

        std::fs::write(&path, r#"{"writer":"ccsl"}"#).unwrap();
        assert!(wait_for(|| observed.lock().unwrap().contains("ccsl")));

        // 紧接上一次刷新，模拟客户端再次写同一配置；不能当成应用自己的回声丢掉。
        std::fs::write(&path, r#"{"writer":"external-client"}"#).unwrap();
        assert!(wait_for(|| observed.lock().unwrap().contains("external-client")));
    }
}
