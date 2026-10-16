//! 独立官网窗口完成认证；远程网页没有 LeanFlip IPC 权限。
//! 只在 Krill 的 HTTPS 主页面读取登录状态，不读取浏览器资料或密码。
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};

use leanflip_core::Tool;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use zeroize::Zeroizing;

use crate::service::{ApiError, ApiResult, KrillLogin};
use crate::AppState;

const LABEL: &str = "krill-quota-login";
const LOGIN_URL: &str = "https://www.krill-code.com/login";
const TIMEOUT: Duration = Duration::from_secs(10 * 60);

// 在页面中再次检查 origin，防止 url() 和 eval() 之间发生跨域跳转。
const READ_LOGIN: &str = include_str!("krill_login.js");

fn is_krill(url: &tauri::Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("www.krill-code.com")
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
}

fn require_main(window: &WebviewWindow) -> ApiResult<()> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(ApiError::message(
            "Unavailable outside the main window".into(),
        ))
    }
}

/// 窗口关闭即取消；成功只返回后端临时引用，JWT 不通过 IPC 回传。
#[tauri::command]
pub async fn quota_krill_login(
    app: AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
    tool: Tool,
) -> ApiResult<KrillLogin> {
    require_main(&window)?;
    if let Some(existing) = app.get_webview_window(LABEL) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Err(ApiError::message(
            "Krill 登录窗口已打开 / Krill sign-in is already open".into(),
        ));
    }
    let login = WebviewWindowBuilder::new(
        &app,
        LABEL,
        WebviewUrl::External(LOGIN_URL.parse().expect("static login URL")),
    )
    .title("登录 Krill · LeanFlip")
    .inner_size(900.0, 760.0)
    .incognito(true)
    .on_navigation(|url| url.scheme() == "https")
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .build()
    .map_err(|_| {
        ApiError::message("无法打开 Krill 登录窗口 / Could not open Krill sign-in".into())
    })?;
    let service = state.0.clone();
    let reader = login.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let closed = cancelled.clone();
    login.on_window_event(move |event| {
        if matches!(
            event,
            WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
        ) {
            closed.store(true, Ordering::SeqCst);
        }
    });
    let stopped = cancelled.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        while started.elapsed() < TIMEOUT {
            if stopped.load(Ordering::SeqCst) {
                return Err(ApiError::message(
                    "Krill 登录已取消 / Krill sign-in cancelled".into(),
                ));
            }
            let url = reader.url().map_err(|_| {
                ApiError::message("Krill 登录已取消 / Krill sign-in cancelled".into())
            })?;
            if is_krill(&url) {
                let (sender, receiver) = mpsc::sync_channel(1);
                reader
                    .eval_with_callback(READ_LOGIN, move |value| {
                        let _ = sender.send(Zeroizing::new(value));
                    })
                    .map_err(|_| {
                        ApiError::message("Krill 登录已取消 / Krill sign-in cancelled".into())
                    })?;
                if let Ok(value) = receiver.recv_timeout(Duration::from_secs(3)) {
                    if let Some(jwt) = parse_token(&value) {
                        return service.quota_krill_login_complete(tool, jwt);
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        Err(ApiError::message(
            "Krill 登录等待超时，请重试 / Krill sign-in timed out; please retry".into(),
        ))
    })
    .await
    .map_err(|_| ApiError::message("Krill 登录中断 / Krill sign-in interrupted".into()));
    let was_cancelled = cancelled.load(Ordering::SeqCst);
    let _ = login.close();
    let _ = window.set_focus();
    if was_cancelled {
        return Err(ApiError::message(
            "Krill 登录已取消 / Krill sign-in cancelled".into(),
        ));
    }
    result?
}

fn parse_token(value: &str) -> Option<String> {
    if value.len() > 32_768 {
        return None;
    }
    serde_json::from_str::<Option<String>>(value)
        .ok()
        .flatten()
        .filter(|jwt| !jwt.trim().is_empty() && jwt.len() <= 16_384)
}

#[tauri::command]
pub fn quota_krill_login_cancel(app: AppHandle, window: WebviewWindow) -> ApiResult<()> {
    require_main(&window)?;
    if let Some(login) = app.get_webview_window(LABEL) {
        let _ = login.close();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_only_the_exact_official_https_origin() {
        assert!(is_krill(&LOGIN_URL.parse().unwrap()));
        for address in [
            "http://www.krill-code.com/login",
            "https://www.krill-code.com.evil.test/",
            "https://evil.test/?krill-code.com",
            "https://www.krill-code.com:444/",
            "https://user@www.krill-code.com/",
        ] {
            assert!(!is_krill(&address.parse().unwrap()));
        }
        assert_eq!(parse_token("\"test-token\""), Some("test-token".into()));
        for value in ["null", "{}", "\"\"", "\"   \"", "broken"] {
            assert!(parse_token(value).is_none());
        }
    }
}
