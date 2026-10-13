//! Interactive presentation. Reads are fresh snapshots; writes reuse the same
//! command dispatcher, locking and redaction as the non-interactive CLI.
use std::io::{self, IsTerminal};
use std::path::Path;

use dialoguer::console::{style, truncate_str, Style, Term};
use dialoguer::{theme::ColorfulTheme, FuzzySelect};
use leanflip_core::Tool;
use leanflip_service::service::{ToolProviders, ToolSummary};

use crate::args::{Cli, Command, Import, Language};
use crate::presentation::{clean, error};
use crate::{open_service, run, CliError};

pub(super) fn theme() -> ColorfulTheme {
    ColorfulTheme {
        active_item_style: Style::new().for_stderr().color256(104).bold(),
        defaults_style: Style::new().for_stderr().color256(104),
        prompt_prefix: style("◇".to_owned()).for_stderr().color256(104),
        fuzzy_match_highlight_style: Style::new().for_stderr().color256(104).underlined(),
        ..ColorfulTheme::default()
    }
}

pub(super) fn terminal_error() -> CliError {
    CliError { code: "terminal_io".into(), message: "Cannot use the interactive terminal. Reopen your terminal or use the non-interactive commands.", exit: 1 }
}

pub fn can_interact() -> bool {
    io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && io::stderr().is_terminal()
        && std::env::var("TERM").as_deref() != Ok("dumb")
}

pub(super) fn name(tool: Tool) -> &'static str {
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

fn tool_label(summary: &ToolSummary, lang: Language) -> String {
    let state = if summary.tool.is_coexist() {
        if summary.live_readable {
            format!(
                "{} {}",
                lang.text("已启用", "Enabled"),
                summary.in_live.len()
            )
        } else {
            lang.text("状态未知", "State unknown").into()
        }
    } else {
        summary
            .current_name
            .clone()
            .unwrap_or_else(|| lang.text("尚未选择", "Not selected").into())
    };
    format!(
        "{} · {} {} · {}",
        name(summary.tool),
        summary.card_count,
        lang.text("个供应商", "providers"),
        clean(&state)
    )
}

#[derive(Debug, PartialEq, Eq)]
enum ProviderAction {
    Switch(String),
    SetEnabled(String, bool),
    Current,
    Unknown,
    Import,
    Edit,
    Paths,
    Back,
}

struct Item {
    label: String,
    action: ProviderAction,
}

fn provider_items(list: &ToolProviders, lang: Language) -> Vec<Item> {
    let mut items = list
        .providers
        .iter()
        .map(|provider| {
            // ● 生效 ○ 可用 ? 未知，与 TUI 一致；状态放在末尾，窄屏截断时仍保留圆点。
            let (glyph, state, action) = if list.tool.is_coexist() {
                if list.live_error.is_some() {
                    (
                        "?",
                        lang.text("状态未知", "Unknown"),
                        ProviderAction::Unknown,
                    )
                } else if list.in_live.contains(&provider.id) {
                    (
                        "●",
                        lang.text("已启用 · 回车停用", "Enabled · Enter to disable"),
                        ProviderAction::SetEnabled(provider.id.clone(), false),
                    )
                } else {
                    (
                        "○",
                        lang.text("未启用 · 回车启用", "Disabled · Enter to enable"),
                        ProviderAction::SetEnabled(provider.id.clone(), true),
                    )
                }
            } else if list.current.as_deref() == Some(&provider.id) {
                (
                    "●",
                    lang.text("当前使用", "Current"),
                    ProviderAction::Current,
                )
            } else {
                (
                    "○",
                    lang.text("回车切换", "Enter to switch"),
                    ProviderAction::Switch(provider.id.clone()),
                )
            };
            Item {
                label: format!(
                    "{glyph} {} · {}  [{state}]",
                    clean(&provider.name),
                    clean(&provider.id)
                ),
                action,
            }
        })
        .collect::<Vec<_>>();
    if !list.providers.is_empty() {
        items.push(Item {
            label: lang
                .text(
                    "编辑供应商 · Base URL / API Key",
                    "Edit provider · Base URL / API Key",
                )
                .into(),
            action: ProviderAction::Edit,
        });
    }
    items.extend([
        Item {
            label: lang
                .text(
                    "从当前客户端配置导入供应商",
                    "Import providers from the current client configuration",
                )
                .into(),
            action: ProviderAction::Import,
        },
        Item {
            label: lang
                .text("查看配置文件路径", "Show configuration paths")
                .into(),
            action: ProviderAction::Paths,
        },
        Item {
            label: lang.text("返回工具列表", "Back to tools").into(),
            action: ProviderAction::Back,
        },
    ]);
    items
}

