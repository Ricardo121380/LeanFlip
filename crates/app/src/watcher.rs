//! 监听客户端配置和数据目录，变化（去抖 500ms）后刷新托盘、通知前端。
//! 只监听启动时已存在的目录（不递归）。

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use ccsl_core::{Env, Tool};
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use tauri::{AppHandle, Manager};

pub(crate) fn watch_dirs(env: &Env) -> BTreeSet<PathBuf> {
    let mut dirs: BTreeSet<PathBuf> = Tool::ALL
        .iter()
        .flat_map(|&tool| env.live_paths(tool))
        .filter_map(|p| p.parent().map(PathBuf::from))
        .collect();
    dirs.insert(env.data_dir.clone());
    dirs.retain(|d| d.is_dir());
    dirs
}

struct WatcherHandle(#[allow(dead_code)] std::sync::Mutex<Box<dyn Send>>);

pub fn start(app: &AppHandle, env: &Env) {
    let handle = app.clone();
    let debouncer = new_debouncer(Duration::from_millis(500), move |result| {
        if let Ok(events) = result {
            let events: Vec<notify_debouncer_mini::DebouncedEvent> = events;
            // 忽略自己的锁文件 / 临时文件。
            if events.iter().any(|e| {
                let name = e.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                !(name.ends_with(".lock") || name.contains(".tmp"))
            }) {
                crate::notify_changed(&handle);
            }
        }
    });
    let mut debouncer = match debouncer {
        Ok(d) => d,
        Err(err) => {
            log::warn!("file watcher unavailable: {err}");
            return;
        }
    };
    for dir in watch_dirs(env) {
        if let Err(err) = debouncer.watcher().watch(&dir, RecursiveMode::NonRecursive) {
            log::warn!("watch {}: {err}", dir.display());
        }
    }
    app.manage(WatcherHandle(std::sync::Mutex::new(Box::new(debouncer))));
}
