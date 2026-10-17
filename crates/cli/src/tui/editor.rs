use super::*;
use crate::editor::Draft;

pub(super) struct Editor {
    draft: Draft,
    paths: Vec<PathBuf>,
    field: usize,
    cursor: usize,
    confirmation: Option<bool>, // true = save, false = discard
    accept: bool,
    scroll: u16,
    max_scroll: u16,
    pub pending: bool,
    message: String,
}
impl Editor {
    pub(super) fn new(tool: Tool, provider: Provider, paths: Vec<PathBuf>) -> Self {
        let draft = Draft::new(tool, provider);
        let cursor = draft.name.len();
        Self {
            draft,
            paths,
            field: 0,
            cursor,
            confirmation: None,
            accept: false,
            scroll: 0,
            max_scroll: 0,
            pending: false,
            message: String::new(),
        }
    }
    pub(super) fn creation(tool: Tool, paths: Vec<PathBuf>) -> Self {
        let draft = Draft::creation(tool);
        let mut editor = Self::new(tool, draft.expected.clone(), paths);
        editor.draft = draft;
        editor
    }
    fn count(&self) -> usize {
        if self.draft.creating {
            5
        } else if self.draft.info.editable {
            3
        } else {
            1
        }
    }
    fn text(&self) -> &str {
        match self.field {
            0 => &self.draft.name,
            1 => &self.draft.url,
            2 => &self.draft.key,
            3 => &self.draft.id,
            _ => &self.draft.model,
        }
    }
    fn text_mut(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.draft.name,
            1 => &mut self.draft.url,
            2 => &mut self.draft.key,
            3 => &mut self.draft.id,
            _ => &mut self.draft.model,
        }
    }
    fn move_field(&mut self, delta: isize) {
        let count = self.count();
        self.field = (self.field as isize + delta).rem_euclid(count as isize) as usize;
        self.cursor = self.text().len();
    }
    pub(super) fn paste(&mut self, text: &str, lang: Language) {
        if self.confirmation.is_some() || self.pending {
            return;
        }
        let mut cursor = self.cursor;
        let accepted = input::insert(self.text_mut(), &mut cursor, text);
        self.cursor = cursor;
        if self.field == 0 {
            self.draft.suggest_id();
        }
        if !accepted {
            self.message = lang
                .text(
                    "只接受单行文字，最多 4096 个字符。",
                    "Use a single line of at most 4096 characters.",
                )
                .into();
        }
        if self.field == 2 && !self.draft.key.is_empty() {
            self.draft.clear_key = false;
        }
    }
    pub(super) fn key(&mut self, key: KeyEvent) -> bool {
        if self.pending {
            return false;
        }
        if let Some(save) = self.confirmation {
            match key.code {
                KeyCode::Esc | KeyCode::Char('n') => self.confirmation = None,
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => self.accept = !self.accept,
                KeyCode::Up | KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(3),
                KeyCode::Down | KeyCode::PageDown => {
                    self.scroll = self.scroll.saturating_add(3).min(self.max_scroll)
                }
                KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                    self.confirmation = None;
                    if self.accept || matches!(key.code, KeyCode::Char('y' | 'Y')) {
                        if save {
                            self.pending = true;
                        } else {
                            return true;
                        }
                    }
                }
                _ => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Esc => {
                if !self.draft.dirty() {
                    return true;
                }
                self.confirmation = Some(false);
                self.accept = false;
                self.scroll = 0;
            }
            KeyCode::Tab | KeyCode::Down => self.move_field(1),
            KeyCode::BackTab | KeyCode::Up => self.move_field(-1),
            KeyCode::Enter if self.field + 1 < self.count() => self.move_field(1),
            KeyCode::Enter => {
                if !self.draft.dirty() {
                    return true;
                }
                self.confirmation = Some(true);
                self.accept = false;
                self.scroll = 0;
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.draft.dirty() {
                    self.confirmation = Some(true);
                    self.accept = false;
                    self.scroll = 0;
                }
            }
            KeyCode::Char('a')
                if key.modifiers.contains(KeyModifiers::CONTROL) && self.draft.creating =>
            {
                self.draft.activate = !self.draft.activate;
            }
            KeyCode::Char('k')
                if key.modifiers.contains(KeyModifiers::CONTROL) && self.draft.info.editable =>
            {
                self.draft.clear_key = !self.draft.clear_key;
                self.draft.key.clear();
                if self.field == 2 {
                    self.cursor = 0;
                }
            }
            _ => {
                let mut cursor = self.cursor;
                input::key(self.text_mut(), &mut cursor, key);
                self.cursor = cursor;
                if self.field == 0 {
                    self.draft.suggest_id();
                }
                if self.field == 2 && !self.draft.key.is_empty() {
                    self.draft.clear_key = false;
                }
            }
        }
        false
    }
    pub(super) fn save(&mut self, home: Option<&Path>, lang: Language) -> Option<String> {
        if !self.pending {
            return None;
        }
        self.pending = false;
        match open_service(home).and_then(|service| {
            if service.env().live_paths(self.draft.tool) != self.paths {
                return Err(CliError {
                    code: "conflict".into(),
                    message: "Configuration paths changed while editing. Reopen the editor and review them.",
                    exit: 1,
                });
            }
            self.draft.save(&service, lang)
        }) {
            Ok(result) => Some(result.text),
            Err(problem) => {
                self.message = error(&problem.code, problem.message, lang);
                None
            }
        }
    }
    pub(super) fn render(&mut self, frame: &mut Frame, lang: Language) {
        let [header, body, message, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .areas(frame.area());
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(
                    " {} / {}",
                    if self.draft.creating {
                        lang.text("新建供应商", "Create provider")
                    } else {
                        lang.text("编辑供应商", "Edit provider")
                    },
                    name(self.draft.tool)
                ),
                accent(),
            )),
            header,
        );
        let block = panel(
            &input::fit(
                &clean(&self.draft.expected.id),
                body.width.saturating_sub(8) as usize,
            ),
            true,
        );
        let inner = block.inner(body);
        frame.render_widget(block, body);
        let key_hint = if self.draft.clear_key {
            lang.text("将清空 · Ctrl+K 撤销", "Will clear · Ctrl+K undo")
        } else if self.draft.info.api_key_set {
            lang.text("已配置 · 留空保留", "Configured · blank keeps it")
        } else {
            lang.text("未配置 · 输入后设置", "Not set · type to configure")
        };
        let url_hint = if self.draft.info.url_hidden {
            lang.text("含敏感内容 · 留空保留", "Sensitive URL · blank keeps it")
        } else {
            ""
        };
        let labels = [
            lang.text("名称", "Name").to_owned(),
            format!("Base URL  {url_hint}"),
            format!("API Key  {key_hint}"),
            "ID".into(),
            lang.text("模型（可留空）", "Model (optional)").into(),
        ];
        let values = [
            &self.draft.name,
            &self.draft.url,
            &self.draft.key,
            &self.draft.id,
            &self.draft.model,
        ];
        let count = self.count();
        let mut lines = vec![];
        for index in 0..count {
            lines.push(Line::from(Span::styled(
                input::fit(
                    &format!(
                        "{} {}",
                        if index == self.field { ">" } else { " " },
                        labels[index]
                    ),
                    inner.width as usize,
                ),
                if index == self.field {
                    accent()
                } else {
                    muted()
                },
            )));
            let shown = if index == self.field {
                input::field(
                    values[index],
                    self.cursor,
                    index == 2,
                    inner.width.saturating_sub(2) as usize,
                )
            } else if index == 2 {
                "*".repeat(
                    self.draft
                        .key
                        .chars()
                        .count()
                        .min(inner.width.saturating_sub(2) as usize),
                )
            } else {
                input::fit(
                    &clean(values[index]),
                    inner.width.saturating_sub(2) as usize,
                )
            };
            lines.push(Line::from(format!("  {shown}")));
            lines.push(Line::default());
        }
        if !self.draft.info.editable {
            lines.push(Line::from(lang.text(
                "官方登录 / 高级连接在 App 中管理。",
                "Manage official login / advanced settings in the App.",
            )));
        }
        let offset = (self.field * 3 + 2).saturating_sub(inner.height as usize) as u16;
        frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), inner);
        frame.render_widget(
            Paragraph::new(if self.pending {
                lang.text("正在保存…", "Saving…")
            } else {
                &self.message
            })
            .wrap(Wrap { trim: false }),
            message,
        );
        frame.render_widget(
            Paragraph::new(if self.draft.creating {
                format!(
                    "{} · {}\n{}",
                    lang.text("Ctrl+A 切换立即启用", "Ctrl+A toggle activation"),
                    if self.draft.activate {
                        lang.text("启用：是", "Activate: yes")
                    } else {
                        lang.text("启用：否", "Activate: no")
                    },
                    lang.text(
                        "Tab 字段  Ctrl+S 检查保存  Esc 取消",
                        "Tab field  Ctrl+S review  Esc cancel"
                    )
                )
            } else {
                lang.text(
                    "Tab 字段  Enter 下一步  Ctrl+S 检查保存\nCtrl+K 清空密钥  Esc 取消",
                    "Tab field  Enter next  Ctrl+S review\nCtrl+K clear key  Esc cancel",
                )
                .into()
            })
            .style(muted()),
            footer,
        );
        if let Some(save) = self.confirmation {
            let body = if save {
                self.draft.summary(&self.paths, lang)
            } else {
                lang.text(
                    "放弃未保存的修改？已有配置不会改变。",
                    "Discard unsaved edits? Existing configuration will not change.",
                )
                .into()
            };
            let mut dialog = Dialog::info(
                lang.text(
                    if save { "确认保存" } else { "放弃修改" },
                    if save {
                        "Confirm save"
                    } else {
                        "Discard edits"
                    },
                ),
                body,
            );
            dialog.accept = self.accept;
            dialog.scroll = self.scroll;
            render_dialog(frame, &mut dialog, lang, true);
            self.max_scroll = dialog.max_scroll;
        }
    }
}
