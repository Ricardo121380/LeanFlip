//! Connection summaries and targeted edits for terminal clients. Summaries
//! never contain credentials; edits preserve all unrelated provider settings.
use super::*;
use serde_json::{Map, Value};
use toml_edit::DocumentMut;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub base_url: Option<String>,
    pub api_key_set: bool,
    pub editable: bool,
    pub uses_login: bool,
    /// URLs carrying credentials or query parameters are not prefilled in forms.
    pub url_hidden: bool,
}

pub enum ApiKeyChange {
    Keep,
    Set(String),
    Clear,
}
pub struct ConnectionUpdate {
    pub name: String,
    /// None keeps the existing URL, Some("") clears the explicit override.
    pub base_url: Option<String>,
    pub api_key: ApiKeyChange,
}

fn unsupported() -> AppError {
    AppError::localized("provider.connection_unsupported", "此卡使用官方登录或高级配置；连接字段请在 App 或原始配置中管理。", "This card uses official login or advanced configuration. Manage its connection in the App or original configuration.")
}
fn invalid() -> AppError {
    AppError::localized(
        "provider.connection_invalid",
        "请输入有效的名称和 HTTP(S) Base URL；新地址不能包含账号密码、查询参数或片段。",
        "Enter a valid name and HTTP(S) Base URL without credentials, a query or fragment.",
    )
}
fn json_paths(tool: Tool, config: &Value) -> (&'static str, &'static str) {
    match tool {
        Tool::Claude | Tool::ClaudeDesktop => (
            "/env/ANTHROPIC_BASE_URL",
            if config
                .pointer("/env/ANTHROPIC_API_KEY")
                .and_then(Value::as_str)
                .is_some_and(|key| !key.is_empty())
                && !config
                    .pointer("/env/ANTHROPIC_AUTH_TOKEN")
                    .and_then(Value::as_str)
                    .is_some_and(|key| !key.is_empty())
            {
                "/env/ANTHROPIC_API_KEY"
            } else {
                "/env/ANTHROPIC_AUTH_TOKEN"
            },
        ),
        Tool::Gemini => ("/env/GOOGLE_GEMINI_BASE_URL", "/env/GEMINI_API_KEY"),
        Tool::OpenCode | Tool::Mcode => ("/options/baseURL", "/options/apiKey"),
        Tool::Hermes => (
            if config.get("base_url").is_some() || config.get("baseUrl").is_none() {
                "/base_url"
            } else {
                "/baseUrl"
            },
            if config.get("api_key").is_some() || config.get("apiKey").is_none() {
                "/api_key"
            } else {
                "/apiKey"
            },
        ),
        _ => (
            if config.get("baseURL").is_some() && config.get("baseUrl").is_none() {
                "/baseURL"
            } else {
                "/baseUrl"
            },
            "/apiKey",
        ),
    }
}
fn config_document(config: &Value) -> Result<DocumentMut, AppError> {
    config
        .get("config")
        .and_then(Value::as_str)
        .ok_or_else(unsupported)?
        .parse()
        .map_err(|_| unsupported())
}
fn table_path(tool: Tool, document: &DocumentMut) -> Result<(&'static str, String), AppError> {
    let (group, selected) = if tool == Tool::Codex {
        (
            "model_providers",
            document
                .get("model_provider")
                .and_then(toml_edit::Item::as_str),
        )
    } else {
        (
            "model",
            document
                .get("models")
                .and_then(|item| item.get("default"))
                .and_then(toml_edit::Item::as_str),
        )
    };
    let tables = document
        .get(group)
        .and_then(toml_edit::Item::as_table_like)
        .ok_or_else(unsupported)?;
    let name = match selected {
        Some(name) => name.to_owned(),
        None if tables.len() == 1 => tables.iter().next().unwrap().0.to_owned(),
        _ => return Err(unsupported()),
    };
    if tables
        .get(&name)
        .and_then(toml_edit::Item::as_table_like)
        .is_none()
    {
        return Err(unsupported());
    }
    Ok((group, name))
}
fn raw(tool: Tool, provider: &Provider) -> Result<(String, bool), AppError> {
    let config = &provider.settings_config;
    if matches!(tool, Tool::Codex | Tool::GrokBuild) {
        let document = config_document(config)?;
        let (group, name) = table_path(tool, &document)?;
        let table = &document[group][&name];
        let url = table
            .get("base_url")
            .and_then(toml_edit::Item::as_str)
            .unwrap_or_default()
            .to_owned();
        let key = if tool == Tool::Codex {
            config
                .pointer("/auth/OPENAI_API_KEY")
                .and_then(Value::as_str)
        } else {
            table
                .get("api_key")
                .and_then(toml_edit::Item::as_str)
                .or_else(|| table.get("env_key").and_then(toml_edit::Item::as_str))
        };
        return Ok((url, key.is_some_and(|key| !key.is_empty())));
    }
    if !config.is_object() {
        return Err(unsupported());
    }
    let (url, key) = json_paths(tool, config);
    Ok((
        config
            .pointer(url)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        config.pointer(key).is_some_and(|value| {
            !value.is_null() && value.as_str().is_none_or(|key| !key.is_empty())
        }),
    ))
}

