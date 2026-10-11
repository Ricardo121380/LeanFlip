//! Keyboard-first data center. Secrets are held only in input fields/service
//! staging, never included in summaries, errors, or terminal output.
use super::*;
use ccsl_core::ccswitch_export::{DuplicatePolicy, MAX_EXPORT_BYTES};
use ccsl_service::{service::RestoreOptions, Service};

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Home,
    LocalPath,
    Import,
    Export,
    Preview,
    Result,
}
enum Source {
    Local(PathBuf),
    Backup,
    File,
}
enum Job {
    Scan(Option<PathBuf>),
    Load,
    Save,
    Apply,
}
struct Choice {
    tool: Option<Tool>,
    label: String,
    checked: bool,
}

pub(super) struct DataPanel {
    service: Service,
    lang: Language,
    view: View,
    menu: usize,
    field: usize,
    cursor: usize,
    path: String,
    password: String,
    confirmation: String,
    source: Option<Source>,
    choices: Vec<Choice>,
    selection: ListState,
    accounts: bool,
    settings: bool,
    paths: bool,
    has_accounts: bool,
    has_settings: bool,
    has_paths: bool,
    duplicate: DuplicatePolicy,
    summary: String,
    message: String,
    confirm: bool,
    accept: bool,
    confirm_scroll: u16,
    confirm_max_scroll: u16,
    pending: Option<Job>,
}

