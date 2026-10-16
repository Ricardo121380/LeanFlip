//! Tauri 命令：每个都是「放到阻塞线程池里调 [`Service`] 的同名方法」，改了数据的再
//! 重建托盘并通知前端。

use std::path::PathBuf;

use leanflip_core::ccswitch_export::{self, DuplicatePolicy};
use leanflip_core::codex::login::LoginSummary;
use leanflip_core::codex::oauth::{AccountInfo, DeviceFlow};
use leanflip_core::{ccswitch_import, Provider, Tool};
use tauri::{AppHandle, State};

use crate::service::{
    ApiError, ApiResult, ImportLiveResult, OpResult, PollResult, SaveSettingsResult, SceneList,
    ScenePreview, SceneReport, SceneUndo, Service, SettingsInput, SettingsView, ToolProviders,
    ToolSummary,
};
use crate::{notify_changed, AppState};
use leanflip_core::scene::Scene;

async fn blocking<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Service) -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    let service = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || f(&service))
        .await
        .map_err(|e| ApiError::message(e.to_string()))?
}

async fn mutating<T: Send + 'static>(
    app: &AppHandle,
    state: &State<'_, AppState>,
    f: impl FnOnce(&Service) -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    let result = blocking(state, f).await;
    notify_changed(app);
    result
}

#[tauri::command]
pub async fn get_recovery_notices(state: State<'_, AppState>) -> ApiResult<Vec<String>> {
    blocking(&state, |s| Ok(s.recovery())).await
}

#[tauri::command]
pub async fn get_overview(state: State<'_, AppState>) -> ApiResult<Vec<ToolSummary>> {
    blocking(&state, |s| s.overview()).await
}

#[tauri::command]
pub async fn list_providers(state: State<'_, AppState>, tool: Tool) -> ApiResult<ToolProviders> {
    blocking(&state, move |s| s.list_providers(tool)).await
}

#[tauri::command]
pub async fn ignored_fields(
    state: State<'_, AppState>,
    tool: Tool,
    settings_config: serde_json::Value,
) -> ApiResult<Vec<String>> {
    blocking(
        &state,
        move |s| Ok(s.ignored_fields(tool, &settings_config)),
    )
    .await
}

#[tauri::command]
pub async fn probe_latency(
    state: State<'_, AppState>,
    tool: Tool,
    settings_config: serde_json::Value,
) -> ApiResult<leanflip_service::probe::ProbeResult> {
    blocking(&state, move |s| Ok(s.probe_latency(tool, &settings_config))).await
}

#[tauri::command]
pub async fn probe_models(
    state: State<'_, AppState>,
    tool: Tool,
    settings_config: serde_json::Value,
) -> ApiResult<leanflip_service::probe::ProbeResult> {
    blocking(&state, move |s| Ok(s.probe_models(tool, &settings_config))).await
}

#[tauri::command]
pub async fn probe_check(
    state: State<'_, AppState>,
    tool: Tool,
    settings_config: serde_json::Value,
    model: Option<String>,
) -> ApiResult<leanflip_service::probe::ProbeResult> {
    blocking(&state, move |s| {
        Ok(s.probe_check(tool, &settings_config, model.as_deref()))
    })
    .await
}

#[tauri::command]
pub async fn probe_efforts(
    state: State<'_, AppState>,
    tool: Tool,
    settings_config: serde_json::Value,
    model: Option<String>,
) -> ApiResult<leanflip_service::probe::ProbeResult> {
    blocking(&state, move |s| {
        Ok(s.probe_efforts(tool, &settings_config, model.as_deref()))
    })
    .await
}

#[tauri::command]
pub async fn usage_report(
    state: State<'_, AppState>,
) -> ApiResult<leanflip_service::usage::UsageReport> {
    blocking(&state, |s| s.usage_report()).await
}

#[tauri::command]
pub async fn quota_overview(
    state: State<'_, AppState>,
) -> ApiResult<leanflip_service::quota::QuotaView> {
    blocking(&state, |s| s.quota_overview()).await
}

