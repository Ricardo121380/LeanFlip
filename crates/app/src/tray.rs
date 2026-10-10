//! 托盘：每个工具一个子菜单。切换式工具是单选（勾 = 当前卡），并存式工具是开关
//! （勾 = 写在客户端配置里）。菜单 id：`sw:<tool>:<id>` / `cx:<tool>:<id>`。

use ccsl_core::Tool;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

use crate::AppState;

const TRAY_ID: &str = "main";

pub fn tool_label(tool: Tool) -> &'static str {
    match tool {
        Tool::Claude => "Claude Code",
        Tool::Codex => "Codex",
        Tool::Gemini => "Gemini CLI",
        Tool::GrokBuild => "Grok Build",
        Tool::ClaudeDesktop => "Claude Desktop",
        Tool::OpenCode => "OpenCode",
        Tool::OpenClaw => "OpenClaw",
        Tool::Hermes => "Hermes",
        Tool::Pi => "Pi",
        Tool::Mcode => "MCode",
    }
}

struct Labels {
    open: &'static str,
    quit: &'static str,
    empty: &'static str,
}

fn labels(english: bool) -> Labels {
    if english {
        Labels {
            open: "Open CC Switch Lite",
            quit: "Quit",
            empty: "(no providers)",
        }
    } else {
        Labels {
            open: "打开 CC Switch Lite",
            quit: "退出",
            empty: "（没有供应商）",
        }
    }
}

fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let state = app.state::<AppState>();
    let service = &state.0;
    let english = service.app_settings().language.as_deref() == Some("en");
    let text = labels(english);
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(
        app,
        "open",
        text.open,
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    for tool in Tool::ALL {
        let Ok(listing) = service.list_providers(tool) else {
            continue;
        };
        if listing.providers.is_empty() {
            continue;
        }
        let prefix = if tool.is_coexist() { "cx" } else { "sw" };
        let mut items: Vec<Box<dyn IsMenuItem<R>>> = Vec::new();
        for provider in &listing.providers {
            let checked = if tool.is_coexist() {
                listing.in_live.contains(&provider.id)
            } else {
                listing.current.as_deref() == Some(provider.id.as_str())
            };
            items.push(Box::new(CheckMenuItem::with_id(
                app,
                format!("{prefix}:{}:{}", tool.as_str(), provider.id),
                &provider.name,
                // 并存式工具的配置文件读不了时不让点（开关状态不可信）。
                listing.live_error.is_none(),
                checked,
                None::<&str>,
            )?));
        }
        if items.is_empty() {
            items.push(Box::new(MenuItem::new(
                app,
                text.empty,
                false,
                None::<&str>,
            )?));
        }
        let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(|b| b.as_ref()).collect();
        let title = match (&listing.current, tool.is_coexist()) {
            (Some(current), false) => {
                let name = listing
                    .providers
                    .iter()
                    .find(|p| &p.id == current)
                    .map(|p| p.name.as_str())
                    .unwrap_or(current.as_str());
                format!("{} · {name}", tool_label(tool))
            }
            (_, true) => format!("{} · {}", tool_label(tool), listing.in_live.len()),
            _ => tool_label(tool).to_string(),
        };
        menu.append(&Submenu::with_items(app, title, true, &refs)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        text.quit,
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// 解析 `sw:<tool>:<id>` / `cx:<tool>:<id>`。
pub(crate) fn parse_menu_id(id: &str) -> Option<(bool, Tool, String)> {
    let mut parts = id.splitn(3, ':');
    let coexist = match parts.next()? {
        "sw" => false,
        "cx" => true,
        _ => return None,
    };
    let tool: Tool = parts.next()?.parse().ok()?;
    let provider = parts.next()?.to_string();
    (!provider.is_empty() && tool.is_coexist() == coexist).then_some((coexist, tool, provider))
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "open" => show_main(app),
        "quit" => app.exit(0),
        other => {
            let Some((coexist, tool, provider)) = parse_menu_id(other) else {
                return;
            };
            let app = app.clone();
            std::thread::spawn(move || {
                let service = app.state::<AppState>().0.clone();
                let result = if coexist {
                    let enabled = service
                        .list_providers(tool)
                        .map(|l| !l.in_live.contains(&provider))
                        .unwrap_or(true);
                    service.set_in_live(tool, &provider, enabled)
                } else {
                    service.switch_provider(tool, &provider)
                };
                if let Err(err) = result {
                    log::warn!("tray action {tool}/{provider} failed: {err}");
                }
                // 失败时也要重建：CheckMenuItem 点击后自己翻转了勾选，要改回来。
                crate::notify_changed(&app);
            });
        }
    }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("CC Switch Lite")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()));
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub fn rebuild<R: Runtime>(app: &AppHandle<R>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    match build_menu(app) {
        Ok(menu) => {
            if let Err(err) = tray.set_menu(Some(menu)) {
                log::warn!("tray set_menu: {err}");
            }
        }
        Err(err) => log::warn!("tray menu: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_round_trip_and_reject_mismatched_kinds() {
        assert_eq!(
            parse_menu_id("sw:codex:my:id"),
            Some((false, Tool::Codex, "my:id".into()))
        );
        assert_eq!(
            parse_menu_id("cx:opencode:a"),
            Some((true, Tool::OpenCode, "a".into()))
        );
        assert_eq!(parse_menu_id("cx:codex:a"), None);
        assert_eq!(parse_menu_id("sw:codex:"), None);
        assert_eq!(parse_menu_id("open"), None);
    }
}