impl DataPanel {
    pub(super) fn new(home: Option<&Path>, lang: Language) -> Result<Self, CliError> {
        Ok(Self {
            service: open_service(home)?,
            lang,
            view: View::Home,
            menu: 0,
            field: 0,
            cursor: 0,
            path: String::new(),
            password: String::new(),
            confirmation: String::new(),
            source: None,
            choices: vec![],
            selection: ListState::default(),
            accounts: false,
            settings: false,
            paths: false,
            has_accounts: false,
            has_settings: false,
            has_paths: false,
            duplicate: DuplicatePolicy::Skip,
            summary: String::new(),
            message: String::new(),
            confirm: false,
            accept: false,
            confirm_scroll: 0,
            confirm_max_scroll: 0,
            pending: None,
        })
    }
    fn clear_secrets(&mut self) {
        self.password.clear();
        self.confirmation.clear();
        if self.field > 0 {
            self.cursor = 0;
        }
    }
    fn reset(&mut self) {
        self.service.backup_discard();
        self.service.export_discard();
        self.clear_secrets();
        self.view = View::Home;
        self.source = None;
        self.message.clear();
        self.confirm = false;
    }
    fn form(&mut self, view: View) {
        self.reset();
        self.view = view;
        self.field = 0;
        self.path = if view == View::Export {
            "leanflip-backup.leanflip".into()
        } else {
            String::new()
        };
        self.cursor = self.path.len();
    }
    fn focus_field(&mut self, index: usize) {
        self.field = index;
        self.cursor = match index {
            0 => self.path.len(),
            1 => self.password.len(),
            _ => self.confirmation.len(),
        };
    }
    fn options(&self) -> RestoreOptions {
        RestoreOptions {
            tools: self
                .choices
                .iter()
                .filter(|choice| choice.checked)
                .filter_map(|choice| choice.tool)
                .collect(),
            accounts: self.accounts,
            settings: self.settings,
            paths: self.paths,
            duplicate: self.duplicate,
        }
    }
    fn scope_rows(&self) -> Vec<(String, bool)> {
        let mut rows = self
            .choices
            .iter()
            .map(|choice| (choice.label.clone(), choice.checked))
            .collect::<Vec<_>>();
        if self.has_accounts {
            rows.push((
                self.lang
                    .text(
                        "托管 Codex 账号与登录凭据",
                        "Managed Codex accounts and credentials",
                    )
                    .into(),
                self.accounts,
            ));
        }
        if self.has_settings {
            rows.push((
                self.lang.text("应用偏好", "Application preferences").into(),
                self.settings,
            ));
        }
        if self.has_paths {
            rows.push((
                self.lang
                    .text(
                        "本机目录覆盖（默认不恢复）",
                        "Machine directory overrides (off by default)",
                    )
                    .into(),
                self.paths,
            ));
        }
        rows
    }
    fn toggle(&mut self) {
        let index = self.selection.selected().unwrap_or(0);
        if let Some(choice) = self.choices.get_mut(index) {
            choice.checked = !choice.checked;
            return;
        }
        let mut extra = index.saturating_sub(self.choices.len());
        if self.has_accounts {
            if extra == 0 {
                self.accounts = !self.accounts;
                return;
            }
            extra -= 1;
        }
        if self.has_settings {
            if extra == 0 {
                self.settings = !self.settings;
                if !self.settings {
                    self.paths = false;
                }
                return;
            }
            extra -= 1;
        }
        if self.has_paths && extra == 0 {
            self.paths = !self.paths;
            if self.paths {
                self.settings = true;
            }
        }
    }
    pub(super) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub(super) fn paste(&mut self, text: &str) {
        if self.pending.is_some()
            || self.confirm
            || !matches!(self.view, View::LocalPath | View::Import | View::Export)
        {
            return;
        }
        let value = match self.field {
            0 => &mut self.path,
            1 => &mut self.password,
            _ => &mut self.confirmation,
        };
        let mut cursor = self.cursor;
        if !input::insert(value, &mut cursor, text) {
            self.message = self
                .lang
                .text(
                    "只接受单行文字，最多 4096 个字符。",
                    "Use a single line of at most 4096 characters.",
                )
                .into();
        }
        self.cursor = cursor;
    }
    /// Returns true only when the user leaves the data center.
    pub(super) fn key(&mut self, key: KeyEvent) -> bool {
        if self.pending.is_some() {
            return false;
        }
        if self.confirm {
            match key.code {
                KeyCode::Esc | KeyCode::Char('n') => self.confirm = false,
                KeyCode::Up | KeyCode::PageUp => {
                    self.confirm_scroll = self.confirm_scroll.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::PageDown => {
                    self.confirm_scroll = self
                        .confirm_scroll
                        .saturating_add(1)
                        .min(self.confirm_max_scroll)
                }
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    self.accept = !self.accept
                }
                KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                    self.confirm = false;
                    if self.accept || matches!(key.code, KeyCode::Char('y' | 'Y')) {
                        self.pending = Some(Job::Apply);
                    }
                }
                _ => {}
            }
            return false;
        }
        if key.code == KeyCode::Esc {
            if self.view == View::Home {
                return true;
            }
            self.reset();
            return false;
        }
        match self.view {
            View::Home => match key.code {
                KeyCode::Char('q') => return true,
                KeyCode::Up | KeyCode::Char('k') => self.menu = (self.menu + 2) % 3,
                KeyCode::Down | KeyCode::Char('j') => self.menu = (self.menu + 1) % 3,
                KeyCode::Char('o') => self.form(View::LocalPath),
                KeyCode::Enter => match self.menu {
                    0 => self.pending = Some(Job::Scan(None)),
                    1 => self.form(View::Import),
                    _ => self.form(View::Export),
                },
                _ => {}
            },
            View::LocalPath | View::Import | View::Export => {
                let count = if self.view == View::Export {
                    3
                } else if self.view == View::Import {
                    2
                } else {
                    1
                };
                match key.code {
                    KeyCode::Tab | KeyCode::Down => self.focus_field((self.field + 1) % count),
                    KeyCode::BackTab | KeyCode::Up => {
                        self.focus_field((self.field + count - 1) % count)
                    }
                    KeyCode::Enter if self.field + 1 < count => self.focus_field(self.field + 1),
                    KeyCode::Enter => {
                        if self.path.trim().is_empty() {
                            self.message = self
                                .lang
                                .text("请填写文件或目录路径。", "Enter a file or directory path.")
                                .into();
                        } else if self.view == View::Export
                            && (!(10..=4096).contains(&self.password.chars().count())
                                || self.password != self.confirmation)
                        {
                            self.message = self
                                .lang
                                .text(
                                    "密码需要 10–4096 个字符，且两次输入一致。",
                                    "Use 10–4096 characters and matching passphrases.",
                                )
                                .into();
                        } else {
                            self.message.clear();
                            self.pending = Some(match self.view {
                                View::LocalPath => Job::Scan(Some(self.input_path())),
                                View::Export => Job::Save,
                                _ => Job::Load,
                            });
                        }
                    }
                    _ => {
                        let input = match self.field {
                            0 => &mut self.path,
                            1 => &mut self.password,
                            _ => &mut self.confirmation,
                        };
                        input::key(input, &mut self.cursor, key);
                    }
                }
            }
            View::Preview => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.move_scope(-1),
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => self.move_scope(1),
                KeyCode::Char(' ') => self.toggle(),
                KeyCode::Char('d') if !matches!(self.source, Some(Source::Local(_))) => {
                    self.duplicate = if self.duplicate == DuplicatePolicy::Skip {
                        DuplicatePolicy::Rename
                    } else {
                        DuplicatePolicy::Skip
                    }
                }
                KeyCode::Enter => {
                    if self.options().tools.is_empty() && !self.accounts && !self.settings {
                        self.message = self
                            .lang
                            .text("至少选择一项恢复内容。", "Choose at least one scope.")
                            .into();
                    } else if matches!(self.source, Some(Source::Local(_)))
                        && self.accounts
                        && !self.options().tools.contains(&Tool::Codex)
                    {
                        self.message = self
                            .lang
                            .text(
                                "迁入账号时请同时选择 Codex。",
                                "Select Codex to include its managed accounts.",
                            )
                            .into();
                    } else {
                        self.confirm = true;
                        self.accept = false;
                        self.confirm_scroll = 0;
                        self.message.clear();
                    }
                }
                _ => {}
            },
            View::Result => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Char('q')) {
                    self.reset();
                }
            }
        }
        false
    }
    fn move_scope(&mut self, delta: isize) {
        let len = self.scope_rows().len();
        if len > 0 {
            self.selection.select(Some(
                (self.selection.selected().unwrap_or(0) as isize + delta).rem_euclid(len as isize)
                    as usize,
            ));
        }
    }
    fn input_path(&self) -> PathBuf {
        let path = self.path.trim();
        if let Some(rest) = path.strip_prefix("~/") {
            self.service.env().home.join(rest)
        } else {
            PathBuf::from(path)
        }
    }
    fn prepare(
        &mut self,
        source: Source,
        rows: Vec<(Tool, String)>,
        accounts: usize,
        settings: bool,
        paths: bool,
    ) {
        self.choices = Tool::ALL
            .iter()
            .filter_map(|tool| {
                let names = rows
                    .iter()
                    .filter(|(kind, _)| kind == tool)
                    .map(|(_, name)| name.as_str())
                    .collect::<Vec<_>>();
                if names.is_empty() && !(*tool == Tool::Codex && accounts > 0) {
                    return None;
                }
                Some(Choice {
                    tool: Some(*tool),
                    label: format!("{} · {}  {}", name(*tool), names.len(), names.join(" · ")),
                    checked: true,
                })
            })
            .collect();
        self.summary = format!(
            "{} {} · {} {}",
            self.lang.text("供应商卡片", "Provider cards"),
            rows.len(),
            self.lang.text("托管账号", "Managed accounts"),
            accounts
        );
        self.source = Some(source);
        self.has_accounts = accounts > 0;
        self.accounts = accounts > 0;
        self.has_settings = settings;
        self.settings = settings;
        self.has_paths = paths;
        self.paths = false;
        self.duplicate = DuplicatePolicy::Skip;
        self.selection.select(Some(0));
        self.view = View::Preview;
    }
    /// Called only after a busy frame has been drawn. Returns whether a write
    /// was attempted so the parent refreshes its view of the shared library.
    pub(super) fn run_pending(&mut self) -> bool {
        let Some(job) = self.pending.take() else {
            return false;
        };
        let changed = matches!(job, Job::Apply);
        let result = self.perform(job);
        self.clear_secrets();
        if let Err(problem) = result {
            self.message = error(&problem.code, problem.message, self.lang);
        }
        changed
    }
    fn perform(&mut self, job: Job) -> Result<(), CliError> {
        match job {
            Job::Scan(dir) => {
                let scan = self.service.ccswitch_scan(dir)?;
                let skipped = scan
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.skip_reason.is_some())
                    .count();
                let rows = scan
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.skip_reason.is_none())
                    .map(|candidate| (candidate.tool, clean(&candidate.provider.name)))
                    .collect();
                self.path = scan.directory.display().to_string();
                self.prepare(
                    Source::Local(scan.directory),
                    rows,
                    scan.accounts,
                    false,
                    false,
                );
                self.message = format!(
                    "{} {skipped}",
                    self.lang.text(
                        "重复或不兼容卡片跳过：",
                        "Duplicate or incompatible cards skipped:"
                    )
                );
                if self.choices.is_empty() {
                    self.message = self
                        .lang
                        .text(
                            "没有发现可迁移数据。Esc 返回后按 o 指定数据目录。",
                            "No data found. Esc, then o to choose a data directory.",
                        )
                        .into();
                }
            }
            Job::Load => {
                self.service.backup_discard();
                self.service.export_discard();
                let bytes = crate::read_limited(&self.input_path(), MAX_EXPORT_BYTES, false)?;
                if bytes.starts_with(b"age-encryption.org/")
                    || self.path.ends_with(".leanflip")
                    || self.path.ends_with(".switchleaf")
                    || self.path.ends_with(".age")
                {
                    let preview = self.service.backup_load(&bytes, &self.password)?;
                    let rows = preview
                        .items
                        .iter()
                        .map(|item| (item.tool, clean(&item.name)))
                        .collect();
                    self.prepare(
                        Source::Backup,
                        rows,
                        preview.accounts,
                        preview.has_settings,
                        preview.has_paths,
                    );
                } else {
                    let preview = self.service.export_load(&bytes, DuplicatePolicy::Skip)?;
                    let rows = preview
                        .items
                        .iter()
                        .filter(|item| item.status != ccsl_core::ccswitch_export::ItemStatus::Skip)
                        .filter_map(|item| item.tool.map(|tool| (tool, clean(&item.name))))
                        .collect();
                    self.prepare(Source::File, rows, 0, false, false);
                }
            }
            Job::Save => {
                self.service
                    .backup_export_file(&self.input_path(), &self.password)?;
                self.view = View::Result;
                self.message = format!("{}\n\n{}\n\n{}", self.lang.text("加密备份已保存", "Encrypted backup saved"), clean(&self.input_path().display().to_string()), self.lang.text("包含供应商、托管账号和应用偏好。请妥善保管密码。", "Includes providers, managed accounts and preferences. Keep the passphrase safe."));
            }
            Job::Apply => {
                let options = self.options();
                match self.source.as_ref().expect("preview has a source") {
                    Source::Backup => {
                        let report = self.service.backup_apply(options)?;
                        self.message = crate::data::report_text(&report, self.lang);
                        if !report.failures.is_empty() {
                            return Ok(());
                        }
                    }
                    Source::Local(dir) => {
                        let report = self.service.ccswitch_apply_options(
                            Some(dir.clone()),
                            options.tools,
                            options.accounts,
                        )?;
                        self.message = format!(
                            "{} {} · {} {}\n{}",
                            self.lang.text("已保存卡片", "Cards saved"),
                            report.created.len(),
                            self.lang.text("新增账号", "Accounts added"),
                            report.accounts_added.len(),
                            self.lang.text(
                                "源数据和客户端当前配置未修改。",
                                "Source data and current client configurations were not changed."
                            )
                        );
                    }
                    Source::File => {
                        let report = self
                            .service
                            .export_apply(options.tools, options.duplicate)?;
                        self.message = format!(
                            "{} {} · {} {}\n{}",
                            self.lang.text("已保存卡片", "Cards saved"),
                            report.created.len(),
                            self.lang.text("跳过", "Skipped"),
                            report.skipped,
                            self.lang.text(
                                "客户端当前配置未切换。",
                                "Current client configurations were not switched."
                            )
                        );
                    }
                }
                self.view = View::Result;
            }
        }
        Ok(())
    }
    pub(super) fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let [header, content, status, footer] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("  LeanFlip  /  DATA", accent())),
                Line::from(self.lang.text(
                    "  数据与迁移 · 预览后再决定写入",
                    "  Data & migration · preview before applying",
                )),
            ]),
            header,
        );
        let block = panel(self.lang.text("数据中心", "Data center"), true);
        let inner = block.inner(content);
        frame.render_widget(block, content);
        match self.view {
            View::Home => {
                let labels = [
                    (
                        "01  检测本机 CC Switch",
                        "01  Detect local CC Switch",
                        "直接读取默认或自定义目录",
                        "Read its default or configured directory",
                    ),
                    (
                        "02  导入文件",
                        "02  Import a file",
                        "LeanFlip 加密备份 / CC Switch 导出",
                        "LeanFlip encrypted backup / CC Switch export",
                    ),
                    (
                        "03  导出加密备份",
                        "03  Export encrypted backup",
                        "保存供应商、托管账号和应用偏好",
                        "Save providers, managed accounts and preferences",
                    ),
                ];
                let rows = labels
                    .iter()
                    .map(|(zh, en, hz, he)| {
                        ListItem::new(vec![
                            Line::from(self.lang.text(zh, en)),
                            Line::from(Span::styled(
                                format!("    {}", self.lang.text(hz, he)),
                                muted(),
                            )),
                            Line::default(),
                        ])
                    })
                    .collect::<Vec<_>>();
                let mut state = ListState::default().with_selected(Some(self.menu));
                frame.render_stateful_widget(
                    List::new(rows)
                        .highlight_symbol(" › ")
                        .highlight_style(selected_style(true)),
                    inner,
                    &mut state,
                );
            }
            View::Import | View::Export | View::LocalPath => {
                let labels = [
                    self.lang.text(
                        if self.view == View::LocalPath {
                            "CC Switch 数据目录（绝对路径）"
                        } else {
                            "文件路径"
                        },
                        if self.view == View::LocalPath {
                            "CC Switch data directory (absolute path)"
                        } else {
                            "File path"
                        },
                    ),
                    self.lang.text(
                        if self.view == View::Import {
                            "备份密码（SQL / DB / JSON 可留空）"
                        } else {
                            "备份密码（至少 10 个字符）"
                        },
                        if self.view == View::Import {
                            "Passphrase (blank for SQL / DB / JSON)"
                        } else {
                            "Passphrase (at least 10 characters)"
                        },
                    ),
                    self.lang.text("再次输入密码", "Confirm passphrase"),
                ];
                let count = if self.view == View::Export {
                    3
                } else if self.view == View::Import {
                    2
                } else {
                    1
                };
                let inputs = [
                    clean(&self.path),
                    "*".repeat(self.password.chars().count()),
                    "*".repeat(self.confirmation.chars().count()),
                ];
                // Compact two-line fields remain usable even at 44×12.
                let mut lines = Vec::new();
                for index in 0..count {
                    let focused = self.field == index;
                    lines.push(Line::from(Span::styled(
                        input::fit(
                            &format!(" {} {}", if focused { ">" } else { " " }, labels[index]),
                            inner.width as usize,
                        ),
                        if focused { accent() } else { muted() },
                    )));
                    let source = match index {
                        0 => &self.path,
                        1 => &self.password,
                        _ => &self.confirmation,
                    };
                    let shown = if focused {
                        input::field(
                            source,
                            self.cursor,
                            index > 0,
                            inner.width.saturating_sub(3) as usize,
                        )
                    } else {
                        input::fit(&inputs[index], inner.width.saturating_sub(3) as usize)
                    };
                    lines.push(Line::from(format!("   {shown}")));
                }
                let offset = if inner.height < (count * 2) as u16 {
                    (self.field * 2).saturating_sub(inner.height.saturating_sub(2) as usize) as u16
                } else {
                    0
                };
                frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), inner);
            }
            View::Preview => {
                let [summary, rows, duplicate] = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Min(1),
                    Constraint::Length(2),
                ])
                .areas(inner);
                frame.render_widget(
                    Paragraph::new(vec![
                        Line::from(self.summary.as_str()),
                        Line::from(Span::styled(
                            input::fit(&clean(&self.path), summary.width as usize),
                            muted(),
                        )),
                    ]),
                    summary,
                );
                let items = self
                    .scope_rows()
                    .into_iter()
                    .map(|(label, checked)| {
                        ListItem::new(input::fit(
                            &format!("[{}] {label}", if checked { "x" } else { " " }),
                            rows.width.saturating_sub(3) as usize,
                        ))
                    })
                    .collect::<Vec<_>>();
                frame.render_stateful_widget(
                    List::new(items)
                        .highlight_symbol(" › ")
                        .highlight_style(selected_style(true)),
                    rows,
                    &mut self.selection,
                );
                let policy = if matches!(self.source, Some(Source::Local(_))) {
                    self.lang
                        .text("重复 ID：保留本机卡片", "Duplicate IDs: keep local cards")
                } else if self.duplicate == DuplicatePolicy::Skip {
                    self.lang.text("d  重复 ID：跳过", "d  Duplicate IDs: skip")
                } else {
                    self.lang
                        .text("d  重复 ID：另存副本", "d  Duplicate IDs: import copies")
                };
                frame.render_widget(
                    Paragraph::new(vec![
                        Line::from(policy),
                        Line::from(self.lang.text(
                            "客户端配置不会自动切换。",
                            "Client configurations will not be switched.",
                        )),
                    ])
                    .style(muted()),
                    duplicate,
                );
            }
            View::Result => frame.render_widget(
                Paragraph::new(self.message.as_str()).wrap(Wrap { trim: false }),
                inner,
            ),
        }
        if self.view != View::Result {
            frame.render_widget(
                Paragraph::new(if self.pending.is_some() {
                    self.lang.text("正在处理，请稍候…", "Working, please wait…")
                } else {
                    &self.message
                })
                .wrap(Wrap { trim: false }),
                status,
            );
        }
        let hints = match self.view {
            View::Home => self.lang.text(
                "↑↓ 选择  Enter 打开  o 手动目录\nEsc 返回工作台",
                "↑↓ select  Enter open  o directory\nEsc workspace",
            ),
            View::Preview => self.lang.text(
                "↑↓ 选择  Space 勾选  Enter 检查\nEsc 返回",
                "↑↓ select  Space toggle  Enter review\nEsc back",
            ),
            View::Result => self.lang.text(
                "Enter 返回数据中心  Esc 返回",
                "Enter data center  Esc back",
            ),
            _ => self.lang.text(
                "Tab 字段  Enter 下一步  Ctrl+U 清空\nEsc 取消 · 路径无需引号",
                "Tab field  Enter next  Ctrl+U clear\nEsc cancel · paths need no quotes",
            ),
        };
        frame.render_widget(Paragraph::new(hints).style(muted()), footer);
        if self.confirm {
            let scopes = self
                .scope_rows()
                .into_iter()
                .filter(|(_, selected)| *selected)
                .map(|(label, _)| label)
                .collect::<Vec<_>>()
                .join("\n");
            let mut dialog = Dialog::info(
                self.lang.text("确认恢复范围", "Confirm restore scope"),
                format!(
                    "{}\n\n{scopes}\n\n{}",
                    clean(&self.path),
                    self.lang.text(
                        "客户端不会自动切换。本机已有账号凭据会保留。",
                        "Clients will not be switched. Existing account credentials are kept."
                    )
                ),
            );
            dialog.accept = self.accept;
            dialog.scroll = self.confirm_scroll;
            render_dialog(frame, &mut dialog, self.lang, true);
            self.confirm_max_scroll = dialog.max_scroll;
        }
    }
}