/// 查到的额度写进了缓存：托盘和菜单栏文字马上跟着变。发起查询的页面已经拿到了结果，
/// 不用再通知前端。
#[tauri::command]
pub async fn quota_refresh(
    app: AppHandle,
    state: State<'_, AppState>,
    keys: Option<Vec<String>>,
) -> ApiResult<leanflip_service::quota::QuotaView> {
    let result = blocking(&state, move |s| s.quota_refresh(keys)).await;
    crate::tray::rebuild(&app);
    result
}

#[tauri::command]
pub async fn quota_detect(
    app: AppHandle,
    state: State<'_, AppState>,
    tool: Tool,
    settings_config: serde_json::Value,
    quota_per_usd: Option<f64>,
) -> ApiResult<leanflip_service::quota::relay::QuotaDetect> {
    let result = blocking(&state, move |s| {
        s.quota_detect(tool, &settings_config, quota_per_usd)
    })
    .await;
    crate::tray::rebuild(&app);
    result
}

#[tauri::command]
pub async fn quota_configure(
    app: AppHandle,
    state: State<'_, AppState>,
    tool: Tool,
    id: String,
    settings: Option<leanflip_service::quota::relay::QuotaSettings>,
) -> ApiResult<()> {
    mutating(&app, &state, move |s| {
        s.quota_configure(tool, &id, settings)
    })
    .await
}

#[tauri::command]
pub async fn switch_provider(
    app: AppHandle,
    state: State<'_, AppState>,
    tool: Tool,
    id: String,
) -> ApiResult<OpResult> {
    mutating(&app, &state, move |s| s.switch_provider(tool, &id)).await
}

#[tauri::command]
pub async fn add_provider(
    app: AppHandle,
    state: State<'_, AppState>,
    tool: Tool,
    provider: Provider,
    add_to_live: bool,
) -> ApiResult<OpResult> {
    mutating(&app, &state, move |s| {
        s.add_provider(tool, provider, add_to_live)
    })
    .await
}

#[tauri::command]
pub async fn update_provider(
    app: AppHandle,
    state: State<'_, AppState>,
    tool: Tool,
    provider: Provider,
) -> ApiResult<OpResult> {
    mutating(&app, &state, move |s| s.update_provider(tool, provider)).await
}

#[tauri::command]
pub async fn delete_provider(
    app: AppHandle,
    state: State<'_, AppState>,
    tool: Tool,
    id: String,
) -> ApiResult<()> {
    mutating(&app, &state, move |s| s.delete_provider(tool, &id)).await
}

#[tauri::command]
pub async fn set_in_live(
    app: AppHandle,
    state: State<'_, AppState>,
    tool: Tool,
    id: String,
    enabled: bool,
) -> ApiResult<OpResult> {
    mutating(&app, &state, move |s| s.set_in_live(tool, &id, enabled)).await
}

// ---- 场景 ----

#[tauri::command]
pub async fn list_scenes(state: State<'_, AppState>) -> ApiResult<SceneList> {
    blocking(&state, |s| s.scenes()).await
}

#[tauri::command]
pub async fn capture_scene(
    state: State<'_, AppState>,
    tools: Option<Vec<Tool>>,
) -> ApiResult<Scene> {
    blocking(&state, move |s| s.scene_capture(tools)).await
}

#[tauri::command]
pub async fn save_scene(
    app: AppHandle,
    state: State<'_, AppState>,
    scene: Scene,
) -> ApiResult<Scene> {
    mutating(&app, &state, move |s| s.save_scene(scene)).await
}

#[tauri::command]
pub async fn delete_scene(app: AppHandle, state: State<'_, AppState>, id: String) -> ApiResult<()> {
    mutating(&app, &state, move |s| s.delete_scene(&id)).await
}

#[tauri::command]
pub async fn preview_scene(state: State<'_, AppState>, id: String) -> ApiResult<ScenePreview> {
    blocking(&state, move |s| s.scene_preview(&id)).await
}

#[tauri::command]
pub async fn apply_scene(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    tools: Option<Vec<Tool>>,
) -> ApiResult<SceneReport> {
    mutating(&app, &state, move |s| s.apply_scene(&id, tools)).await
}

