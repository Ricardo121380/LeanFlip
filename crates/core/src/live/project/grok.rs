//! A selected Grok model is replaced as one unit, while other models remain user-owned.
use crate::{
    error::AppError,
    live::patch::{
        toml::{TomlDocPatch, TomlPatch},
        KeyPath, LiveWriteError,
    },
};
use serde_json::Value;
use std::path::Path;
use toml_edit::{DocumentMut, Item, Table};
#[derive(Debug, Clone)]
pub struct GrokProjection {
    pub table: Option<(String, Table)>,
}
impl GrokProjection {
    pub fn of(settings: &Value, official: bool) -> Result<Self, AppError> {
        if official {
            return Ok(Self { table: None });
        }
        let invalid = || {
            AppError::Config("Grok Build 需要有效的模型表和 models.default / A valid Grok model table and selector are required".into())
        };
        let root = settings
            .get("config")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?
            .parse::<toml::Value>()
            .map_err(|_| invalid())?;
        let models = root
            .get("model")
            .and_then(toml::Value::as_table)
            .ok_or_else(invalid)?;
        let selected = root
            .get("models")
            .and_then(|v| v.get("default"))
            .and_then(toml::Value::as_str)
            .filter(|name| models.get(*name).is_some_and(toml::Value::is_table));
        let name = if let Some(name) = selected {
            name
        } else {
            let names: Vec<_> = models
                .iter()
                .filter(|(_, v)| v.is_table())
                .map(|(k, _)| k.as_str())
                .collect();
            if names.len() != 1 {
                return Err(invalid());
            }
            names[0]
        };
        let source = toml::to_string(&models[name]).map_err(|_| invalid())?;
        let table = source
            .parse::<DocumentMut>()
            .map_err(|_| invalid())?
            .as_table()
            .clone();
        crate::grok_config::validate_config_toml(&normalized_text(name, &table))?;
        Ok(Self {
            table: Some((name.into(), table)),
        })
    }
    pub fn table_name(&self) -> Option<&str> {
        self.table.as_ref().map(|(name, _)| name.as_str())
    }
    pub fn written_tables(&self) -> Vec<String> {
        self.table_name().map(str::to_string).into_iter().collect()
    }
}
pub(crate) fn normalized_text(name: &str, table: &Table) -> String {
    let mut root = DocumentMut::new();
    let mut selector = Table::new();
    selector.insert("default", toml_edit::value(name));
    let mut models = Table::new();
    models.set_implicit(true);
    models.insert(name, Item::Table(table.clone()));
    root.insert("models", Item::Table(selector));
    root.insert("model", Item::Table(models));
    root.to_string()
}
#[derive(Debug, Clone, Default)]
pub struct GrokConfigPatch {
    pub target: Option<(String, Table)>,
    pub retired: Vec<String>,
    pub placeholder: Option<String>,
}
impl GrokConfigPatch {
    pub fn direct(target: &GrokProjection, retired: Vec<String>, placeholder: &str) -> Self {
        Self {
            target: target.table.clone(),
            retired,
            placeholder: Some(placeholder.into()),
        }
    }
    pub fn apply_to(&self, path: &Path, doc: &mut DocumentMut) -> Result<(), LiveWriteError> {
        let mut edit = TomlPatch::default();
        let root = KeyPath::new(&["model"]);
        edit.remove.push(KeyPath::new(&["models", "default"]));
        edit.remove
            .extend(self.retired.iter().map(|name| root.child(name)));
        if let Some(models) = doc.get("model").and_then(Item::as_table_like) {
            for (name, model) in models.iter() {
                if self
                    .placeholder
                    .as_deref()
                    .is_some_and(|p| model.get("api_key").and_then(Item::as_str) == Some(p))
                {
                    edit.remove.push(root.child(name));
                }
            }
        }
        if let Some((name, table)) = &self.target {
            edit.set
                .push((KeyPath::new(&["models", "default"]), toml_edit::value(name)));
            let inline = matches!(doc.get("model"), Some(Item::Value(_)));
            let entry = if inline {
                Item::Value(toml_edit::Value::InlineTable(
                    table.clone().into_inline_table(),
                ))
            } else {
                Item::Table(table.clone())
            };
            edit.set.push((root.child(name), entry));
        }
        edit.apply_to(path, doc)?;
        for key in ["models", "model"] {
            if doc
                .get(key)
                .and_then(Item::as_table_like)
                .is_some_and(|t| t.is_empty())
            {
                doc.remove(key);
            }
        }
        Ok(())
    }
}
impl TomlDocPatch for GrokConfigPatch {
    fn apply_to(&self, path: &Path, doc: &mut DocumentMut) -> Result<(), LiveWriteError> {
        self.apply_to(path, doc)
    }
}
