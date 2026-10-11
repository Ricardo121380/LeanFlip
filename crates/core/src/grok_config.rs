//! Validation of the public Grok Build configuration format.
use crate::error::AppError;
fn invalid(field: &str) -> AppError {
    AppError::localized(
        "provider.grokbuild.config.invalid",
        format!("Grok Build 配置字段无效：{field}"),
        format!("Invalid Grok Build configuration field: {field}"),
    )
}
pub fn is_official_live_config(source: &str) -> bool {
    source.parse::<toml::Table>().is_ok_and(|root| {
        !["models", "model"]
            .iter()
            .any(|key| root.contains_key(*key))
    })
}
pub fn validate_config_toml(source: &str) -> Result<(), AppError> {
    let root = source
        .parse::<toml::Value>()
        .map_err(|_| invalid("TOML syntax"))?;
    let name = root
        .get("models")
        .and_then(|v| v.get("default"))
        .and_then(toml::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| invalid("models.default"))?;
    let model = root
        .get("model")
        .and_then(|v| v.get(name))
        .and_then(toml::Value::as_table)
        .ok_or_else(|| invalid("model.<selected>"))?;
    let string = |key: &str| {
        model
            .get(key)
            .and_then(toml::Value::as_str)
            .filter(|s| !s.trim().is_empty())
    };
    for key in ["name", "model", "base_url", "api_backend"] {
        if string(key).is_none() {
            return Err(invalid(key));
        }
    }
    if string("api_key").is_none() && string("env_key").is_none() {
        return Err(invalid("api_key / env_key"));
    }
    if model
        .get("context_window")
        .and_then(toml::Value::as_integer)
        .is_none_or(|n| n <= 0)
    {
        return Err(invalid("context_window"));
    }
    Ok(())
}
