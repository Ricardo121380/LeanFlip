//! Convert stored connection settings into a narrow Codex configuration edit.
use crate::{
    error::AppError,
    live::{
        floor,
        patch::{
            toml::{TomlClearScope, TomlDocPatch, TomlPatch},
            KeyPath, LiveWriteError,
        },
    },
};
use serde_json::Value;
use std::path::Path;
use toml_edit::{DocumentMut, Item, Table, Value as Atom};
pub const ROUTE_ID: &str = "custom";
// These strings identify imported data, not a dependency on another implementation.
pub const OFFICIAL_PROXY_ROUTE_ID: &str = "cc-switch-official";
pub const CATALOG_FILENAME: &str = "cc-switch-model-catalog.json";
pub const MODEL_CATALOG_JSON: &str = "model_catalog_json";
pub use super::claude::PROXY_TOKEN_PLACEHOLDER;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteAuth {
    EnvKey,
    Bearer,
    Headers,
    None,
}
pub fn requires_openai_auth(auth: RouteAuth, login_on_disk: bool) -> bool {
    login_on_disk && matches!(auth, RouteAuth::EnvKey | RouteAuth::Bearer)
}
#[derive(Debug, Clone)]
pub enum Route {
    Official,
    Custom { table: Table, auth: RouteAuth },
    BuiltIn { id: String, table: Option<Table> },
    Default,
}
#[derive(Debug, Clone)]
pub enum RouteWrite {
    Official,
    Custom(Table),
    BuiltIn { id: String, table: Option<Table> },
    Default,
}
#[derive(Debug, Clone)]
pub struct CodexProjection {
    pub route: Route,
    pub top: Vec<(String, Atom)>,
    pub nested: Vec<(Vec<String>, Atom)>,
    pub exclusive: Vec<(String, Atom)>,
}
pub struct RowInput<'a> {
    pub settings: &'a Value,
    pub official: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownTable {
    pub id: String,
    pub base_url: String,
}
#[derive(Debug, Clone)]
pub struct CodexConfigPatch {
    pub top: Vec<(String, Atom)>,
    pub nested: Vec<(Vec<String>, Atom)>,
    pub exclusive: Vec<(String, Atom)>,
    pub outgoing: Vec<(String, Atom)>,
    pub route: RouteWrite,
    pub retired: Vec<KnownTable>,
}
fn invalid(message: &str) -> AppError {
    AppError::localized(
        "provider.codex.config.invalid",
        format!("Codex 配置无效：{message}"),
        format!("Invalid Codex configuration: {message}"),
    )
}
fn fallback_error() -> AppError {
    AppError::localized("provider.codex.config.official_auth_fallback","第三方地址没有独立凭据；请填写密钥或指定鉴权方式，避免发送官方登录凭据", "A third-party endpoint requires its own authentication; refusing to forward the official login")
}
pub fn is_keyless_fallback(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Localized {
            key: "provider.codex.config.official_auth_fallback",
            ..
        }
    )
}
fn text<'a>(root: &'a toml::Value, key: &str) -> Option<&'a str> {
    root.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
