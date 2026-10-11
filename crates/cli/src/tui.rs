//! Full-screen presentation only. Configuration reads and writes retain the
//! service layer's paths, locking, recovery, and redacted error contract.
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use ccsl_core::{Provider, Tool};
use ccsl_service::service::{ToolProviders, ToolSummary};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use serde_json::{json, Value};

use crate::args::{Cli, Command, Import, Language};
use crate::menu::{can_interact, name, terminal_error};
use crate::presentation::{clean, error};
use crate::{open_service, run, CliError};

mod data;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Tools,
    Providers,
}

enum Action {
    Switch(String),
    Enable(String),
    Disable(String),
    Import,
}

struct Pending {
    tool: Tool,
    action: Action,
    // Only compared in memory; configuration values are never rendered.
    basis: Value,
}

struct Dialog {
    title: String,
    body: String,
    scroll: u16,
    max_scroll: u16,
    pending: Option<Pending>,
    accept: bool,
}

impl Dialog {
    fn info(title: &str, body: String) -> Self {
        Self {
            title: title.into(),
            body,
            scroll: 0,
            max_scroll: 0,
            pending: None,
            accept: false,
        }
    }
}

struct App {
    home: Option<PathBuf>,
    lang: Language,
    tool: usize,
    overview: Vec<ToolSummary>,
    list: Option<ToolProviders>,
    read_error: Option<String>,
    tools_state: ListState,
    providers_state: ListState,
    focus: Focus,
    query: String,
    searching: bool,
    notice: String,
    dialog: Option<Dialog>,
    quit: bool,
    data: Option<data::DataPanel>,
}

impl App {
    fn new(home: Option<&Path>, lang: Language, tool: Option<Tool>) -> Self {
        let mut app = Self {
            home: home.map(Path::to_path_buf),
            lang,
            tool: tool
                .and_then(|tool| Tool::ALL.iter().position(|item| *item == tool))
                .unwrap_or(0),
            overview: vec![],
            list: None,
            read_error: None,
            tools_state: ListState::default(),
            providers_state: ListState::default(),
            focus: if tool.is_some() {
                Focus::Providers
            } else {
                Focus::Tools
            },
            query: String::new(),
            searching: false,
            notice: lang
                .text(
                    "浏览不会修改配置。按 ? 查看快捷键。",
                    "Browsing is read-only. Press ? for shortcuts.",
                )
                .into(),
            dialog: None,
            quit: false,
            data: None,
        };
        app.refresh();
        app
    }

    fn tool(&self) -> Tool {
        Tool::ALL[self.tool]
    }

    fn visible(&self) -> Vec<&Provider> {
        let query = self.query.to_lowercase();
        self.list
            .iter()
            .flat_map(|list| &list.providers)
            .filter(|provider| {
                clean(&provider.name).to_lowercase().contains(&query)
                    || clean(&provider.id).to_lowercase().contains(&query)
            })
            .collect()
    }

    fn selected(&self) -> Option<&Provider> {
        self.providers_state
            .selected()
            .and_then(|index| self.visible().get(index).copied())
    }

