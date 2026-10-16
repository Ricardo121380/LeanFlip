//! 托盘：每个工具一个子菜单。切换式工具是单选（勾 = 当前卡），并存式工具是开关
//! （勾 = 写在客户端配置里）。菜单 id：`sw:<tool>:<id>` / `cx:<tool>:<id>`。
//! 有场景时最上面多一个「场景」子菜单（勾 = 和现在完全一致），id 是 `sc:<scene-id>`；
//! 点了直接应用，结果通过事件交给窗口（有失败时把窗口叫出来）。提醒规则满足时，菜单
//! 最上面多一项「建议切换到 X」（`sh:<scene-id>`），点了同样直接应用。
//! 有额度数据的供应商后面带一小段额度；设置里打开后，状态栏图标旁显示最紧张的一项。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use leanflip_core::Tool;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

use crate::service::{rule_label, SceneReport, SceneStatus, Service};
use crate::AppState;

const TRAY_ID: &str = "main";
/// macOS 菜单栏用的单色模板图（icons/tray.svg 渲染而来），由系统按深浅色着色。
const TRAY_TEMPLATE: &[u8] = include_bytes!("../icons/tray.png");

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
    refresh: &'static str,
    scenes: &'static str,
    drifted: &'static str,
    suggest: &'static str,
}

fn labels(english: bool) -> Labels {
    if english {
        Labels {
            open: "Open LeanFlip",
            quit: "Quit",
            empty: "(no providers)",
            refresh: "Refresh usage",
            scenes: "Scenes",
            drifted: "drifted",
            suggest: "Switch to scene",
        }
    } else {
        Labels {
            open: "打开 LeanFlip",
            quit: "退出",
            empty: "（没有供应商）",
            refresh: "刷新额度",
            scenes: "场景",
            drifted: "已偏离",
            suggest: "建议切换到场景",
        }
    }
}

/// 名字后面带上额度：`拼车站（$8.75）`。
fn with_quota(name: &str, brief: Option<&String>, english: bool) -> String {
    match (brief, english) {
        (Some(brief), true) => format!("{name} ({brief})"),
        (Some(brief), false) => format!("{name}（{brief}）"),
        (None, _) => name.to_string(),
    }
}

/// 托盘菜单要显示的内容（纯数据）。读各工具配置在这里做，比较相等就不必重建菜单。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrayModel {
    pub english: bool,
    /// 场景子菜单；没有场景时为空。
    pub scenes: Option<ToolEntry>,
    /// 提醒规则满足时的「建议切换到 X」。
    pub scene_hint: Option<Item>,
    pub tools: Vec<ToolEntry>,
    /// 状态栏图标旁的文字；设置里没打开或没有数据时为空。
    pub title: Option<String>,
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
    let settings = service.app_settings();
    let english = settings.language.as_deref() == Some("en");
    let quota = service.quota_overview().ok();
    let mut briefs: HashMap<(Tool, String), String> = HashMap::new();
    for source in quota.iter().flat_map(|view| &view.sources) {
        let Some(brief) = source
            .snapshot
            .as_ref()
            .and_then(|snap| leanflip_service::quota::brief(snap, english))
        else {
            continue;
        };
        for user in &source.users {
            briefs.insert((user.tool, user.provider_id.clone()), brief.clone());
        }
    }
    let title = quota
        .as_ref()
        .filter(|_| settings.tray_quota_title)
        .and_then(|view| {
            let key = view.busiest.as_ref()?;
            let source = view.sources.iter().find(|s| &s.key == key)?;
            leanflip_service::quota::brief(source.snapshot.as_ref()?, english)
        });
    let (scenes, scene_hint) = scene_entry(service, english).unzip();
    let scene_hint = scene_hint.flatten();
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
                label: with_quota(
                    &provider.name,
                    briefs.get(&(tool, provider.id.clone())),
                    english,
                ),
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
                with_quota(
                    &format!("{} · {name}", tool_label(tool)),
                    briefs.get(&(tool, current.clone())),
                    english,
                )
            }
            (_, true) => format!("{} · {}", tool_label(tool), listing.in_live.len()),
            _ => tool_label(tool).to_string(),
        };
        tools.push(ToolEntry { title, items });
    }
    TrayModel {
        english,
        scenes,
        scene_hint,
        tools,
        title,
    }
}