fn as_table(value: &toml::Value) -> Result<Table, AppError> {
    let encoded = toml::to_string(value).map_err(|_| invalid("provider table"))?;
    encoded
        .parse::<DocumentMut>()
        .map(|doc| doc.as_table().clone())
        .map_err(|_| invalid("provider table"))
}
fn atoms(root: &toml::Value, keys: &[&str]) -> Vec<(String, Atom)> {
    keys.iter()
        .filter_map(|key| {
            let value = root.get(*key)?;
            let wrapper = toml::Value::Table(
                [(String::from("value"), value.clone())]
                    .into_iter()
                    .collect(),
            );
            let table = as_table(&wrapper).ok()?;
            Some((key.to_string(), table.get("value")?.as_value()?.clone()))
        })
        .collect()
}
impl CodexProjection {
    pub fn of(input: &RowInput<'_>) -> Result<Self, AppError> {
        let settings = input
            .settings
            .as_object()
            .ok_or_else(|| invalid("expected object"))?;
        let config = settings
            .get("config")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let parsed = config
            .parse::<toml::Value>()
            .map_err(|_| invalid("TOML syntax"))?;
        let mut result = Self {
            route: Route::Default,
            top: atoms(
                &parsed,
                &[
                    "model",
                    "review_model",
                    "model_reasoning_effort",
                    "plan_mode_reasoning_effort",
                    "disable_response_storage",
                ],
            ),
            nested: vec![],
            exclusive: atoms(&parsed, floor::CODEX_EXCLUSIVE_TOP),
        };
        if let Some(pointer) = text(&parsed, MODEL_CATALOG_JSON)
            .filter(|p| Path::new(p).file_name().and_then(|x| x.to_str()) != Some(CATALOG_FILENAME))
        {
            result.top.push((MODEL_CATALOG_JSON.into(), pointer.into()));
        }
        for path in floor::CODEX_FLOOR_NESTED {
            let mut parent = Some(&parsed);
            for key in &path[..path.len() - 1] {
                parent = parent.and_then(|v| v.get(*key));
            }
            if let Some(parent) = parent {
                for (_, value) in atoms(parent, &path[path.len() - 1..]) {
                    result
                        .nested
                        .push((path.iter().map(|s| s.to_string()).collect(), value));
                }
            }
        }
        if input.official {
            result.route = Route::Official;
            return Ok(result);
        }
        let selector = text(&parsed, "model_provider");
        let selected =
            selector.and_then(|id| parsed.get("model_providers").and_then(|p| p.get(id)));
        let bearer = settings
            .get("auth")
            .and_then(|v| v.get("OPENAI_API_KEY"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| selected.and_then(|t| text(t, "experimental_bearer_token")))
            .or_else(|| text(&parsed, "experimental_bearer_token"));
        let legacy_endpoint = text(&parsed, "openai_base_url").or_else(|| {
            if selector.is_none() {
                text(&parsed, "base_url")
            } else {
                None
            }
        });
        let builtin = selector.is_some_and(|id| {
            [
                "amazon-bedrock",
                "amazon-bedrock-runtime",
                "ollama",
                "lmstudio",
            ]
            .contains(&id)
        });
        if builtin
            && (selected.is_none() || selector.is_some_and(|id| id.starts_with("amazon-bedrock")))
        {
            result.route = Route::BuiltIn {
                id: selector.unwrap().into(),
                table: selected.map(as_table).transpose()?,
            };
            return Ok(result);
        }
        let mut route = if let Some(value) = selected {
            if !value.is_table() {
                return Err(invalid("model_providers entry must be a table"));
            }
            as_table(value)?
        } else if let Some(endpoint) = legacy_endpoint {
            let mut table = Table::new();
            table.insert("base_url", toml_edit::value(endpoint));
            table
        } else {
            if let Some(id) = selector {
                if id == "openai" {
                    return Err(fallback_error());
                }
                return Err(AppError::localized(
                    "provider.codex.config.no_custom_provider",
                    "所选供应商缺少对应配置表",
                    "Selected provider has no configuration table",
                ));
            }
            if bearer.is_some() && parsed.as_table().is_some_and(|p| p.is_empty()) {
                return Err(invalid("an API key needs an endpoint"));
            }
            return Ok(result);
        };
        if route
            .get("name")
            .and_then(Item::as_str)
            .is_none_or(|s| s.trim().is_empty())
        {
            route.insert("name", toml_edit::value(selector.unwrap_or("Custom")));
        }
        if !route.contains_key("wire_api")
            || selector.is_some_and(|id| ["openai", "ollama", "lmstudio"].contains(&id))
        {
            route.insert("wire_api", toml_edit::value("responses"));
        }
        let has_header = ["http_headers", "env_http_headers"].iter().any(|field| {
            route
                .get(field)
                .and_then(Item::as_table_like)
                .is_some_and(|headers| {
                    headers
                        .iter()
                        .any(|(key, _)| key.eq_ignore_ascii_case("authorization"))
                })
        });
        let auth = if route.contains_key("auth") || route.contains_key("aws") {
            if route.contains_key("env_key") || route.contains_key("experimental_bearer_token") {
                return Err(invalid(
                    "command/AWS authentication cannot be combined with another token source",
                ));
            }
            RouteAuth::Headers
        } else if route.contains_key("env_key") {
            if route
                .get("env_key")
                .and_then(Item::as_str)
                .is_none_or(|s| s.trim().is_empty())
            {
                return Err(invalid("env_key must name a variable"));
            }
            route.remove("experimental_bearer_token");
            RouteAuth::EnvKey
        } else if has_header
            && !route
                .get("requires_openai_auth")
                .and_then(Item::as_bool)
                .unwrap_or(false)
        {
            route.remove("experimental_bearer_token");
            RouteAuth::Headers
        } else if let Some(token) = bearer {
            route.insert("experimental_bearer_token", toml_edit::value(token));
            RouteAuth::Bearer
        } else {
            if route.get("requires_openai_auth").and_then(Item::as_bool) == Some(true)
                || legacy_endpoint.is_some()
            {
                return Err(fallback_error());
            }
            if has_header || route.contains_key("query_params") {
                RouteAuth::Headers
            } else {
                RouteAuth::None
            }
        };
        route.remove("requires_openai_auth");
        result.route = Route::Custom { table: route, auth };
        Ok(result)
    }
}
pub fn foreign_catalog(doc: &DocumentMut) -> Option<&Atom> {
    doc.get(MODEL_CATALOG_JSON)?.as_value().filter(|v| {
        v.as_str().is_some_and(|s| {
            Path::new(s).file_name().and_then(|s| s.to_str()) != Some(CATALOG_FILENAME)
        })
    })
}
impl CodexConfigPatch {
    pub fn apply_to(&self, path: &Path, doc: &mut DocumentMut) -> Result<(), LiveWriteError> {
        let (selector, table) = match &self.route {
            RouteWrite::Custom(table) => (Some(ROUTE_ID), Some((ROUTE_ID, table))),
            RouteWrite::BuiltIn { id, table } => (
                Some(id.as_str()),
                table.as_ref().map(|table| (id.as_str(), table)),
            ),
            _ => (None, None),
        };
        let mut edit = TomlPatch {
            clear: vec![TomlClearScope {
                parent: KeyPath::root(),
                is_floor: |k| floor::CODEX_FLOOR_TOP.contains(&k),
            }],
            remove: floor::CODEX_FLOOR_NESTED
                .iter()
                .map(|p| KeyPath::new(p))
                .collect(),
            ..Default::default()
        };
        for (key, value) in self.top.iter().chain(&self.exclusive) {
            edit.set
                .push((KeyPath::root().child(key), Item::Value(value.clone())));
        }
        for (keys, value) in &self.nested {
            edit.set
                .push((KeyPath(keys.clone()), Item::Value(value.clone())));
        }
        for (key, value) in &self.outgoing {
            edit.remove_if
                .push((KeyPath::root().child(key), vec![value.clone()]));
        }
        if let Some(id) = selector {
            edit.set
                .push((KeyPath::new(&["model_provider"]), toml_edit::value(id)));
        }
        let referenced: Vec<String> = doc
            .get("profiles")
            .and_then(Item::as_table_like)
            .into_iter()
            .flat_map(|p| p.iter())
            .filter_map(|(_, p)| p.get("model_provider")?.as_str().map(str::to_string))
            .collect();
        let provider_path = KeyPath::new(&["model_providers"]);
        if let Some(providers) = doc.get("model_providers").and_then(Item::as_table_like) {
            let mut reserved = Vec::new();
            for (id, entry) in providers.iter() {
                let placeholder = entry
                    .get("experimental_bearer_token")
                    .and_then(Item::as_str)
                    == Some(PROXY_TOKEN_PLACEHOLDER);
                let retired = self.retired.iter().any(|old| {
                    old.id == id
                        && entry.get("base_url").and_then(Item::as_str)
                            == Some(old.base_url.as_str())
                });
                if ["openai", "ollama", "lmstudio"].contains(&id) {
                    if !placeholder && !retired {
                        reserved.push(entry.clone());
                    }
                    edit.remove.push(provider_path.child(id));
                } else if id != ROUTE_ID
                    && !referenced.iter().any(|r| r == id)
                    && (placeholder || retired || id == OFFICIAL_PROXY_ROUTE_ID)
                {
                    edit.remove.push(provider_path.child(id));
                }
            }
            for (index, entry) in reserved.into_iter().enumerate() {
                let mut suffix = index + 1;
                let name = loop {
                    let name = format!("leanflip-legacy-{suffix}");
                    if !providers.contains_key(&name)
                        && !edit
                            .set
                            .iter()
                            .any(|(p, _)| p == &provider_path.child(&name))
                    {
                        break name;
                    }
                    suffix += 1;
                };
                edit.set.push((provider_path.child(&name), entry));
            }
        }
        if let Some((id, table)) = table {
            let inline = matches!(doc.get("model_providers"), Some(Item::Value(_)))
                || matches!(
                    doc.get("model_providers").and_then(|p| p.get(id)),
                    Some(Item::Value(_))
                );
            let entry = if inline {
                Item::Value(Atom::InlineTable(table.clone().into_inline_table()))
            } else {
                Item::Table(table.clone())
            };
            edit.set.push((provider_path.child(id), entry));
        } else {
            edit.remove.push(
                provider_path
                    .child(ROUTE_ID)
                    .child("experimental_bearer_token"),
            );
        }
        edit.apply_to(path, doc)?;
        if doc
            .get("model_providers")
            .and_then(Item::as_table_like)
            .is_some_and(|p| p.is_empty())
        {
            doc.remove("model_providers");
        }
        if let Some(name) = doc.get("profile").and_then(Item::as_str) {
            if let Some(profile) = doc.get("profiles").and_then(|p| p.get(name)) {
                for key in [
                    "model_provider",
                    "openai_base_url",
                    "experimental_bearer_token",
                ] {
                    if let Some(value) = profile
                        .get(key)
                        .and_then(Item::as_str)
                        .filter(|s| !s.trim().is_empty())
                    {
                        if key != "model_provider" || value != selector.unwrap_or("openai") {
                            return Err(LiveWriteError::Route {
                                profile: name.into(),
                                key: key.into(),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
impl TomlDocPatch for CodexConfigPatch {
    fn apply_to(&self, path: &Path, doc: &mut DocumentMut) -> Result<(), LiveWriteError> {
        self.apply_to(path, doc)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_auth_and_environment_token_are_not_silently_combined() {
        let settings = serde_json::json!({"auth":{},"config":"model_provider='vendor'\n[model_providers.vendor]\nbase_url='https://example.invalid'\nenv_key='TOKEN'\nauth={command='credential-helper'}\n"});
        assert!(CodexProjection::of(&RowInput {
            settings: &settings,
            official: false
        })
        .is_err());
    }
    #[test]
    fn a_third_party_route_cannot_inherit_the_official_token() {
        let settings = serde_json::json!({"auth":{},"config":"model_provider='vendor'\n[model_providers.vendor]\nbase_url='https://example.invalid'\nrequires_openai_auth=true\n"});
        assert!(is_keyless_fallback(
            &CodexProjection::of(&RowInput {
                settings: &settings,
                official: false
            })
            .unwrap_err()
        ));
    }
}