    fn refresh(&mut self) -> bool {
        let selected = self.selected().map(|provider| provider.id.clone());
        let result = open_service(self.home.as_deref())
            .and_then(|service| Ok((service.overview()?, service.list_providers(self.tool())?)));
        match result {
            Ok((overview, list)) => {
                self.overview = overview;
                self.list = Some(list);
                self.read_error = None;
                let visible = self.visible();
                let index = selected
                    .and_then(|id| visible.iter().position(|provider| provider.id == id))
                    .or_else(|| (!visible.is_empty()).then_some(0));
                self.providers_state.select(index);
                true
            }
            Err(problem) => {
                self.overview.clear();
                self.list = None;
                self.providers_state.select(None);
                self.read_error = Some(error(&problem.code, problem.message, self.lang));
                false
            }
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.focus == Focus::Tools {
            self.tool = (self.tool as isize + delta).rem_euclid(Tool::ALL.len() as isize) as usize;
            self.query.clear();
            self.providers_state = ListState::default();
            self.refresh();
        } else {
            let count = self.visible().len();
            self.providers_state.select((count > 0).then(|| {
                (self.providers_state.selected().unwrap_or(0) as isize + delta)
                    .rem_euclid(count as isize) as usize
            }));
        }
    }

    fn problem(&mut self, problem: CliError) {
        self.notice = self
            .lang
            .text(
                "操作未完成；请检查提示后重试。",
                "Operation failed; review the message and retry.",
            )
            .into();
        self.dialog = Some(Dialog::info(
            self.lang.text("操作未完成", "Operation failed"),
            error(&problem.code, problem.message, self.lang),
        ));
    }

    fn prepare(&mut self, import: bool) {
        let selected = self.selected().map(|provider| provider.id.clone());
        if !self.refresh() {
            return;
        }
        let list = self.list.as_ref().expect("successful refresh has a list");
        let (action, target, verb) = if import {
            (
                Action::Import,
                name(list.tool).into(),
                self.lang.text("从客户端导入", "Import from client"),
            )
        } else {
            let Some(provider) = self
                .selected()
                .filter(|provider| Some(&provider.id) == selected.as_ref())
            else {
                self.notice = self
                    .lang
                    .text(
                        "请先选择供应商；列表变化后需要重新选择。",
                        "Select a provider first; reselect if the list changed.",
                    )
                    .into();
                return;
            };
            let target = format!("{}\nID: {}", clean(&provider.name), clean(&provider.id));
            if list.tool.is_coexist() {
                if list.live_error.is_some() {
                    self.notice = self.lang.text("配置无法读取，未修改任何配置。按 p 查看路径，修复后按 r 刷新。", "Configuration unreadable; nothing changed. Press p for paths, repair it, then r to refresh.").into();
                    return;
                }
                if list.in_live.contains(&provider.id) {
                    (
                        Action::Disable(provider.id.clone()),
                        target,
                        self.lang.text("停用供应商", "Disable provider"),
                    )
                } else {
                    (
                        Action::Enable(provider.id.clone()),
                        target,
                        self.lang.text("启用供应商", "Enable provider"),
                    )
                }
            } else if list.current.as_deref() == Some(&provider.id) {
                self.notice = self
                    .lang
                    .text(
                        "已经是当前供应商，无需切换。",
                        "This provider is already selected.",
                    )
                    .into();
                return;
            } else {
                (
                    Action::Switch(provider.id.clone()),
                    target,
                    self.lang.text("切换供应商", "Switch provider"),
                )
            }
        };
        let explanation = if import {
            self.lang.text(
                "只保存卡片，不切换客户端配置。\n读取路径：",
                "Save cards only; keep the client selection.\nRead from:",
            )
        } else {
            self.lang
                .text("将更新以下客户端配置：", "Client configuration to update:")
        };
        let paths = list
            .paths
            .iter()
            .map(|path| clean(&path.display().to_string()))
            .collect::<Vec<_>>()
            .join("\n");
        let mut dialog = Dialog::info(
            self.lang.text("确认操作", "Confirm action"),
            format!(
                "{verb} · {}\n\n{target}\n\n{explanation}\n{paths}",
                name(list.tool)
            ),
        );
        dialog.pending = Some(Pending {
            tool: list.tool,
            action,
            basis: json!(list),
        });
        self.dialog = Some(dialog);
    }

    fn apply(&mut self, pending: Pending) {
        let result = (|| {
            let latest = open_service(self.home.as_deref())?.list_providers(pending.tool)?;
            if json!(latest) != pending.basis {
                return Err(CliError { code: "conflict".into(), message: "Configuration changed while confirming. Refresh and confirm the action again.", exit: 1 });
            }
            let tool = pending.tool.into();
            let command = match pending.action {
                Action::Switch(provider_id) => Command::Use { tool, provider_id },
                Action::Enable(provider_id) => Command::Enable { tool, provider_id },
                Action::Disable(provider_id) => Command::Disable { tool, provider_id },
                Action::Import => Command::Import {
                    source: Import::Live { tool },
                },
            };
            run(Cli {
                json: false,
                home: self.home.clone(),
                lang: self.lang,
                command,
            })
        })();
        self.refresh();
        match result {
            Ok(output) => {
                self.notice = output.text.lines().next().unwrap_or_default().to_owned();
                if output.data["warningCount"].as_u64().unwrap_or(0) > 0 {
                    self.dialog = Some(Dialog::info(
                        self.lang.text("操作结果与警告", "Result and warnings"),
                        output.text,
                    ));
                }
            }
            Err(problem) => self.problem(problem),
        }
    }

    fn details(&self) -> String {
        let Some(list) = &self.list else {
            return self.read_error.clone().unwrap_or_default();
        };
        let mut lines = vec![
            name(list.tool).to_owned(),
            self.lang.text("模式：", "Mode: ").to_owned()
                + if list.tool.is_coexist() {
                    self.lang
                        .text("可同时启用多个供应商", "Multiple providers can be enabled")
                } else {
                    self.lang.text("切换当前供应商", "One selected provider")
                },
        ];
        if let Some(provider) = self.selected() {
            lines.extend([
                String::new(),
                clean(&provider.name),
                format!("ID: {}", clean(&provider.id)),
                state(list, provider, self.lang).into(),
            ]);
        }
        lines.push(String::new());
        lines.push(self.lang.text("配置路径", "Configuration paths").into());
        lines.extend(
            list.paths
                .iter()
                .map(|path| clean(&path.display().to_string())),
        );
        lines.push(String::new());
        lines.push(if list.tool.is_coexist() {
            self.lang.text("状态来自客户端配置。", "State is read from the client configuration.")
        } else {
            self.lang.text("当前选择来自卡片库；不代表客户端文件未经外部修改。", "Selection is stored in the card library; client files may have changed externally.")
        }.into());
        lines.join("\n")
    }

    fn key(&mut self, key: KeyEvent) -> Option<Pending> {
        if let Some(data) = &mut self.data {
            if data.key(key) {
                self.data = None;
                self.refresh();
            }
            return None;
        }
        if let Some(dialog) = &mut self.dialog {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q' | 'n' | 'N') => self.dialog = None,
                KeyCode::Up | KeyCode::Char('k') => dialog.scroll = dialog.scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    dialog.scroll = dialog.scroll.saturating_add(1).min(dialog.max_scroll)
                }
                KeyCode::PageDown => {
                    dialog.scroll = dialog.scroll.saturating_add(8).min(dialog.max_scroll)
                }
                KeyCode::PageUp => dialog.scroll = dialog.scroll.saturating_sub(8),
                KeyCode::Home => dialog.scroll = 0,
                KeyCode::End => dialog.scroll = dialog.max_scroll,
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    dialog.accept = !dialog.accept
                }
                KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                    let accept = dialog.accept || matches!(key.code, KeyCode::Char('y' | 'Y'));
                    let dialog = self.dialog.take().expect("dialog is open");
                    if accept {
                        return dialog.pending;
                    }
                }
                _ => {}
            }
            return None;
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.query.clear();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(ch)
                    if !ch.is_control()
                        && !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && self.query.chars().count() < 128 =>
                {
                    self.query.push(ch);
                }
                _ => {}
            }
            self.providers_state
                .select((!self.visible().is_empty()).then_some(0));
            return None;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc if !self.query.is_empty() => { self.query.clear(); self.providers_state.select((!self.visible().is_empty()).then_some(0)); }
            KeyCode::Esc if self.focus == Focus::Providers => self.focus = Focus::Tools,
            KeyCode::Esc => self.quit = true,
            KeyCode::Tab | KeyCode::BackTab => self.focus = if self.focus == Focus::Tools { Focus::Providers } else { Focus::Tools },
            KeyCode::Left | KeyCode::Char('h') => self.focus = Focus::Tools,
            KeyCode::Right | KeyCode::Char('l') => self.focus = Focus::Providers,
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Enter if self.focus == Focus::Tools => self.focus = Focus::Providers,
            KeyCode::Enter => self.prepare(false),
            KeyCode::Char('/') => { self.focus = Focus::Providers; self.searching = true; }
            KeyCode::Char('i') => self.prepare(true),
            KeyCode::Char('b') => match data::DataPanel::new(self.home.as_deref(), self.lang) { Ok(panel) => self.data = Some(panel), Err(problem) => self.problem(problem) },
            KeyCode::Char('r') => {
                self.notice = if self.refresh() {
                    self.lang.text("已刷新当前状态。", "Current state refreshed.")
                } else { self.lang.text("读取失败；修复配置后按 r 重试。", "Read failed; repair the configuration and press r to retry.") }.into();
            }
            KeyCode::Char('p' | 'd') => self.dialog = Some(Dialog::info(self.lang.text("详情与路径", "Details and paths"), self.details())),
            KeyCode::Char('?') | KeyCode::F(1) => self.dialog = Some(Dialog::info(self.lang.text("快捷键", "Keyboard shortcuts"), self.lang.text(
                "Tab / ← →   切换工具与供应商区域\n↑ ↓ / j k   移动选择\nEnter       选择工具 / 预览操作\n/           搜索供应商名称或 ID\nEsc         清除搜索 / 返回 / 取消\nb           数据中心：迁移、导入、加密备份\ni           从当前客户端导入卡片\nr           重新读取最新状态\np / d       查看完整详情与路径\n? / F1      打开帮助\nq           关闭面板或退出\nCtrl+C      退出并恢复终端\n\n确认弹窗默认取消；Tab 选择确认后按 Enter，或按 y 执行。\n长内容使用 ↑↓ / PgUp / PgDn 滚动。\n\n浏览、搜索和刷新不会写配置。卡片编辑和账号登录可使用 switchleaf --help 中的命令。",
                "Tab / Left Right  Change focused pane\nUp Down / j k     Move selection\nEnter             Select tool / preview action\n/                 Search provider name or ID\nEsc               Clear filter / back / cancel\nb                 Data center: migrate, import, backup\ni                 Import current client settings\nr                 Reload current state\np / d             Show full details and paths\n? / F1            Show help\nq                 Close a panel or quit\nCtrl+C            Exit and restore the terminal\n\nConfirmation defaults to Cancel. Tab then Enter, or y, applies it.\nScroll long panels with Up/Down or PgUp/PgDn.\n\nBrowsing, search and refresh never write configuration. Use switchleaf --help for card editing and account login."
            ).into())),
            _ => {}
        }
        None
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::default().style(surface()), area);
        if area.width < 44 || area.height < 12 {
            frame.render_widget(
                Paragraph::new(self.lang.text(
                    "窗口太小：请扩大到至少 44×12。\nq 退出 · Ctrl+C 中断",
                    "Terminal too small: resize to at least 44x12.\nq quit · Ctrl+C exit",
                ))
                .wrap(Wrap { trim: false }),
                area,
            );
            return;
        }
        if let Some(data) = &mut self.data {
            data.render(frame);
            return;
        }
        let [header, body, notice, footer] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("  SwitchLeaf", accent()),
                    Span::styled("  /  WORKSPACE", muted()),
                    Span::raw(format!("  {}", env!("CARGO_PKG_VERSION"))),
                ]),
                Line::from(format!(
                    " {} · {}",
                    name(self.tool()),
                    if self.home.is_some() {
                        self.lang.text("独立配置目录", "Custom configuration home")
                    } else {
                        self.lang
                            .text("与 App 共用配置", "Shared with the desktop App")
                    }
                )),
            ]),
            header,
        );
        if area.width >= 110 {
            let [tools, providers, details] = Layout::horizontal([
                Constraint::Length(24),
                Constraint::Percentage(38),
                Constraint::Min(24),
            ])
            .areas(body);
            self.render_tools(frame, tools);
            self.render_providers(frame, providers);
            frame.render_widget(
                Paragraph::new(self.details())
                    .block(panel(
                        self.lang.text("详情 · p 展开", "Details · p expand"),
                        false,
                    ))
                    .wrap(Wrap { trim: false }),
                details,
            );
        } else if area.width >= 76 {
            let [tools, right] =
                Layout::horizontal([Constraint::Length(22), Constraint::Min(20)]).areas(body);
            self.render_tools(frame, tools);
            self.render_providers(frame, right);
        } else {
            match self.focus {
                Focus::Tools => self.render_tools(frame, body),
                Focus::Providers => self.render_providers(frame, body),
            }
        }
        frame.render_widget(
            Paragraph::new(self.notice.as_str()).wrap(Wrap { trim: false }),
            notice,
        );
        let hints = if self.searching {
            self.lang.text(
                "搜索中 · 输入文字   Enter 完成   Esc 清除",
                "Searching · type to filter   Enter done   Esc clear",
            )
        } else if area.width < 76 {
            self.lang.text(
                "Tab 切换区域  ↑↓ 选择  Enter 打开\n/ 搜索  b 数据中心  r 刷新  ? 帮助  q 退出",
                "Tab pane  Up/Down select  Enter open\n/ search  b data  r reload  ? help  q quit",
            )
        } else {
            self.lang.text("Tab/←→ 切换区域   ↑↓/jk 选择   Enter 预览操作   / 搜索\nb 数据中心   i 从客户端导入   r 刷新   p 详情与路径   ? 帮助   q 退出", "Tab/Left/Right pane   Up/Down/jk select   Enter preview   / search\nb data center   i from client   r reload   p details/paths   ? help   q quit")
        };
        frame.render_widget(Paragraph::new(hints).style(accent()), footer);
        if let Some(dialog) = &mut self.dialog {
            let confirmation = dialog.pending.is_some();
            render_dialog(frame, dialog, self.lang, confirmation);
        }
    }

    fn render_tools(&mut self, frame: &mut Frame, area: Rect) {
        let rows = Tool::ALL
            .iter()
            .map(|tool| {
                let summary = self.overview.iter().find(|summary| summary.tool == *tool);
                let suffix = summary
                    .map(|summary| format!("  {}", summary.card_count))
                    .unwrap_or_default();
                ListItem::new(format!("{}{suffix}", name(*tool)))
            })
            .collect::<Vec<_>>();
        self.tools_state.select(Some(self.tool));
        frame.render_stateful_widget(
            List::new(rows)
                .block(panel(
                    self.lang.text("工具 · 卡片数", "Tools · cards"),
                    self.focus == Focus::Tools,
                ))
                .highlight_symbol("> ")
                .highlight_style(selected_style(self.focus == Focus::Tools)),
            area,
            &mut self.tools_state,
        );
    }

    fn render_providers(&mut self, frame: &mut Frame, area: Rect) {
        let visible = self.visible();
        let title = format!(
            "{} · {}",
            self.lang.text("供应商", "Providers"),
            visible.len()
        );
        let block = panel(&title, self.focus == Focus::Providers);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [search, rows] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(inner);
        let search_text = if self.query.is_empty() && !self.searching {
            self.lang
                .text("/ 按名称或 ID 搜索", "/ search name or ID")
                .into()
        } else {
            format!(
                "/ {}{}",
                clean(&self.query),
                if self.searching { "_" } else { "" }
            )
        };
        let mut search_lines = vec![Line::from(search_text)];
        if self
            .list
            .as_ref()
            .is_some_and(|list| list.live_error.is_some())
        {
            search_lines.push(Line::from(self.lang.text(
                "配置无法读取 · p 查看路径 · r 重试",
                "Unreadable config · p paths · r retry",
            )));
        }
        frame.render_widget(
            Paragraph::new(search_lines).style(if self.searching {
                accent()
            } else {
                Style::default()
            }),
            search,
        );
        if let Some(problem) = &self.read_error {
            frame.render_widget(
                Paragraph::new(format!(
                    "{problem}\n\n{}",
                    self.lang
                        .text("r 重试读取 · q 退出", "r retry reading · q quit")
                ))
                .wrap(Wrap { trim: false }),
                rows,
            );
            return;
        }
        if visible.is_empty() {
            let empty = if self.query.is_empty() {
                self.lang.text("还没有供应商\n\n按 b 迁入本机 CC Switch 或备份。\n按 i 导入当前客户端配置。\n也可使用 switchleaf provider add 添加卡片。", "No providers yet\n\nPress b to migrate CC Switch or a backup.\nPress i to import current client settings,\nor use switchleaf provider add to create a card.")
            } else {
                self.lang.text(
                    "没有匹配结果\n\n按 / 修改搜索，或 Esc 清除筛选。",
                    "No matches\n\nPress / to edit the search or Esc to clear it.",
                )
            };
            frame.render_widget(Paragraph::new(empty).wrap(Wrap { trim: false }), rows);
            return;
        }
        let list = self.list.as_ref().expect("visible providers have a list");
        let items = visible
            .iter()
            .map(|provider| {
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(
                            clean(&provider.name),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("  [{}]", state(list, provider, self.lang)),
                            if list.current.as_deref() == Some(&provider.id)
                                || list.in_live.contains(&provider.id)
                            {
                                accent()
                            } else {
                                muted()
                            },
                        ),
                    ]),
                    Line::from(Span::styled(clean(&provider.id), muted())),
                    Line::default(),
                ])
            })
            .collect::<Vec<_>>();
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol("> ")
                .highlight_style(selected_style(self.focus == Focus::Providers)),
            rows,
            &mut self.providers_state,
        );
    }
}

