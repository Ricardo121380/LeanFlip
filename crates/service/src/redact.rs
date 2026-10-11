//! 不把从文件里来的官方登录凭据交给前端。
//!
//! 卡片里可能带着 ChatGPT 登录（老版本 cc-switch 把 `auth.json` 抄进了官方卡，或者用户
//! 粘贴了整份 `auth.json`）。发给前端之前把这些值换成占位符；前端原样带回占位符时，
//! 用库里的旧值还原。第三方 API Key 是用户自己在卡片里填的，照常显示（和 cc-switch 一样）。

use ccsl_core::{Provider, Tool};
use serde_json::Value;

pub const PLACEHOLDER: &str = "__ccsl_redacted__";

/// `auth` 里属于 ChatGPT 登录的键。
const LOGIN_KEYS: &[&str] = &[
    "tokens",
    "id_token",
    "access_token",
    "refresh_token",
    "last_refresh",
];

fn auth_mut(tool: Tool, provider: &mut Provider) -> Option<&mut serde_json::Map<String, Value>> {
    if tool != Tool::Codex {
        return None;
    }
    provider
        .settings_config
        .get_mut("auth")
        .and_then(Value::as_object_mut)
}

/// 发给前端的副本。
pub fn redact(tool: Tool, provider: &Provider) -> Provider {
    let mut copy = provider.clone();
    if let Some(auth) = auth_mut(tool, &mut copy) {
        for key in LOGIN_KEYS {
            if let Some(value) = auth.get_mut(*key) {
                *value = Value::String(PLACEHOLDER.into());
            }
        }
    }
    copy
}

/// 前端带回来的卡：占位符换回库里的值；库里没有就去掉这个键（不会把占位符写进文件）。
pub fn restore(tool: Tool, incoming: &mut Provider, stored: Option<&Provider>) {
    let stored_auth = stored
        .and_then(|p| p.settings_config.get("auth"))
        .and_then(Value::as_object)
        .cloned();
    if let Some(auth) = auth_mut(tool, incoming) {
        for key in LOGIN_KEYS {
            if auth.get(*key).and_then(Value::as_str) == Some(PLACEHOLDER) {
                match stored_auth.as_ref().and_then(|s| s.get(*key)) {
                    Some(original) => {
                        auth.insert((*key).to_string(), original.clone());
                    }
                    None => {
                        auth.shift_remove(*key);
                    }
                }
            }
        }
    }
}

/// 序列化后的任意值里有没有占位符（防呆：写盘前检查）。
pub fn contains_placeholder(value: &Value) -> bool {
    match value {
        Value::String(s) => s == PLACEHOLDER,
        Value::Array(items) => items.iter().any(contains_placeholder),
        Value::Object(map) => map.values().any(contains_placeholder),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn login_values_never_leave_and_come_back_intact() {
        let stored = Provider::new(
            "official",
            "OpenAI",
            json!({"auth": {"auth_mode": "chatgpt", "OPENAI_API_KEY": null,
                             "tokens": {"refresh_token": "rt-secret", "id_token": "jwt"}},
                   "config": "model = \"gpt-5\"\n"}),
        );
        let sent = redact(Tool::Codex, &stored);
        let text = serde_json::to_string(&sent).unwrap();
        assert!(
            !text.contains("rt-secret") && !text.contains("jwt"),
            "{text}"
        );
        assert_eq!(sent.settings_config["auth"]["auth_mode"], "chatgpt");

        let mut back = sent.clone();
        back.settings_config["config"] = json!("model = \"gpt-5.5\"\n");
        restore(Tool::Codex, &mut back, Some(&stored));
        assert_eq!(back.settings_config["auth"], stored.settings_config["auth"]);
        assert!(!contains_placeholder(&back.settings_config));

        // 库里没有对应值：去掉，不写占位符。
        let mut fresh = sent;
        restore(Tool::Codex, &mut fresh, None);
        assert!(fresh.settings_config["auth"].get("tokens").is_none());
    }

    #[test]
    fn other_tools_are_untouched() {
        let card = Provider::new("a", "a", json!({"auth": {"tokens": "x"}}));
        assert_eq!(redact(Tool::Claude, &card), card);
    }
}