/// 场景子菜单：标题带上当前场景（最近应用的那个，或任意一个完全一致的）。另外返回提醒
/// 规则建议切换到的场景（只读上一次检查的结果，不在这里检查规则）。
fn scene_entry(service: &Service, english: bool) -> Option<(ToolEntry, Option<Item>)> {
    let text = labels(english);
    let list = service.scenes().ok()?;
    if list.scenes.is_empty() {
        return None;
    }
    let drifted = |name: &str| {
        if english {
            format!("{name} ({})", text.drifted)
        } else {
            format!("{name}（{}）", text.drifted)
        }
    };
    let current = list
        .scenes
        .iter()
        .find(|view| {
            list.last.as_deref() == Some(view.scene.id.as_str()) && view.status != SceneStatus::Idle
        })
        .or_else(|| {
            list.scenes
                .iter()
                .find(|view| view.status == SceneStatus::Active)
        });
    let title = match current {
        Some(view) if view.status == SceneStatus::Drifted => {
            format!("{} · {}", text.scenes, drifted(&view.scene.name))
        }
        Some(view) => format!("{} · {}", text.scenes, view.scene.name),
        None => text.scenes.to_string(),
    };
    let items = list
        .scenes
        .iter()
        .map(|view| Item {
            id: format!("sc:{}", view.scene.id),
            label: if view.status == SceneStatus::Drifted {
                drifted(&view.scene.name)
            } else {
                view.scene.name.clone()
            },
            enabled: true,
            checked: view.status == SceneStatus::Active,
        })
        .collect();
    let hint = service
        .scene_hint()
        .filter(|hint| {
            list.scenes
                .iter()
                .any(|view| view.scene.id == hint.id && view.suggested)
        })
        .map(|hint| {
            let reason = rule_label(&hint.rule, english);
            Item {
                id: format!("sh:{}", hint.id),
                label: if english {
                    format!("{} {} ({reason})", text.suggest, hint.name)
                } else {
                    format!("{}「{}」（{reason}）", text.suggest, hint.name)
                },
                enabled: true,
                checked: false,
            }
        });
    Some((ToolEntry { title, items }, hint))
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
    if let Some(hint) = &model.scene_hint {
        menu.append(&MenuItem::with_id(
            app,
            &hint.id,
            &hint.label,
            hint.enabled,
            None::<&str>,
        )?)?;
    }
    if let Some(entry) = &model.scenes {
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
        let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(|b| b.as_ref()).collect();
        menu.append(&Submenu::with_items(app, &entry.title, true, &refs)?)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
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
        "quota-refresh",
        text.refresh,
        true,
        None::<&str>,
    )?)?;
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
        "quota-refresh" => {
            let app = app.clone();
            std::thread::spawn(move || {
                let service = app.state::<AppState>().0.clone();
                if let Err(err) = service.quota_refresh(None) {
                    log::warn!("tray quota refresh failed: {err}");
                }
                crate::notify_changed(&app);
            });
        }
        other if other.starts_with("sc:") || other.starts_with("sh:") => {
            let id = other[3..].to_string();
            let app = app.clone();
            std::thread::spawn(move || apply_scene(&app, &id));
        }
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

/// 托盘里应用场景的结果，交给窗口提示（成功可撤销，失败列出工具）。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SceneEvent {
    pub id: String,
    pub name: String,
    pub report: Option<SceneReport>,
    pub error: Option<String>,
}

