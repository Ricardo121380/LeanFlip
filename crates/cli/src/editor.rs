//! Shared terminal editing flow. Existing secrets are never put into fields.
use crate::{args::Language, menu, presentation::clean, CliError, Output};
use dialoguer::{Confirm, FuzzySelect, Input, Password};
use leanflip_core::{Provider, Tool};
use leanflip_service::{
    service::{connection_info, ApiKeyChange, ConnectionInfo, ConnectionUpdate},
    Service,
};

pub(super) struct Draft {
    pub tool: Tool,
    pub expected: Provider,
    pub info: ConnectionInfo,
    pub name: String,
    pub url: String,
    initial_url: String,
    pub key: String,
    pub clear_key: bool,
    pub creating: bool,
    pub id: String,
    generated_id: String,
    pub model: String,
    pub activate: bool,
}
impl Draft {
    pub fn new(tool: Tool, provider: Provider) -> Self {
        let info = connection_info(tool, &provider);
        let url = if info.url_hidden {
            String::new()
        } else {
            info.base_url.clone().unwrap_or_default()
        };
        Self {
            tool,
            name: clean(&provider.name),
            expected: provider,
            info,
            initial_url: url.clone(),
            url,
            key: String::new(),
            clear_key: false,
            creating: false,
            id: String::new(),
            generated_id: "custom".into(),
            model: String::new(),
            activate: false,
        }
    }
    pub fn creation(tool: Tool) -> Self {
        let mut draft = Self::new(tool, Provider::new("custom", "", serde_json::json!({})));
        draft.creating = true;
        draft.id = "custom".into();
        draft.info.editable = true;
        draft
    }
    pub fn suggest_id(&mut self) {
        if self.creating && self.id == self.generated_id {
            self.id = crate::creation::suggest_id(&self.name);
            self.generated_id = self.id.clone();
        }
    }
    pub fn dirty(&self) -> bool {
        if self.creating {
            return !self.name.is_empty()
                || !self.url.is_empty()
                || !self.key.is_empty()
                || !self.model.is_empty()
                || self.id != "custom"
                || self.activate;
        }
        self.name != self.expected.name
            || self.url != self.initial_url
            || !self.key.is_empty()
            || self.clear_key
    }
    pub fn summary(&self, paths: &[std::path::PathBuf], lang: Language) -> String {
        if self.creating {
            return format!(
                "{} · {}\n{}: {}\nID: {}\nBase URL: {}\nAPI Key: {}\n{}: {}\n{}\n{}",
                menu::name(self.tool),
                lang.text("新建供应商", "Create provider"),
                lang.text("名称", "Name"),
                clean(&self.name),
                clean(&self.id),
                safe_url(&self.url),
                if self.key.is_empty() {
                    lang.text("未配置", "Not set")
                } else {
                    lang.text("已输入（内容隐藏）", "Entered (hidden)")
                },
                lang.text("模型", "Model"),
                if self.model.is_empty() {
                    match self.tool {
                        Tool::Codex => "gpt-5.5",
                        Tool::GrokBuild => "grok-4.5",
                        _ => lang.text("客户端默认 / 未设置", "Client default / not set"),
                    }
                } else {
                    &self.model
                },
                if self.activate {
                    lang.text("保存并立即启用，将写入：", "Save and activate; writes:")
                } else {
                    lang.text("仅保存卡片，不启用。", "Save the card without activation.")
                },
                if self.activate {
                    paths
                        .iter()
                        .map(|p| clean(&p.display().to_string()))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    String::new()
                }
            );
        }
        let endpoint = if self.url == self.initial_url {
            self.info
                .base_url
                .as_deref()
                .unwrap_or(lang.text("客户端默认", "Client default"))
        } else if self.url.is_empty() {
            lang.text("清除地址覆盖", "Clear URL override")
        } else {
            &self.url
        };
        // Typed URLs may still be invalid; credentials/query strings must never
        // be echoed in the preview.
        let endpoint = safe_url(endpoint);
        format!("{} · {}\nID: {}\n\n{} {}\nBase URL: {}\nAPI Key: {}\n\n{}\n{}", menu::name(self.tool), lang.text("编辑供应商", "Edit provider"), clean(&self.expected.id), lang.text("名称：", "Name:"),clean(&self.name), endpoint,
            if self.clear_key { lang.text("将清空", "will be cleared") } else if self.key.is_empty() { lang.text("保持不变", "keep existing") } else { lang.text("将替换（内容隐藏）", "replace (hidden)") },
            lang.text("保存供应商；如果正在使用，会同步以下客户端配置。不会切换到未使用的供应商。", "Save the provider; if it is active, these client files are synchronized. Inactive providers are not activated."), paths.iter().map(|path|clean(&path.display().to_string())).collect::<Vec<_>>().join("\n"))
    }
    pub fn save(&mut self, service: &Service, lang: Language) -> Result<Output, CliError> {
        if self.creating {
            let result =
                service.add_provider(self.tool, crate::creation::provider(self)?, self.activate)?;
            self.key.clear();
            return Ok(Output::operation(
                result,
                lang,
                "供应商已创建。",
                "Provider created.",
            ));
        }
        let update = ConnectionUpdate {
            name: self.name.clone(),
            base_url: (self.url != self.initial_url).then(|| self.url.trim().to_owned()),
            api_key: if self.clear_key {
                ApiKeyChange::Clear
            } else if self.key.is_empty() {
                ApiKeyChange::Keep
            } else {
                ApiKeyChange::Set(self.key.clone())
            },
        };
        let result = service.update_connection(self.tool, &self.expected, update)?;
        self.key.clear();
        Ok(Output::operation(
            result,
            lang,
            "供应商连接设置已保存。",
            "Provider connection saved.",
        ))
    }
}