/// Numbers keep truncated labels unique, so equal display names cannot select
/// a different provider. Stored IDs are never clipped or inferred from labels.
fn labels(items: impl IntoIterator<Item = String>, width: usize) -> Vec<String> {
    items
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            format!(
                "{:>2}. {}",
                index + 1,
                truncate_str(&clean(&value), width.saturating_sub(8).max(1), "…")
            )
        })
        .collect()
}

struct Menu<'a> {
    term: Term,
    theme: ColorfulTheme,
    home: Option<&'a Path>,
    lang: Language,
}

impl Drop for Menu<'_> {
    fn drop(&mut self) {
        let _ = self.term.show_cursor();
        let _ = self.term.flush();
    }
}

impl Menu<'_> {
    fn line(&self, text: &str) -> Result<(), CliError> {
        self.term.write_line(text).map_err(|_| terminal_error())
    }

    fn choose(
        &self,
        prompt: &str,
        items: Vec<String>,
        selected: usize,
    ) -> Result<Option<usize>, CliError> {
        let items = labels(items, self.term.size().1 as usize);
        let max_length = (self.term.size().0 as usize).saturating_sub(7).clamp(3, 12);
        FuzzySelect::with_theme(&self.theme)
            .with_prompt(prompt)
            .items(&items)
            .default(selected)
            .max_length(max_length)
            .report(false)
            .interact_on_opt(&self.term)
            .map_err(|problem| {
                let dialoguer::Error::IO(problem) = problem;
                if problem.kind() == io::ErrorKind::Interrupted {
                    CliError {
                        code: "interrupted".into(),
                        message: "Interrupted.",
                        exit: 130,
                    }
                } else {
                    terminal_error()
                }
            })
    }

    fn show_error(&self, problem: CliError) -> Result<(), CliError> {
        self.line(&error(&problem.code, problem.message, self.lang))
    }

    fn retry(&self, problem: CliError) -> Result<bool, CliError> {
        self.show_error(problem)?;
        Ok(self.choose(
            self.lang.text("下一步", "Next"),
            vec![
                self.lang.text("重试读取", "Retry reading").into(),
                self.lang.text("返回", "Back").into(),
            ],
            0,
        )? == Some(0))
    }

    fn execute(&self, command: Command) -> Result<(), CliError> {
        self.line(self.lang.text("正在处理…", "Working…"))?;
        match run(Cli {
            json: false,
            home: self.home.map(Path::to_path_buf),
            lang: self.lang,
            command,
        }) {
            Ok(output) => self.line(&output.text),
            Err(problem) => self.show_error(problem),
        }
    }

    fn tool(&self, tool: Tool) -> Result<(), CliError> {
        let mut selected = 0;
        loop {
            let list = match open_service(self.home)
                .and_then(|service| service.list_providers(tool).map_err(Into::into))
            {
                Ok(list) => list,
                Err(problem) => {
                    if self.retry(problem)? {
                        continue;
                    }
                    return Ok(());
                }
            };
            self.line(&format!("\n{}", style(name(tool)).bold()))?;
            if list.providers.is_empty() {
                self.line(self.lang.text(
                    "还没有供应商。可以先导入客户端现有配置，再选择使用。",
                    "No providers yet. Import your current client configuration to get started.",
                ))?;
                self.line(&format!(
                    "{} leanflip provider add {} --file provider.json",
                    self.lang
                        .text("也可从文件新增：", "Or add one from a JSON file:"),
                    tool
                ))?;
            } else if list.live_error.is_some() && tool.is_coexist() {
                self.line(self.lang.text(
                    "客户端配置无法读取，已暂停启用/停用。请查看路径并修复配置后重试。",
                    "Client configuration is unreadable; toggles are paused. Check its paths and repair the configuration.",
                ))?;
            }
            let items = provider_items(&list, self.lang);
            let choice = self.choose(
                self.lang.text(
                    "选择供应商或操作（Esc 返回）",
                    "Choose a provider or action (Esc back)",
                ),
                items.iter().map(|item| item.label.clone()).collect(),
                selected.min(items.len() - 1),
            )?;
            let Some(index) = choice else {
                return Ok(());
            };
            selected = index;
            if matches!(
                items[index].action,
                ProviderAction::Switch(_) | ProviderAction::SetEnabled(..) | ProviderAction::Import
            ) && !self.confirm_action(&items[index], &list)?
            {
                continue;
            }
            match &items[index].action {
                ProviderAction::Switch(id) => self.execute(Command::Use { tool: tool.into(), provider_id: id.clone() })?,
                ProviderAction::SetEnabled(id, true) => self.execute(Command::Enable { tool: tool.into(), provider_id: id.clone() })?,
                ProviderAction::SetEnabled(id, false) => self.execute(Command::Disable { tool: tool.into(), provider_id: id.clone() })?,
                ProviderAction::Current => self.line(self.lang.text("此供应商已是当前选择，无需切换。", "This provider is already selected."))?,
                ProviderAction::Unknown => self.line(self.lang.text("状态未知，未修改任何配置。请先修复客户端配置。", "State is unknown; no configuration was changed. Repair the client configuration first."))?,
                ProviderAction::Import => self.execute(Command::Import { source: Import::Live { tool: tool.into() } })?,
                ProviderAction::Edit => {
                    let labels = list.providers.iter().map(|provider| format!("{} · {}",clean(&provider.name),clean(&provider.id))).collect();
                    if let Some(index) = self.choose(self.lang.text("选择要编辑的供应商", "Choose a provider to edit"),labels,0)? {
                        self.execute(Command::Provider { action: crate::args::ProviderCommand::Edit { tool: tool.into(), provider_id:list.providers[index].id.clone() } })?;
                    }
                },
                ProviderAction::Paths => self.execute(Command::Paths { tool: Some(tool.into()) })?,
                ProviderAction::Back => return Ok(()),
            }
        }
    }

    fn confirm_action(&self, item: &Item, list: &ToolProviders) -> Result<bool, CliError> {
        self.line(&format!("\n{}", item.label))?;
        if matches!(item.action, ProviderAction::Import) {
            self.line(self.lang.text(
                "只保存供应商，不切换当前配置。",
                "Save providers without switching the client configuration.",
            ))?;
        } else {
            self.line(self.lang.text(
                "将更新以下客户端配置：",
                "The following client configuration will be updated:",
            ))?;
            for path in &list.paths {
                self.line(&format!("  {}", clean(&path.display().to_string())))?;
            }
        }
        Ok(self.choose(
            self.lang.text("确认操作", "Confirm action"),
            vec![
                self.lang.text("确认执行", "Apply").into(),
                self.lang
                    .text("取消，保留当前配置", "Cancel, keep current configuration")
                    .into(),
            ],
            0,
        )? == Some(0))
    }
}