fn apply_scene(app: &AppHandle, id: &str) {
    use tauri::Emitter;
    let service = app.state::<AppState>().0.clone();
    let name = service
        .scenes()
        .ok()
        .and_then(|list| list.scenes.into_iter().find(|view| view.scene.id == id))
        .map(|view| view.scene.name)
        .unwrap_or_else(|| id.to_string());
    let event = match service.apply_scene(id, None) {
        Ok(report) => SceneEvent {
            id: id.to_string(),
            name,
            report: Some(report),
            error: None,
        },
        Err(err) => {
            log::warn!("tray scene {id} failed: {err}");
            SceneEvent {
                id: id.to_string(),
                name,
                report: None,
                error: Some(err.0),
            }
        }
    };
    let failed = event.report.as_ref().is_none_or(SceneReport::failed);
    if failed {
        show_main(app);
    }
    let _ = app.emit(crate::EVENT_SCENE_APPLIED, event);
    // CheckMenuItem 点击后自己翻转了勾选；不管成败都按实际状态重建。
    force_next(app);
    crate::notify_changed(app);
}

/// 先放一个只有「打开 / 退出」的菜单让窗口马上出来，真正的菜单在后台线程里建。
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let placeholder = TrayModel {
        english: app.state::<AppState>().0.app_settings().language.as_deref() == Some("en"),
        scenes: None,
        scene_hint: None,
        tools: Vec::new(),
        title: None,
    };
    let menu = build_menu(app, &placeholder)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("LeanFlip")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()));
    if cfg!(target_os = "macos") {
        builder = builder
            .icon(tauri::image::Image::from_bytes(TRAY_TEMPLATE)?)
            .icon_as_template(true);
    } else if let Some(icon) = app.default_window_icon() {
        // 其他系统的托盘不会给模板图着色，黑色图形在深色面板上看不见，所以沿用 App 图标。
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
        // Windows 不支持图标旁的文字；Linux 只在部分面板上显示。
        if last.as_ref().map(|m| &m.title) != Some(&next.title) {
            if let Err(err) = tray.set_title(next.title.as_deref()) {
                log::warn!("tray title: {err}");
            }
        }
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
        use leanflip_core::{Env, Provider};
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
    fn model_shows_quota_next_to_providers_and_a_title_only_when_enabled() {
        use crate::service::SettingsInput;
        use leanflip_core::{Env, Provider};
        use leanflip_service::quota::{QuotaBalance, QuotaLevel, QuotaOrigin, QuotaSnapshot};
        use serde_json::json;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let env = Env::with_home(&home);
        let service = Service::start(env.clone(), None).unwrap();
        let mut card = Provider::new(
            "corp",
            "Corp",
            json!({"env": {"ANTHROPIC_BASE_URL": "https://relay.example.invalid", "ANTHROPIC_AUTH_TOKEN": "sk-x"}}),
        );
        card.meta.extra.insert(
            "leanflipQuota".into(),
            json!({"adapter": "newapi", "refreshMinutes": 0}),
        );
        service.add_provider(Tool::Claude, card, false).unwrap();
        service.switch_provider(Tool::Claude, "corp").unwrap();
        let key = service.quota_overview().unwrap().sources[0].key.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let snapshot = QuotaSnapshot {
            windows: Vec::new(),
            credits: None,
            balance: Some(QuotaBalance {
                unit: "USD".into(),
                remaining: Some(31.4),
                limit: Some(58.0),
                used: Some(26.6),
                unlimited: false,
                expires_at: None,
                account_remaining: None,
            }),
            plan: None,
            fetched_at: now,
            origin: QuotaOrigin::Network,
            level: QuotaLevel::Ok,
            used_fraction: Some(0.46),
            limit_reached: false,
        };
        std::fs::write(
            env.data_file("quota-cache.json"),
            json!({"version": 1, "entries": [], "relays": [{"key": key, "snapshot": snapshot}]})
                .to_string(),
        )
        .unwrap();

        let off = model(&service);
        assert_eq!(off.tools[0].title, "Claude Code · Corp（$31.40）");
        assert_eq!(off.tools[0].items[0].label, "Corp（$31.40）");
        assert_eq!(off.title, None);

        let mut app = service.app_settings();
        app.tray_quota_title = true;
        service
            .save_settings(SettingsInput {
                app,
                keep_codex_login: false,
            })
            .unwrap();
        assert_eq!(model(&service).title.as_deref(), Some("$31.40"));
    }

    #[test]
    fn model_lists_scenes_with_the_current_one_checked_and_drift_marked() {
        use leanflip_core::scene::{Scene, SceneTool};
        use leanflip_core::{Env, Provider};
        use serde_json::json;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let service = Service::start(Env::with_home(&home), None).unwrap();
        for id in ["a", "b"] {
            service
                .add_provider(
                    Tool::Claude,
                    Provider::new(
                        id,
                        id,
                        json!({"env": {"ANTHROPIC_BASE_URL": "https://example.invalid", "ANTHROPIC_AUTH_TOKEN": "sk-x"}}),
                    ),
                    false,
                )
                .unwrap();
        }
        service.switch_provider(Tool::Claude, "a").unwrap();
        assert_eq!(model(&service).scenes, None);

        let mut ids = Vec::new();
        for (name, card) in [("Work", "a"), ("Home", "b")] {
            let mut scene = Scene {
                name: name.into(),
                ..Scene::default()
            };
            scene
                .tools
                .insert("claude".into(), SceneTool::current(card));
            ids.push(service.save_scene(scene).unwrap().id);
        }
        let scenes = model(&service).scenes.unwrap();
        assert_eq!(scenes.title, "场景 · Work");
        assert_eq!(
            scenes
                .items
                .iter()
                .map(|i| (i.id.as_str(), i.label.as_str(), i.checked))
                .collect::<Vec<_>>(),
            [
                (format!("sc:{}", ids[0]).as_str(), "Work", true),
                (format!("sc:{}", ids[1]).as_str(), "Home", false)
            ]
        );

        service.apply_scene(&ids[1], None).unwrap();
        service.switch_provider(Tool::Claude, "a").unwrap();
        let scenes = model(&service).scenes.unwrap();
        assert_eq!(scenes.title, "场景 · Home（已偏离）");
        assert_eq!(scenes.items[1].label, "Home（已偏离）");
        assert!(scenes.items[0].checked);
    }

    #[test]
    fn model_puts_a_rule_suggestion_at_the_top_until_it_is_applied() {
        use leanflip_core::scene::{Scene, SceneTool};
        use leanflip_core::{Env, Provider};
        use serde_json::json;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let service = Service::start(Env::with_home(&home), None).unwrap();
        for id in ["a", "b"] {
            service
                .add_provider(
                    Tool::Claude,
                    Provider::new(
                        id,
                        id,
                        json!({"env": {"ANTHROPIC_BASE_URL": "https://example.invalid", "ANTHROPIC_AUTH_TOKEN": "sk-x"}}),
                    ),
                    false,
                )
                .unwrap();
        }
        service.switch_provider(Tool::Claude, "a").unwrap();
        // 两条合起来覆盖全天，结果不依赖测试运行的时刻。
        let rules = [("00:00", "12:00"), ("12:00", "00:00")]
            .into_iter()
            .map(|(start, end)| {
                serde_json::from_value(json!({"kind": "time", "days": [1, 2, 3, 4, 5, 6, 7], "start": start, "end": end}))
                    .unwrap()
            })
            .collect();
        let mut office = Scene {
            name: "Office".into(),
            rules,
            ..Scene::default()
        };
        office
            .tools
            .insert("claude".into(), SceneTool::current("b"));
        let office = service.save_scene(office).unwrap();
        assert_eq!(
            model(&service).scene_hint,
            None,
            "rules are checked in the background"
        );

        assert!(service.refresh_scene_hint().unwrap().notify.is_some());
        let hint = model(&service).scene_hint.unwrap();
        assert_eq!(hint.id, format!("sh:{}", office.id));
        // 标签写的是现在命中的那条规则，上午和下午不同。
        assert!(
            [
                "建议切换到场景「Office」（每天 00:00–12:00）",
                "建议切换到场景「Office」（每天 12:00–次日 00:00）"
            ]
            .contains(&hint.label.as_str()),
            "{}",
            hint.label
        );

        service.apply_scene(&office.id, None).unwrap();
        assert_eq!(model(&service).scene_hint, None);
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
