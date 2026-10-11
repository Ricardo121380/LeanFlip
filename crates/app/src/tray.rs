//! 托盘：每个工具一个子菜单。切换式工具是单选（勾 = 当前卡），并存式工具是开关
//! （勾 = 写在客户端配置里）。菜单 id：`sw:<tool>:<id>` / `cx:<tool>:<id>`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use ccsl_core::Tool;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

use crate::service::Service;
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
            open: "Open SwitchLeaf",
            quit: "Quit",
            empty: "(no providers)",
        }
    } else {
        Labels {
            open: "打开 SwitchLeaf",
            quit: "退出",
            empty: "（没有供应商）",
        }
    }
}

/// 托盘菜单要显示的内容（纯数据）。读各工具配置在这里做，比较相等就不必重建菜单。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrayModel {
    pub english: bool,
    pub tools: Vec<ToolEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolEntry {
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub checked: bool,
}

/// 读 11 个工具的卡和 live 状态（有文件 IO，别在异步运行时 / 主线程上调）。
pub(crate) fn model(service: &Service) -> TrayModel {
    let english = service.app_settings().language.as_deref() == Some("en");
    let mut tools = Vec::new();
    for tool in Tool::ALL {
        let Ok(listing) = service.list_providers(tool) else {
            continue;
        };
        if listing.providers.is_empty() {
            continue;
        }
        let prefix = if tool.is_coexist() { "cx" } else { "sw" };
        let items = listing
            .providers
            .iter()
            .map(|provider| Item {
                id: format!("{prefix}:{}:{}", tool.as_str(), provider.id),
                label: provider.name.clone(),
                // 并存式工具的配置文件读不了时不让点（开关状态不可信）。
                enabled: listing.live_error.is_none(),
                checked: if tool.is_coexist() {
                    listing.in_live.contains(&provider.id)
                } else {
                    listing.current.as_deref() == Some(provider.id.as_str())
                },
            })
            .collect();
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
        tools.push(ToolEntry { title, items });
    }
    TrayModel { english, tools }
}

/// 按模型建菜单（只创建菜单项，不读文件）。
fn build_menu<R: Runtime>(app: &AppHandle<R>, model: &TrayModel) -> tauri::Result<Menu<R>> {
    let text = labels(model.english);
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(
        app,
        "open",
        text.open,
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    for entry in &model.tools {
        let mut items: Vec<Box<dyn IsMenuItem<R>>> = Vec::new();
        for item in &entry.items {
            items.push(Box::new(CheckMenuItem::with_id(
                app,
                &item.id,
                &item.label,
                item.enabled,
                item.checked,
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
        menu.append(&Submenu::with_items(app, &entry.title, true, &refs)?)?;
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

/// 合并重建请求：请求只置一个标志，后台线程每次醒来清掉标志再干活。干活期间来的
/// 请求只会让它再跑一轮，N 次连续请求最多两轮。
#[derive(Default)]
pub(crate) struct Coalescer {
    dirty: Mutex<bool>,
    wake: Condvar,
}

impl Coalescer {
    pub(crate) fn request(&self) {
        *self.dirty.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.wake.notify_one();
    }

    /// 等到有请求，消费掉它。
    pub(crate) fn wait(&self) {
        let mut dirty = self.dirty.lock().unwrap_or_else(|e| e.into_inner());
        while !*dirty {
            dirty = self.wake.wait(dirty).unwrap_or_else(|e| e.into_inner());
        }
        *dirty = false;
    }

    /// 起一个后台线程：每次有请求跑一次 `job`。
    pub(crate) fn spawn(self: &Arc<Self>, name: &str, mut job: impl FnMut() + Send + 'static) {
        let this = self.clone();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || loop {
                this.wait();
                job();
            })
            .expect("spawn tray thread");
    }
}

struct TrayRebuilder {
    coalescer: Arc<Coalescer>,
    force: Arc<AtomicBool>,
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
                force_next(&app);
                crate::notify_changed(&app);
            });
        }
    }
}

/// 先放一个只有「打开 / 退出」的菜单让窗口马上出来，真正的菜单在后台线程里建。
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let placeholder = TrayModel {
        english: app.state::<AppState>().0.app_settings().language.as_deref() == Some("en"),
        tools: Vec::new(),
    };
    let menu = build_menu(app, &placeholder)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("SwitchLeaf")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()));
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    let coalescer = Arc::new(Coalescer::default());
    let force = Arc::new(AtomicBool::new(false));
    let force_flag = force.clone();
    let handle = app.clone();
    let mut last = Some(placeholder);
    coalescer.spawn("tray-rebuild", move || {
        let service = handle.state::<AppState>().0.clone();
        let next = model(&service);
        // 增量：内容没变就不动菜单（文件监听的回声、切到同一张卡等）。
        let forced = force_flag.swap(false, Ordering::AcqRel);
        if !forced && last.as_ref() == Some(&next) {
            return;
        }
        let Some(tray) = handle.tray_by_id(TRAY_ID) else {
            return;
        };
        match build_menu(&handle, &next).and_then(|menu| tray.set_menu(Some(menu))) {
            Ok(()) => last = Some(next),
            Err(err) => log::warn!("tray menu: {err}"),
        }
    });
    coalescer.request();
    app.manage(TrayRebuilder { coalescer, force });
    Ok(())
}