pub(super) fn install_interrupt_handler() -> Result<(), CliError> {
    // console restores input mode before raising SIGINT, but dialoguer has
    // hidden the cursor. Restore it even when Ctrl+C exits outside Rust drops.
    ctrlc::set_handler(|| {
        let term = Term::stderr();
        let _ = term.show_cursor();
        let _ = term.write_line("");
        std::process::exit(130);
    })
    .map_err(|_| terminal_error())
}

pub fn start(home: Option<&Path>, lang: Language, tool: Option<Tool>) -> Result<(), CliError> {
    if !can_interact() {
        return Err(CliError::input("interactive_terminal", "The menu requires an interactive terminal. Run leanflip --help for non-interactive commands."));
    }
    install_interrupt_handler()?;
    let menu = Menu {
        term: Term::stderr(),
        theme: theme(),
        home,
        lang,
    };
    menu.line(&format!("\n{}", style("LeanFlip").bold().color256(104)))?;
    menu.line(lang.text(
        "交互菜单 · 输入筛选，↑↓选择，回车确认，Esc 返回或退出",
        "Interactive menu · type to filter, ↑↓ move, Enter confirm, Esc back or exit",
    ))?;
    if let Some(home) = home {
        menu.line(&format!(
            "{} {}",
            lang.text("配置根目录：", "Custom HOME:"),
            clean(&home.display().to_string())
        ))?;
    }
    if let Some(tool) = tool {
        menu.tool(tool)?;
    }
    let mut selected = 0;
    loop {
        let overview =
            match open_service(home).and_then(|service| service.overview().map_err(Into::into)) {
                Ok(overview) => overview,
                Err(problem) => {
                    if menu.retry(problem)? {
                        continue;
                    }
                    break;
                }
            };
        let mut items = overview
            .iter()
            .map(|summary| tool_label(summary, lang))
            .collect::<Vec<_>>();
        items.push(lang.text("查看全部状态", "Show all status").into());
        items.push(lang.text("退出", "Exit").into());
        let choice = menu.choose(lang.text("选择工具", "Choose a tool"), items, selected)?;
        match choice {
            Some(index) if index < overview.len() => {
                selected = index;
                menu.tool(overview[index].tool)?;
            }
            Some(index) if index == overview.len() => menu.execute(Command::Status)?,
            _ => break,
        }
    }
    menu.line(lang.text("已退出菜单。", "Menu closed."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use leanflip_core::Provider;
    use leanflip_service::service::ToolKind;
    use serde_json::json;

    fn fixture(tool: Tool) -> ToolProviders {
        ToolProviders {
            tool,
            kind: if tool.is_coexist() {
                ToolKind::Coexist
            } else {
                ToolKind::Switch
            },
            providers: ["first", "second"]
                .into_iter()
                .map(|id| {
                    serde_json::from_value::<Provider>(json!({
                        "id":id, "name":"Same name", "settingsConfig":{"secret":"hidden-token"}
                    }))
                    .unwrap()
                })
                .collect(),
            current: Some("first".into()),
            in_live: vec!["first".into()],
            live_error: None,
            paths: vec![],
        }
    }

    #[test]
    fn switching_uses_exact_ids_and_keeps_current_selection_read_only() {
        let items = provider_items(&fixture(Tool::Claude), Language::Zh);
        assert_eq!(items[0].action, ProviderAction::Current);
        assert_eq!(items[1].action, ProviderAction::Switch("second".into()));
        assert!(items[0].label.contains("当前使用"));
        assert!(!items.iter().any(|item| item.label.contains("hidden-token")));
    }

    #[test]
    fn coexist_toggles_reflect_live_state_and_block_unreadable_configuration() {
        let mut list = fixture(Tool::OpenCode);
        let items = provider_items(&list, Language::En);
        assert_eq!(
            items[0].action,
            ProviderAction::SetEnabled("first".into(), false)
        );
        assert_eq!(
            items[1].action,
            ProviderAction::SetEnabled("second".into(), true)
        );
        list.live_error = Some("malformed config containing hidden-token".into());
        let items = provider_items(&list, Language::En);
        assert_eq!(items[0].action, ProviderAction::Unknown);
        assert_eq!(items[1].action, ProviderAction::Unknown);
        assert!(!items.iter().any(|item| item.label.contains("hidden-token")));
    }

    #[test]
    fn empty_tools_still_offer_import_paths_and_a_way_back() {
        let mut list = fixture(Tool::Gemini);
        list.providers.clear();
        let actions = provider_items(&list, Language::Zh)
            .into_iter()
            .map(|item| item.action)
            .collect::<Vec<_>>();
        assert_eq!(
            actions,
            [
                ProviderAction::Import,
                ProviderAction::Paths,
                ProviderAction::Back
            ]
        );
    }

    #[test]
    fn clipped_labels_stay_unique_and_escape_terminal_controls() {
        let values = [
            "非常长的供应商名称".repeat(10),
            "非常长的供应商名称".repeat(10),
            "\u{1b}[2Jinjected\nname".into(),
        ];
        let labels = labels(values, 40);
        assert_ne!(labels[0], labels[1]);
        for label in &labels {
            assert!(!label.contains('\u{1b}'));
            assert!(!label.contains('\n'));
            assert!(unicode_width::UnicodeWidthStr::width(label.as_str()) <= 40);
        }
    }
}
