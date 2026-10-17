//! LeanFlip 桌面应用：Tauri 2 外壳，业务都在 `leanflip-core` 和 [`service`] 里。

pub mod commands;
mod krill_login;
pub use leanflip_service::{oauth_http, redact, service, settings};
mod tray;
mod watcher;

use std::sync::Arc;
use std::time::Duration;

use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use crate::service::Service;

/// 供应商 / 客户端配置变了（托盘操作、文件监听、后台刷新）。前端收到后重新拉数据。
pub const EVENT_PROVIDERS_CHANGED: &str = "providers-changed";
/// 托盘里应用了场景，载荷是 [`tray::SceneEvent`]。
pub const EVENT_SCENE_APPLIED: &str = "scene-applied";
/// 场景的提醒规则检查结果变了。载荷是刚变成满足、要提醒的 [`service::SceneSuggestion`]，
/// 只是建议变化（不提醒）时为 `null`。
pub const EVENT_SCENE_SUGGESTED: &str = "scene-suggested";

#[derive(Clone)]
pub struct AppState(pub Arc<Service>);

/// 应用自己改了数据：立即请求重建托盘菜单、通知前端。
pub fn notify_changed(app: &tauri::AppHandle) {
    refresh_views(app);
}

/// 请求重建托盘菜单（后台线程，合并，不阻塞调用方）、通知前端。
fn refresh_views(app: &tauri::AppHandle) {
    tray::rebuild(app);
    let _ = app.emit(EVENT_PROVIDERS_CHANGED, ());
}

fn start_background_refresh(app: tauri::AppHandle, service: Arc<Service>) {
    std::thread::Builder::new()
        .name("codex-oauth-refresh".into())
        .spawn(move || {
            // 启动后先等一会儿，别和首屏抢。
            std::thread::sleep(Duration::from_secs(60));
            loop {
                let results = match service.refresh_stale_accounts() {
                    Ok(results) => results,
                    Err(err) => {
                        log::warn!("background account refresh failed: {err}");
                        std::thread::sleep(Duration::from_secs(60 * 60));
                        continue;
                    }
                };
                let mut changed = false;
                for (id, result) in results {
                    match result {
                        Ok(refreshed) => changed |= refreshed,
                        Err(err) => {
                            changed = true;
                            log::warn!("background refresh of account {id} failed: {err}");
                        }
                    }
                }
                if changed {
                    notify_changed(&app);
                }
                std::thread::sleep(Duration::from_secs(60 * 60));
            }
        })
        .expect("spawn refresh thread");
}

/// 额度：每分钟看一次有没有到点该自动刷新的中转来源；没有时每 5 分钟重建一次托盘，
/// 带上 Codex 本机记录里的新数字（重建前会比较，没变化就不动菜单）。
fn start_quota_refresh(app: tauri::AppHandle, service: Arc<Service>) {
    std::thread::Builder::new()
        .name("quota-refresh".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_secs(30));
            let mut tick: u64 = 0;
            loop {
                match service.quota_refresh_due() {
                    Ok(true) => notify_changed(&app),
                    Ok(false) if tick.is_multiple_of(5) => tray::rebuild(&app),
                    Ok(false) => {}
                    Err(err) => log::warn!("quota auto refresh failed: {err}"),
                }
                tick += 1;
                std::thread::sleep(Duration::from_secs(60));
            }
        })
        .expect("spawn quota thread");
}

/// 每分钟检查一次场景的提醒规则；只提醒，不切换。
fn start_scene_rules(app: tauri::AppHandle, service: Arc<Service>) {
    std::thread::Builder::new()
        .name("scene-rules".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            loop {
                match service.refresh_scene_hint() {
                    Ok(update) if update.changed || update.notify.is_some() => {
                        tray::rebuild(&app);
                        let _ = app.emit(EVENT_SCENE_SUGGESTED, update.notify);
                    }
                    Ok(_) => {}
                    Err(err) => log::warn!("scene rules check failed: {err}"),
                }
                std::thread::sleep(Duration::from_secs(60));
            }
        })
        .expect("spawn scene rules thread");
}

fn app_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