// Keep this display boundary independent from parsers and credential contents.
pub(super) fn safe_url(value: &str) -> String {
    if value.contains('@') || value.contains('?') || value.contains('#') {
        return "[URL: hidden sensitive components]".into();
    }
    clean(value)
}
pub(super) fn connection_lines(tool: Tool, provider: &Provider, lang: Language) -> String {
    let info = connection_info(tool, provider);
    format!(
        "Base URL: {}{}\nAPI Key: {}\n{}",
        info.base_url.as_deref().map(clean).unwrap_or_else(|| lang
            .text("客户端默认 / 未设置", "Client default / not set")
            .into()),
        if info.url_hidden {
            lang.text("（敏感部分隐藏）", " (sensitive parts hidden)")
        } else {
            ""
        },
        if info.uses_login {
            lang.text("使用官方 / 托管登录", "Official / managed login")
        } else if !info.editable {
            lang.text("由高级配置管理", "Managed by advanced configuration")
        } else if info.api_key_set {
            lang.text("已配置 · 不显示密钥", "Configured · hidden")
        } else {
            lang.text("未配置", "Not set")
        },
        if info.editable {
            lang.text(
                "e 编辑名称、Base URL 和 API Key",
                "e edit name, Base URL and API Key",
            )
        } else {
            lang.text(
                "连接由官方登录或高级配置管理；e 可改名称",
                "Connection managed by login or advanced config; e edits the name",
            )
        }
    )
}

pub(super) fn start(
    service: &Service,
    tool: Tool,
    id: &str,
    lang: Language,
) -> Result<Output, CliError> {
    if !menu::can_interact() {
        return Err(CliError::input(
            "interactive_terminal",
            "Editing requires an interactive terminal. Use provider update --file for scripts.",
        ));
    }
    let list = service.list_providers(tool)?;
    let provider = list
        .providers
        .into_iter()
        .find(|provider| provider.id == id)
        .ok_or_else(|| CliError::input("not_found", "Provider not found."))?;
    let theme = menu::theme();
    let mut draft = Draft::new(tool, provider);
    let term = dialoguer::console::Term::stderr();
    term.write_line(&format!(
        "\n{}\n{}",
        clean(&draft.name),
        connection_lines(tool, &draft.expected, lang)
    ))
    .map_err(|_| menu::terminal_error())?;
    draft.name = Input::with_theme(&theme)
        .with_prompt(lang.text("名称", "Name"))
        .with_initial_text(&draft.name)
        .interact_text()
        .map_err(|_| menu::terminal_error())?;
    if draft.info.editable {
        if draft.info.url_hidden {
            term.write_line(lang.text(
                "已有 URL 含敏感部分，留空可保留。",
                "The existing URL has sensitive components; leave blank to keep it.",
            ))
            .map_err(|_| menu::terminal_error())?;
        }
        draft.url = Input::with_theme(&theme)
            .with_prompt("Base URL")
            .with_initial_text(&draft.url)
            .allow_empty(true)
            .interact_text()
            .map_err(|_| menu::terminal_error())?;
        let action = FuzzySelect::with_theme(&theme)
            .with_prompt("API Key")
            .items([
                lang.text("保持现有密钥", "Keep existing key"),
                lang.text(
                    "设置 / 替换密钥（隐藏输入）",
                    "Set / replace key (hidden input)",
                ),
                lang.text("清空密钥", "Clear key"),
            ])
            .default(0)
            .interact_opt()
            .map_err(|_| menu::terminal_error())?;
        match action {
            Some(1) => {
                draft.key = Password::with_theme(&theme)
                    .with_prompt(
                        lang.text("新的 API Key（隐藏输入）", "New API Key (hidden input)"),
                    )
                    .interact()
                    .map_err(|_| menu::terminal_error())?
            }
            Some(2) => draft.clear_key = true,
            None => {
                return Ok(Output::done(
                    lang,
                    "已取消，未修改配置。",
                    "Cancelled; nothing changed.",
                ))
            }
            _ => {}
        }
    }
    if !draft.dirty() {
        return Ok(Output::done(lang, "没有修改。", "No changes."));
    }
    term.write_line(&draft.summary(&list.paths, lang))
        .map_err(|_| menu::terminal_error())?;
    if !Confirm::with_theme(&theme)
        .with_prompt(lang.text("保存这些修改？", "Save these changes?"))
        .default(false)
        .interact()
        .map_err(|_| menu::terminal_error())?
    {
        return Ok(Output::done(
            lang,
            "已取消，未修改配置。",
            "Cancelled; nothing changed.",
        ));
    }
    draft.save(service, lang)
}
