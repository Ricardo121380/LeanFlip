//! Tauri 命令：每个都是「放到阻塞线程池里调 [`Service`] 的同名方法」，改了数据的再
//! 重建托盘并通知前端。

use std::path::PathBuf;

use ccsl_core::codex::login::LoginSummary;
use ccsl_core::codex::oauth::{AccountInfo, DeviceFlow};
use ccsl_core::{ccswitch_import, Provider, Tool};
use tauri::{AppHandle, State};

use crate::service::{
    ApiError, ApiResult, ImportLiveResult, OpResult, PollResult, SaveSettingsResult, Service,
    SettingsInput, SettingsView, ToolProviders, ToolSummary,
};
use crate::{notify_changed, AppState};

async fn blocking<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Service) -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    let service = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || f(&service))
        .await
        .map_err(|e| ApiError(e.to_string()))?
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
pub async fn get_overview(state: State<'_, AppState>) -> ApiResult<Vec<ToolSummary>> {
    blocking(&state, |s| s.overview()).await
}

#[tauri::command]
pub async fn list_providers(state: State<'_, AppState>, tool: Tool) -> ApiResult<ToolProviders> {
    blocking(&state, move |s| s.list_providers(tool)).await
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
    blocking(&state, |s| Ok(s.codex_accounts())).await
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
pub async fn codex_logins(state: State<'_, AppState>) -> ApiResult<Vec<LoginSummary>> {
    blocking(&state, |s| s.codex_logins()).await
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
    let autostart = input.app.autostart;
    let result = mutating(&app, &state, move |s| s.save_settings(input)).await?;
    {
        use tauri_plugin_autostart::ManagerExt;
        let launcher = app.autolaunch();
        let applied = if autostart {
            launcher.enable()
        } else if launcher.is_enabled().unwrap_or(false) {
            launcher.disable()
        } else {
            Ok(())
        };
        if let Err(err) = applied {
            log::warn!("autostart: {err}");
            return Err(ApiError(format!("autostart: {err}")));
        }
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
) -> ApiResult<ccswitch_import::Report> {
    mutating(&app, &state, move |s| s.ccswitch_apply(dir, tools)).await
}

/// 只允许打开 https 地址（设备码登录页等）。
#[tauri::command]
pub async fn open_url(app: AppHandle, url: String) -> ApiResult<()> {
    if !url.starts_with("https://") {
        return Err(ApiError("only https URLs can be opened".into()));
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| ApiError(e.to_string()))
}