pub fn connection_info(tool: Tool, provider: &Provider) -> ConnectionInfo {
    let uses_login = !tool.is_coexist()
        && (provider.is_official_category() || provider.meta.auth_binding.is_some());
    let advanced_auth = matches!(tool, Tool::Claude | Tool::ClaudeDesktop)
        && ["CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX"]
            .iter()
            .any(|key| {
                provider
                    .settings_config
                    .get("env")
                    .and_then(|env| env.get(*key))
                    .is_some_and(|value| {
                        matches!(value, Value::Bool(true))
                            || value
                                .as_str()
                                .is_some_and(|flag| matches!(flag, "1" | "true"))
                    })
            });
    let values = raw(tool, provider);
    let mut result = ConnectionInfo {
        base_url: None,
        api_key_set: false,
        editable: values.is_ok() && !uses_login && !advanced_auth,
        uses_login,
        url_hidden: false,
    };
    if let Ok((url, key)) = values {
        result.api_key_set = key;
        if !url.is_empty() {
            if let Ok(mut parsed) = reqwest::Url::parse(&url) {
                if matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() {
                    result.url_hidden = !parsed.username().is_empty()
                        || parsed.password().is_some()
                        || parsed.query().is_some()
                        || parsed.fragment().is_some();
                    let _ = parsed.set_username("");
                    let _ = parsed.set_password(None);
                    parsed.set_query(None);
                    parsed.set_fragment(None);
                    result.base_url = Some(parsed.to_string());
                } else {
                    result.url_hidden = true;
                }
            } else {
                result.url_hidden = true;
            }
        }
    }
    result
}