#[tauri::command]
pub async fn undo_scene(
    app: AppHandle,
    state: State<'_, AppState>,
    undo: SceneUndo,
) -> ApiResult<SceneReport> {
    mutating(&app, &state, move |s| s.undo_scene(undo)).await
}

#[tauri::command]
pub async fn import_live(
    app: AppHandle,
    state: State<'_, AppState>,
    tool: Tool,
) -> ApiResult<ImportLiveResult> {
    mutating(&app, &state, move |s| s.import_live(tool)).await
}

#[tauri::command]
pub async fn codex_accounts(state: State<'_, AppState>) -> ApiResult<Vec<AccountInfo>> {
    blocking(&state, |s| s.codex_accounts()).await
}

#[tauri::command]
pub async fn codex_start_login(
    state: State<'_, AppState>,
    target: Option<String>,
) -> ApiResult<DeviceFlow> {
    blocking(&state, move |s| s.codex_start_login(target.as_deref())).await
}

#[tauri::command]
pub async fn codex_poll_login(
    app: AppHandle,
    state: State<'_, AppState>,
    device_code: String,
) -> ApiResult<PollResult> {
    let result = blocking(&state, move |s| s.codex_poll_login(&device_code)).await;
    if matches!(result, Ok(PollResult::Added { .. })) {
        notify_changed(&app);
    }
    result
}

#[tauri::command]
pub async fn codex_cancel_login(
    state: State<'_, AppState>,
    device_code: String,
) -> ApiResult<bool> {
    blocking(&state, move |s| Ok(s.codex_cancel_login(&device_code))).await
}

