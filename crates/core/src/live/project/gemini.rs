//! Gemini connection values span a dotenv file and a JSON settings file.
use crate::live::{
    floor,
    patch::{dotenv::DotenvPatch, json::JsonPatch, KeyPath},
};
use serde_json::Value;
pub const SELECTED_TYPE_OAUTH: &str = "oauth-personal";
pub const SELECTED_TYPE_API_KEY: &str = "gemini-api-key";
pub const BASE_URL_ENV: &str = "GOOGLE_GEMINI_BASE_URL";
pub const API_KEY_ENV: &str = "GEMINI_API_KEY";
#[derive(Debug, Clone, PartialEq)]
pub struct GeminiProjection {
    pub env: Vec<(String, String)>,
    pub selected_type: Option<&'static str>,
    pub model_name: Option<Value>,
}
impl GeminiProjection {
    pub fn empty() -> Self {
        Self {
            env: vec![],
            selected_type: None,
            model_name: None,
        }
    }
    pub fn of(settings: &Value, official: bool) -> Self {
        let mut selected = Self::empty();
        if let Some(values) = settings.get("env").and_then(Value::as_object) {
            for (key, value) in values {
                if floor::gemini_floor_env(key) {
                    if let Some(text) = value.as_str() {
                        selected.env.push((key.clone(), text.into()));
                    }
                }
            }
        }
        selected.model_name = settings
            .pointer("/config/model/name")
            .filter(|v| !v.is_null())
            .cloned();
        selected.selected_type = Some(
            match settings
                .pointer("/config/security/auth/selectedType")
                .and_then(Value::as_str)
            {
                Some("vertex-ai") => "vertex-ai",
                Some(SELECTED_TYPE_API_KEY) => SELECTED_TYPE_API_KEY,
                Some(SELECTED_TYPE_OAUTH) => SELECTED_TYPE_OAUTH,
                _ if official => SELECTED_TYPE_OAUTH,
                _ => SELECTED_TYPE_API_KEY,
            },
        );
        selected
    }
    pub fn env_patch(&self) -> DotenvPatch {
        DotenvPatch {
            clear: Some(floor::gemini_floor_env),
            set: self.env.clone(),
            ..Default::default()
        }
    }
    pub fn settings_patch(&self) -> JsonPatch {
        let mut result = JsonPatch::default();
        for (key, value) in [
            (selected_type_path(), self.selected_type.map(Value::from)),
            (model_name_path(), self.model_name.clone()),
        ] {
            if let Some(value) = value {
                result.set.push((key, value));
            } else {
                result.remove.push(key);
            }
        }
        result
    }
}
pub fn selected_type_path() -> KeyPath {
    KeyPath::new(&["security", "auth", "selectedType"])
}
pub fn model_name_path() -> KeyPath {
    KeyPath::new(&["model", "name"])
}