/// 请求重建托盘菜单：不阻塞，读配置和建菜单都在后台线程里，连续请求会合并。
pub fn rebuild<R: Runtime>(app: &AppHandle<R>) {
    if let Some(rebuilder) = app.try_state::<TrayRebuilder>() {
        rebuilder.coalescer.request();
    }
}

/// 下一次重建即使内容没变也重设菜单：托盘里点过 CheckMenuItem 后它自己翻转了勾选，
/// 操作失败时模型不变，也得把勾改回来。
fn force_next<R: Runtime>(app: &AppHandle<R>) {
    if let Some(rebuilder) = app.try_state::<TrayRebuilder>() {
        rebuilder.force.store(true, Ordering::Release);
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

    #[test]
    fn coalescer_merges_bursts_into_at_most_two_runs() {
        use std::sync::atomic::AtomicUsize;
        use std::time::{Duration, Instant};

        let runs = Arc::new(AtomicUsize::new(0));
        let coalescer = Arc::new(Coalescer::default());
        let counter = runs.clone();
        coalescer.spawn("test-coalescer", move || {
            counter.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(60));
        });
        let wait_idle = |expected_at_least: usize| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while runs.load(Ordering::SeqCst) < expected_at_least && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            // 让可能的第二轮跑完。
            std::thread::sleep(Duration::from_millis(200));
        };

        for _ in 0..100 {
            coalescer.request();
        }
        wait_idle(1);
        let first = runs.load(Ordering::SeqCst);
        assert!((1..=2).contains(&first), "burst ran {first} times");

        // 空闲之后的新请求还会再跑。
        coalescer.request();
        wait_idle(first + 1);
        assert_eq!(runs.load(Ordering::SeqCst), first + 1);
    }

    #[test]
    fn model_reflects_cards_and_is_stable_without_changes() {
        use ccsl_core::{Env, Provider};
        use serde_json::json;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let service = Service::start(Env::with_home(&home), None).unwrap();

        let empty = model(&service);
        assert!(empty.tools.is_empty());
        assert!(!empty.english);

        let card = |id: &str| {
            Provider::new(
                id,
                id,
                json!({"env": {"ANTHROPIC_BASE_URL": "https://example.invalid", "ANTHROPIC_AUTH_TOKEN": "sk-x"}}),
            )
        };
        service
            .add_provider(Tool::Claude, card("a"), false)
            .unwrap();
        service
            .add_provider(Tool::Claude, card("b"), false)
            .unwrap();
        service.switch_provider(Tool::Claude, "b").unwrap();

        let first = model(&service);
        assert_eq!(first.tools.len(), 1);
        let claude = &first.tools[0];
        assert_eq!(claude.title, "Claude Code · b");
        let checked: Vec<_> = claude
            .items
            .iter()
            .filter(|i| i.checked)
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(checked, ["sw:claude:b"]);
        // 没有变化时模型相等，后台线程会跳过 set_menu。
        assert_eq!(model(&service), first);

        service.switch_provider(Tool::Claude, "a").unwrap();
        assert_ne!(model(&service), first);
    }

    #[test]
    fn coalescer_wait_consumes_one_pending_request() {
        let coalescer = Coalescer::default();
        coalescer.request();
        coalescer.request();
        coalescer.wait();
        assert!(!*coalescer.dirty.lock().unwrap());
    }
}