pub fn run() {
    let _ = env_logger_init();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let transport = oauth_http::HttpTransport::new(oauth_http::Endpoints::default())
                .map(|t| {
                    Arc::new(t) as Arc<dyn leanflip_core::codex::oauth::transport::OAuthTransport>
                })
                .map_err(|e| log::error!("http client: {e}"))
                .ok();
            // 0.1.x kept its data in the previous data directory; move it before reading anything.
            if let Err(error) =
                leanflip_core::env::migrate_legacy_data_dir(&leanflip_core::Env::system_home())
            {
                log::warn!("data directory migration skipped: {error}");
            }
            let service = match Service::start(
                leanflip_core::Env::from_system(),
                transport,
            ) {
                Ok(service) => Arc::new(service),
                Err(error) => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.hide();
                    }
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(format!("LeanFlip 无法安全启动，请修复下列文件后重试。\nLeanFlip could not start safely. Repair the file below and try again.\n\n{error}"))
                        .title("LeanFlip")
                        .kind(MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                    return Ok(());
                }
            };
            app.manage(AppState(service.clone()));
            // 只放占位菜单，读取 Tool::ALL 的配置在托盘后台线程里做，不挡窗口显示。
            tray::create(app.handle())?;
            watcher::start(app.handle(), &service.env());
            start_background_refresh(app.handle().clone(), service.clone());
            start_scene_rules(app.handle().clone(), service.clone());
            start_quota_refresh(app.handle().clone(), service);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    state.0.backup_discard();
                }
                let close_to_tray = window
                    .app_handle()
                    .try_state::<AppState>()
                    .is_some_and(|s| s.0.app_settings().close_to_tray);
                if close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_recovery_notices,
            commands::get_overview,
            commands::get_common_config,
            commands::save_common_config,
            commands::list_providers,
            commands::ignored_fields,
            commands::claude_onboarding_path,
            commands::complete_claude_onboarding,
            commands::credential_commands,
            commands::approve_credential_commands,
            commands::probe_latency,
            commands::probe_models,
            commands::probe_check,
            commands::probe_efforts,
            commands::usage_report,
            commands::quota_overview,
            commands::quota_refresh,
            commands::quota_detect,
            commands::quota_account_detect,
            commands::quota_krill_detect,
            krill_login::quota_krill_login,
            krill_login::quota_krill_login_cancel,
            commands::quota_configure,
            commands::switch_provider,
            commands::add_provider,
            commands::update_provider,
            commands::delete_provider,
            commands::set_in_live,
            commands::list_scenes,
            commands::capture_scene,
            commands::save_scene,
            commands::delete_scene,
            commands::preview_scene,
            commands::apply_scene,
            commands::undo_scene,
            commands::import_live,
            commands::codex_accounts,
            commands::codex_start_login,
            commands::codex_poll_login,
            commands::codex_cancel_login,
            commands::codex_set_default,
            commands::codex_remove_account,
            commands::codex_logins,
            commands::codex_set_login_ref,
            commands::get_settings,
            commands::save_settings,
            commands::ccswitch_scan,
            commands::ccswitch_apply,
            commands::ccswitch_export_load,
            commands::ccswitch_export_preview,
            commands::ccswitch_export_apply,
            commands::ccswitch_export_discard,
            commands::backup_save,
            commands::backup_load,
            commands::backup_apply,
            commands::backup_discard,
            commands::open_url,
        ])
        .run(app_context())
        .expect("error while running LeanFlip");
}

/// 简单的 stderr 日志（`RUST_LOG` 不设时只打 warn 以上）。
fn env_logger_init() -> Result<(), log::SetLoggerError> {
    struct Stderr(log::LevelFilter);
    impl log::Log for Stderr {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= self.0
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!(
                    "[{}] {}: {}",
                    record.level(),
                    record.target(),
                    record.args()
                );
            }
        }
        fn flush(&self) {}
    }
    let level = match std::env::var("RUST_LOG").as_deref() {
        Ok("debug") => log::LevelFilter::Debug,
        Ok("info") => log::LevelFilter::Info,
        _ => log::LevelFilter::Warn,
    };
    log::set_logger(Box::leak(Box::new(Stderr(level))))?;
    log::set_max_level(level);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn generated_window_icon_has_rgba8_pixels_for_the_tray() {
        let context = super::app_context();
        let icon = context.default_window_icon().expect("window and tray icon");
        assert_eq!(
            icon.rgba().len(),
            (icon.width() * icon.height() * 4) as usize,
            "the tray requires four bytes per pixel"
        );
    }
}