fn state(list: &ToolProviders, provider: &Provider, lang: Language) -> &'static str {
    if list.tool.is_coexist() {
        if list.live_error.is_some() {
            lang.text("状态未知", "Unknown")
        } else if list.in_live.contains(&provider.id) {
            lang.text("已启用", "Enabled")
        } else {
            lang.text("未启用", "Disabled")
        }
    } else if list.current.as_deref() == Some(&provider.id) {
        lang.text("当前使用", "Current")
    } else {
        lang.text("可切换", "Available")
    }
}

fn accent() -> Style {
    let style = Style::default().add_modifier(Modifier::BOLD);
    if std::env::var_os("NO_COLOR").is_some() {
        style
    } else {
        style.fg(Color::Rgb(134, 218, 171))
    }
}

fn surface() -> Style {
    if std::env::var_os("NO_COLOR").is_some() {
        Style::default()
    } else {
        Style::default()
            .bg(Color::Rgb(18, 27, 23))
            .fg(Color::Rgb(222, 232, 224))
    }
}

fn muted() -> Style {
    if std::env::var_os("NO_COLOR").is_some() {
        Style::default()
    } else {
        Style::default().fg(Color::Rgb(153, 176, 161))
    }
}

fn selected_style(focused: bool) -> Style {
    if focused {
        if std::env::var_os("NO_COLOR").is_some() {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .bg(Color::Rgb(38, 75, 54))
                .fg(Color::Rgb(235, 250, 237))
                .add_modifier(Modifier::BOLD)
        }
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    }
}

