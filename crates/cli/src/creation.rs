//! Provider wizard shared by Menu and the full-screen editor.
use crate::{args::Language, editor::Draft, menu, CliError, Output};
use dialoguer::{Confirm, Input, Password};
use leanflip_core::{Provider, Tool};
use leanflip_service::Service;
use serde_json::json;

pub(super) fn provider(draft: &Draft) -> Result<Provider, CliError> {
    let id = draft.id.trim();
    let name = draft.name.trim();
    if id.is_empty() || name.is_empty() {
        return Err(CliError::input(
            "invalid_input",
            "Name and ID are required.",
        ));
    }
    let url = draft.url.trim();
    if !url.is_empty()
        && !url::Url::parse(url).is_ok_and(|value| {
            matches!(value.scheme(), "http" | "https")
                && value.host_str().is_some()
                && value.username().is_empty()
                && value.password().is_none()
        })
    {
        return Err(CliError::input(
            "invalid_input",
            "Base URL must be an HTTP(S) address.",
        ));
    }
    let key = &draft.key;
    let model = draft.model.trim();
    let models = if model.is_empty() {
        vec![]
    } else {
        vec![json!({"id":model,"name":model})]
    };
    let model_map = if model.is_empty() {
        json!({})
    } else {
        json!({model:{"name":model}})
    };
    let config = match draft.tool {
        Tool::Claude | Tool::ClaudeDesktop => {
            let mut value = json!({"env":{"ANTHROPIC_BASE_URL":url,"ANTHROPIC_AUTH_TOKEN":key}});
            if !model.is_empty() {
                value["env"]["ANTHROPIC_MODEL"] = json!(model);
            }
            value
        }
        Tool::Gemini => {
            json!({"env":{"GOOGLE_GEMINI_BASE_URL":url,"GEMINI_API_KEY":key,"GEMINI_MODEL":model}})
        }
        Tool::Codex => {
            let model = if model.is_empty() { "gpt-5.5" } else { model };
            let config: toml::Value = serde_json::from_value(json!({"model":model,"model_provider":"custom","model_providers":{"custom":{"name":name,"base_url":url,"wire_api":"responses","requires_openai_auth":true}}})).map_err(|_| CliError::input("config", "Invalid provider configuration."))?;
            json!({"auth":{"OPENAI_API_KEY":key},"config":toml::to_string(&config).map_err(|_| CliError::input("config", "Invalid provider configuration."))?})
        }
        Tool::GrokBuild => {
            let model = if model.is_empty() { "grok-4.5" } else { model };
            let config: toml::Value = serde_json::from_value(json!({"models":{"default":model},"model":{model:{"model":model,"name":name,"base_url":url,"api_key":key,"api_backend":"responses","context_window":500000}}})).map_err(|_| CliError::input("config", "Invalid provider configuration."))?;
            json!({"config":toml::to_string(&config).map_err(|_| CliError::input("config", "Invalid provider configuration."))?})
        }
        Tool::OpenCode => {
            json!({"npm":"@ai-sdk/openai-compatible","name":name,"options":{"baseURL":url,"apiKey":key},"models":model_map})
        }
        Tool::OpenClaw => {
            json!({"baseUrl":url,"apiKey":key,"api":"openai-completions","models":models})
        }
        Tool::Hermes => {
            json!({"name":name,"base_url":url,"api_key":key,"api_mode":"chat_completions","models":models})
        }
        Tool::Pi => {
            json!({"name":name,"baseUrl":url,"apiKey":key,"api":"openai-completions","models":models})
        }
        Tool::Mcode => {
            json!({"name":name,"kind":"custom","enabled":true,"api":"openai-completions","options":{"baseURL":url,"apiKey":key},"models":model_map})
        }
    };
    Ok(Provider::new(id, name, config))
}

pub(super) fn start(service: &Service, tool: Tool, lang: Language) -> Result<Output, CliError> {
    if !menu::can_interact() {
        return Err(CliError::input(
            "interactive_terminal",
            "The wizard requires an interactive terminal. Use provider add --file for scripts.",
        ));
    }
    let theme = menu::theme();
    let mut draft = Draft::creation(tool);
    draft.name = Input::with_theme(&theme)
        .with_prompt(lang.text("名称", "Name"))
        .interact_text()
        .map_err(|_| menu::terminal_error())?;
    draft.suggest_id();
    for (label, value) in [("ID", &mut draft.id), ("Base URL", &mut draft.url)] {
        *value = Input::with_theme(&theme)
            .with_prompt(label)
            .with_initial_text(value.as_str())
            .interact_text()
            .map_err(|_| menu::terminal_error())?;
    }
    draft.key = Password::with_theme(&theme)
        .with_prompt(lang.text("API Key（隐藏输入，可留空）", "API Key (hidden; optional)"))
        .allow_empty_password(true)
        .interact()
        .map_err(|_| menu::terminal_error())?;
    draft.model = Input::with_theme(&theme)
        .with_prompt(lang.text("模型（可留空）", "Model (optional)"))
        .allow_empty(true)
        .interact_text()
        .map_err(|_| menu::terminal_error())?;
    draft.activate = Confirm::with_theme(&theme)
        .with_prompt(lang.text("保存后立即启用？", "Activate after saving?"))
        .default(false)
        .interact()
        .map_err(|_| menu::terminal_error())?;
    provider(&draft)?;
    dialoguer::console::Term::stderr()
        .write_line(&draft.summary(&service.env().live_paths(tool), lang))
        .map_err(|_| menu::terminal_error())?;
    if !Confirm::with_theme(&theme)
        .with_prompt(lang.text("保存供应商？", "Save provider?"))
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

pub(super) fn suggest_id(name: &str) -> String {
    let slug = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase();
    if slug.is_empty() {
        "provider".into()
    } else {
        slug
    }
}