fn set_json(config: &mut Value, path: &str, value: Option<String>) -> Result<(), AppError> {
    let mut parts = path.trim_start_matches('/').split('/').peekable();
    let mut current = config.as_object_mut().ok_or_else(unsupported)?;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            if let Some(value) = value {
                current.insert(part.into(), Value::String(value));
            } else {
                current.remove(part);
            }
            return Ok(());
        }
        current = current
            .entry(part)
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .ok_or_else(unsupported)?;
    }
    Ok(())
}
fn patch(tool: Tool, provider: &mut Provider, update: ConnectionUpdate) -> Result<(), AppError> {
    if update.name.trim().is_empty() || update.name.chars().any(char::is_control) {
        return Err(invalid());
    }
    let changed_connection =
        update.base_url.is_some() || !matches!(update.api_key, ApiKeyChange::Keep);
    if changed_connection && !connection_info(tool, provider).editable {
        return Err(unsupported());
    }
    if let Some(url) = update.base_url.as_ref().filter(|url| !url.is_empty()) {
        let parsed = reqwest::Url::parse(url).map_err(|_| invalid())?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(invalid());
        }
    }
    if let ApiKeyChange::Set(key) = &update.api_key {
        if key.is_empty() || key.chars().any(char::is_control) {
            return Err(invalid());
        }
    }
    provider.name = update.name.trim().into();
    if !changed_connection {
        return Ok(());
    }
    if matches!(tool, Tool::Codex | Tool::GrokBuild) {
        let mut document = config_document(&provider.settings_config)?;
        let (group, name) = table_path(tool, &document)?;
        let table = document[group][&name]
            .as_table_like_mut()
            .ok_or_else(unsupported)?;
        if let Some(url) = update.base_url {
            if url.is_empty() {
                table.remove("base_url");
            } else {
                table.insert("base_url", toml_edit::value(url));
            }
        }
        if tool == Tool::GrokBuild {
            match update.api_key {
                ApiKeyChange::Keep => {}
                ApiKeyChange::Set(key) => {
                    table.insert("api_key", toml_edit::value(key));
                    table.remove("env_key");
                }
                ApiKeyChange::Clear => {
                    table.remove("api_key");
                    table.remove("env_key");
                }
            }
        } else {
            match update.api_key {
                ApiKeyChange::Keep => {}
                ApiKeyChange::Set(key) => set_json(
                    &mut provider.settings_config,
                    "/auth/OPENAI_API_KEY",
                    Some(key),
                )?,
                ApiKeyChange::Clear => {
                    set_json(&mut provider.settings_config, "/auth/OPENAI_API_KEY", None)?
                }
            }
        }
        provider.settings_config["config"] = Value::String(document.to_string());
    } else {
        let (url_path, key_path) = json_paths(tool, &provider.settings_config);
        if let Some(url) = update.base_url {
            if tool == Tool::Hermes {
                set_json(&mut provider.settings_config, "/base_url", None)?;
                set_json(&mut provider.settings_config, "/baseUrl", None)?;
            } else if matches!(tool, Tool::OpenClaw | Tool::Pi) {
                set_json(&mut provider.settings_config, "/baseUrl", None)?;
                set_json(&mut provider.settings_config, "/baseURL", None)?;
            }
            set_json(
                &mut provider.settings_config,
                url_path,
                (!url.is_empty()).then_some(url),
            )?;
        }
        if !matches!(update.api_key, ApiKeyChange::Keep)
            && matches!(tool, Tool::Claude | Tool::ClaudeDesktop)
        {
            set_json(
                &mut provider.settings_config,
                "/env/ANTHROPIC_AUTH_TOKEN",
                None,
            )?;
            set_json(
                &mut provider.settings_config,
                "/env/ANTHROPIC_API_KEY",
                None,
            )?;
        }
        if tool == Tool::Hermes && !matches!(update.api_key, ApiKeyChange::Keep) {
            set_json(&mut provider.settings_config, "/api_key", None)?;
            set_json(&mut provider.settings_config, "/apiKey", None)?;
        }
        match update.api_key {
            ApiKeyChange::Keep => {}
            ApiKeyChange::Set(key) => set_json(&mut provider.settings_config, key_path, Some(key))?,
            ApiKeyChange::Clear => set_json(&mut provider.settings_config, key_path, None)?,
        }
    }
    Ok(())
}

impl Service {
    pub fn update_connection(
        &self,
        tool: Tool,
        expected: &Provider,
        update: ConnectionUpdate,
    ) -> ApiResult<OpResult> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        self.settle(&rt, tool)?;
        let data = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        let stored = data
            .provider(tool, &expected.id)
            .ok_or_else(|| self.err(AppError::InvalidInput("Provider no longer exists".into())))?;
        if redact::redact(tool, stored) != *expected {
            return Err(self.err(AppError::localized(
                "conflict",
                "编辑期间卡片已变更，请重新打开编辑。",
                "The card changed while editing. Open the editor again.",
            )));
        }
        let mut next = stored.clone();
        patch(tool, &mut next, update).map_err(|e| self.err(e))?;
        self.check_incoming(&next)?;
        switch::update_provider_with(&rt.env, services(&rt), tool, next)
            .map(|report| self.op_result(report))
            .map_err(|e| self.err(e))
    }
}

#[cfg(test)]
mod tests;