fn panel(title: &str, focused: bool) -> Block<'static> {
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .padding(ratatui::widgets::Padding::horizontal(1))
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(surface())
        .border_style(if focused { accent() } else { muted() })
}

fn render_dialog(frame: &mut Frame, dialog: &mut Dialog, lang: Language, confirmation: bool) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(90);
    let height = screen.height.saturating_sub(2).min(28);
    let area = Rect::new(
        (screen.width - width) / 2,
        (screen.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let block = panel(&dialog.title, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [body, hints, buttons] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let paragraph = Paragraph::new(dialog.body.as_str()).wrap(Wrap { trim: false });
    dialog.max_scroll = paragraph
        .line_count(body.width)
        .saturating_sub(body.height as usize)
        .min(u16::MAX as usize) as u16;
    dialog.scroll = dialog.scroll.min(dialog.max_scroll);
    frame.render_widget(paragraph.scroll((dialog.scroll, 0)), body);
    if dialog.max_scroll > 0 {
        frame.render_widget(
            Paragraph::new(lang.text("↑↓ / PgUp PgDn 滚动查看", "Up/Down / PgUp PgDn to scroll")),
            hints,
        );
    }
    let buttons_text = if confirmation {
        Line::from(vec![
            Span::styled(
                lang.text(" [取消 Esc] ", " [Cancel Esc] "),
                selected_style(!dialog.accept),
            ),
            Span::raw("   "),
            Span::styled(
                lang.text(" [确认 Y] ", " [Apply Y] "),
                selected_style(dialog.accept),
            ),
            Span::raw(lang.text("  Tab 选择 · Enter 执行", "  Tab select · Enter choose")),
        ])
    } else {
        Line::from(lang.text("Esc / Enter / q 关闭", "Esc / Enter / q close"))
    };
    frame.render_widget(Paragraph::new(buttons_text), buttons);
}

struct RestoreTerminal;
impl Drop for RestoreTerminal {
    fn drop(&mut self) {
        ratatui::restore();
        let _ = ratatui::crossterm::execute!(std::io::stdout(), ratatui::crossterm::cursor::Show);
    }
}

pub fn start(home: Option<&Path>, lang: Language, tool: Option<Tool>) -> Result<(), CliError> {
    if !can_interact() {
        return Err(CliError::input("interactive_terminal", "The TUI requires an interactive terminal. Run ccsl --help for non-interactive commands."));
    }
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal = interrupted.clone();
    ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed))
        .map_err(|_| terminal_error())?;
    let _restore = RestoreTerminal;
    let mut terminal = ratatui::try_init().map_err(|_| terminal_error())?;
    let mut app = App::new(home, lang, tool);
    while !app.quit {
        if interrupted.load(Ordering::Relaxed) {
            return Err(CliError {
                code: "interrupted".into(),
                message: "Interrupted.",
                exit: 130,
            });
        }
        terminal
            .draw(|frame| app.render(frame))
            .map_err(|_| terminal_error())?;
        let changed = app.data.as_mut().is_some_and(data::DataPanel::run_pending);
        if changed {
            app.refresh();
        }
        if !event::poll(Duration::from_millis(100)).map_err(|_| terminal_error())? {
            continue;
        }
        if let Event::Key(key) = event::read().map_err(|_| terminal_error())? {
            if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                continue;
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                interrupted.store(true, Ordering::Relaxed);
                continue;
            }
            let size = terminal.size().map_err(|_| terminal_error())?;
            if size.width < 44 || size.height < 12 {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                    app.quit = true;
                }
                continue;
            }
            if let Some(pending) = app.key(key) {
                app.notice = lang
                    .text("正在应用，请稍候…", "Applying changes, please wait…")
                    .into();
                terminal
                    .draw(|frame| app.render(frame))
                    .map_err(|_| terminal_error())?;
                app.apply(pending);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
