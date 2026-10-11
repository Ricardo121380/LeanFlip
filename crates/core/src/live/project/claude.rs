//! Claude settings selection and merge policy.
use crate::live::{
    floor,
    patch::{
        json::{ClearScope, JsonPatch},
        KeyPath, LiveWriteError,
    },
};
use serde_json::{Map, Value};
use std::path::Path;

pub const PROXY_TOKEN_PLACEHOLDER: &str = "PROXY_MANAGED";

pub fn ignored_keys(settings: &Value) -> Vec<String> {
    let mut ignored = Vec::new();
    if let Some(fields) = settings.as_object() {
        for (key, value) in fields {
            if key == "env" {
                if let Some(env) = value.as_object() {
                    ignored.extend(
                        env.keys()
                            .filter(|key| {
                                !floor::claude_floor_env(key) && !floor::claude_exclusive_env(key)
                            })
                            .map(|key| format!("env.{key}")),
                    );
                } else {
                    ignored.push(key.clone());
                }
            } else if !floor::claude_floor_top(key) {
                ignored.push(key.clone());
            }
        }
    }
    ignored.sort();
    ignored
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClaudeProjection {
    pub top: Map<String, Value>,
    pub env: Map<String, Value>,
    pub exclusive: Map<String, Value>,
}
impl ClaudeProjection {
    pub fn of(settings: &Value) -> Self {
        let mut result = Self::default();
        for (source, target, predicate) in [
            (
                settings.as_object(),
                &mut result.top,
                floor::claude_floor_top as fn(&str) -> bool,
            ),
            (
                settings.get("env").and_then(Value::as_object),
                &mut result.env,
                floor::claude_floor_env,
            ),
            (
                settings.get("env").and_then(Value::as_object),
                &mut result.exclusive,
                floor::claude_exclusive_env,
            ),
        ] {
            if let Some(source) = source {
                target.extend(
                    source
                        .iter()
                        .filter(|(k, _)| predicate(k))
                        .map(|(k, v)| (k.clone(), v.clone())),
                );
            }
        }
        let bedrock = result.env.get("CLAUDE_CODE_USE_BEDROCK").is_some_and(|v| {
            ["true", "1", "yes", "on"].contains(
                &v.as_str()
                    .unwrap_or(if v == &Value::Bool(true) { "true" } else { "" })
                    .to_ascii_lowercase()
                    .as_str(),
            )
        });
        if bedrock {
            if let Some(token) = result.top.shift_remove("apiKey") {
                result
                    .env
                    .entry("AWS_BEARER_TOKEN_BEDROCK")
                    .or_insert(token);
            }
        }
        result
    }
    fn values(&self) -> Vec<(KeyPath, Value)> {
        let mut values = Vec::new();
        for (prefix, fields) in [
            (KeyPath::root(), &self.top),
            (KeyPath::new(&["env"]), &self.env),
            (KeyPath::new(&["env"]), &self.exclusive),
        ] {
            values.extend(
                fields
                    .iter()
                    .map(|(key, value)| (prefix.child(key), value.clone())),
            );
        }
        values
    }
}
pub fn direct_patch(previous: Option<&ClaudeProjection>, next: &ClaudeProjection) -> JsonPatch {
    let mut patch = JsonPatch {
        clear: vec![
            ClearScope {
                parent: KeyPath::root(),
                is_floor: floor::claude_floor_top,
            },
            ClearScope {
                parent: KeyPath::new(&["env"]),
                is_floor: floor::claude_floor_env,
            },
        ],
        set: next.values(),
        ..Default::default()
    };
    // A familiar numeric value is not evidence that LeanFlip owns a setting.
    // Only a recorded outgoing value may be removed, and only if unchanged.
    if let Some(previous) = previous {
        patch.remove_if.extend(
            previous
                .exclusive
                .iter()
                .map(|(key, value)| (KeyPath::new(&["env"]).child(key), vec![value.clone()])),
        );
    }
    patch
}
pub fn project_onto(
    path: &Path,
    live: &Value,
    previous: Option<&ClaudeProjection>,
    next: &ClaudeProjection,
) -> Result<Value, LiveWriteError> {
    let mut result = live.clone();
    direct_patch(previous, next).apply_to(path, &mut result)?;
    Ok(result)
}
pub fn store_into_row(row: &Value, projection: &ClaudeProjection) -> Value {
    let mut result = row.as_object().cloned().unwrap_or_default();
    result.retain(|key, _| !floor::claude_floor_top(key));
    result.extend(projection.top.clone());
    let mut env = result
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    env.retain(|key, _| !floor::claude_floor_env(key) && !floor::claude_exclusive_env(key));
    env.extend(projection.env.clone());
    env.extend(projection.exclusive.clone());
    if !env.is_empty() || result.contains_key("env") {
        result.insert("env".into(), Value::Object(env));
    }
    Value::Object(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unowned_limits_survive_and_edited_outgoing_limits_survive() {
        let old = ClaudeProjection::of(&json!({"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":"50"}}));
        let live = json!({"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":"262144","ANTHROPIC_AUTH_TOKEN":"before","API_TIMEOUT_MS":"99"},"hooks":{"Stop":[]}});
        let target = ClaudeProjection::of(&json!({"env":{"ANTHROPIC_AUTH_TOKEN":"after"}}));
        for previous in [None, Some(&old)] {
            let output =
                project_onto(Path::new("settings.json"), &live, previous, &target).unwrap();
            assert_eq!(output["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "262144");
            assert_eq!(output["hooks"], live["hooks"]);
            assert_eq!(output["env"]["ANTHROPIC_AUTH_TOKEN"], "after");
        }
    }
}