#[tauri::command]
pub async fn codex_set_default(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> ApiResult<()> {
    mutating(&app, &state, move |s| s.codex_set_default(&id)).await
}

#[tauri::command]
pub async fn codex_remove_account(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> ApiResult<()> {
    mutating(&app, &state, move |s| s.codex_remove_account(&id)).await
}

#[tauri::command]
pub async fn codex_logins(
    app: AppHandle,
    state: State<'_, AppState>,
) -> ApiResult<Vec<LoginSummary>> {
    let before = state.0.recovery().len();
    let result = blocking(&state, |s| s.codex_logins()).await;
    // 登录查询本身可能补完之前的写入；提示查询可能已先返回，需要再刷新一次。
    if state.0.recovery().len() != before {
        notify_changed(&app);
    }
    result
}

#[tauri::command]
pub async fn codex_set_login_ref(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    identity: Option<String>,
) -> ApiResult<OpResult> {
    mutating(&app, &state, move |s| {
        s.codex_set_login_ref(&id, identity.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> ApiResult<SettingsView> {
    blocking(&state, |s| s.settings()).await
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SettingsInput,
) -> ApiResult<SaveSettingsResult> {
    leanflip_service::settings::validate(&input.app)
        .map_err(|e| ApiError::message(e.to_string()))?;
    let autostart = input.app.autostart;
    let previous = state.0.app_settings().autostart;
    use tauri_plugin_autostart::ManagerExt;
    let launcher = app.autolaunch();
    if autostart != previous {
        let applied = if autostart {
            launcher.enable()
        } else {
            launcher.disable()
        };
        applied.map_err(|e| ApiError::message(format!("autostart: {e}")))?;
    }
    let result = match mutating(&app, &state, move |s| s.save_settings(input)).await {
        Ok(result) => result,
        Err(error) => {
            if autostart != previous {
                let rollback = if previous {
                    launcher.enable()
                } else {
                    launcher.disable()
                };
                if let Err(rollback) = rollback {
                    return Err(ApiError::message(format!(
                        "{error}; autostart rollback failed: {rollback}"
                    )));
                }
            }
            return Err(error);
        }
    };
    if result.dirs_changed {
        crate::watcher::reconfigure(&app, state.0.env());
    }
    Ok(result)
}

#[tauri::command]
pub async fn ccswitch_scan(
    state: State<'_, AppState>,
    dir: Option<PathBuf>,
) -> ApiResult<ccswitch_import::Scan> {
    blocking(&state, move |s| s.ccswitch_scan(dir)).await
}

#[tauri::command]
pub async fn ccswitch_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    dir: Option<PathBuf>,
    tools: Vec<Tool>,
    accounts: Option<bool>,
) -> ApiResult<ccswitch_import::Report> {
    mutating(&app, &state, move |s| {
        s.ccswitch_apply_options(dir, tools, accounts.unwrap_or(true))
    })
    .await
}

#[tauri::command]
pub async fn backup_save(
    app: AppHandle,
    state: State<'_, AppState>,
    password: String,
) -> ApiResult<Option<PathBuf>> {
    use tauri_plugin_dialog::DialogExt;
    let password = zeroize::Zeroizing::new(password);
    blocking(&state, move |service| {
        let filename = format!(
            "leanflip-backup-{}.leanflip",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let Some(file) = app
            .dialog()
            .file()
            .add_filter("LeanFlip encrypted backup", &["leanflip"])
            .set_file_name(filename)
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = file
            .into_path()
            .map_err(|_| ApiError::message("Choose a local backup file".into()))?;
        service.backup_export_file(&path, &password)?;
        Ok(Some(path))
    })
    .await
}

#[tauri::command]
pub async fn backup_load(
    state: State<'_, AppState>,
    bytes: Vec<u8>,
    password: String,
) -> ApiResult<crate::service::BackupPreview> {
    let password = zeroize::Zeroizing::new(password);
    blocking(&state, move |service| {
        service.backup_load(&bytes, &password)
    })
    .await
}

#[tauri::command]
pub async fn backup_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    options: crate::service::RestoreOptions,
) -> ApiResult<crate::service::RestoreReport> {
    let report = mutating(&app, &state, move |service| service.backup_apply(options)).await?;
    if report.dirs_changed {
        crate::watcher::reconfigure(&app, state.0.env());
    }
    Ok(report)
}

#[tauri::command]
pub fn backup_discard(state: State<'_, AppState>) {
    state.0.backup_discard();
}

/// 读 cc-switch 导出文件：请求体是文件的原始字节（前端 `<input type="file">` 读出来的），
/// 重复策略放在 `x-duplicate-policy` 头里。
#[tauri::command]
pub async fn ccswitch_export_load(
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> ApiResult<ccswitch_export::Preview> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(ApiError::message(
            "expected the file bytes as the request body".into(),
        ));
    };
    let bytes = bytes.clone();
    let policy = match request
        .headers()
        .get("x-duplicate-policy")
        .and_then(|v| v.to_str().ok())
    {
        Some("rename") => DuplicatePolicy::Rename,
        _ => DuplicatePolicy::Skip,
    };
    blocking(&state, move |s| s.export_load(&bytes, policy)).await
}

#[tauri::command]
pub async fn ccswitch_export_preview(
    state: State<'_, AppState>,
    policy: DuplicatePolicy,
) -> ApiResult<ccswitch_export::Preview> {
    blocking(&state, move |s| s.export_preview(policy)).await
}

#[tauri::command]
pub async fn ccswitch_export_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    tools: Vec<Tool>,
    policy: DuplicatePolicy,
) -> ApiResult<ccswitch_export::Report> {
    mutating(&app, &state, move |s| s.export_apply(tools, policy)).await
}

#[tauri::command]
pub fn ccswitch_export_discard(state: State<'_, AppState>) {
    state.0.export_discard();
}

/// 只允许打开 https 地址（设备码登录页等）。
#[tauri::command]
pub async fn open_url(app: AppHandle, url: String) -> ApiResult<()> {
    if !url.starts_with("https://") {
        return Err(ApiError::message("only https URLs can be opened".into()));
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| ApiError::message(e.to_string()))
}

#[tauri::command]
pub async fn get_common_config(
    state: State<'_, AppState>,
    tool: Tool,
) -> ApiResult<leanflip_core::common_config::CommonConfig> {
    blocking(&state, move |s| s.common_config(tool)).await
}
#[tauri::command]
pub async fn save_common_config(
    app: AppHandle,
    state: State<'_, AppState>,
    tool: Tool,
    revision: String,
    config: serde_json::Value,
) -> ApiResult<OpResult> {
    mutating(&app, &state, move |s| {
        s.save_common_config(tool, &revision, config)
    })
    .await
}
