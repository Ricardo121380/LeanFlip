//! 供应商（一张「卡」）的数据模型。
//!
//! 字段名和 cc-switch 的 `Provider` 对齐（camelCase 序列化），`settingsConfig` 的形状也
//! 一样：Claude 是 `settings.json` 的片段，Codex 是 `{auth, config}`。这样以后可以直接
//! 从 cc-switch 导入行。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::AppError;

/// 支持切换的工具。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tool {
    Claude,
    Codex,
    Gemini,
    GrokBuild,
    ClaudeDesktop,
    OpenCode,
    OpenClaw,
    Hermes,
    Pi,
    Mcode,
}

impl Tool {
    pub const ALL: [Tool; 10] = [
        Tool::Claude,
        Tool::Codex,
        Tool::Gemini,
        Tool::GrokBuild,
        Tool::ClaudeDesktop,
        Tool::OpenCode,
        Tool::OpenClaw,
        Tool::Hermes,
        Tool::Pi,
        Tool::Mcode,
    ];

    /// 并存式工具：多家供应商同时写在客户端配置里，没有「当前供应商」，
    /// 只有「写进 / 移出客户端配置」（见 [`crate::coexist`]）。
    pub fn is_coexist(self) -> bool {
        matches!(
            self,
            Tool::OpenCode | Tool::OpenClaw | Tool::Hermes | Tool::Pi | Tool::Mcode
        )
    }

    /// 存储和写锁用的名字（和 cc-switch 的 `AppType::as_str` 一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            Tool::Claude => "claude",
            Tool::Codex => "codex",
            Tool::Gemini => "gemini",
            Tool::GrokBuild => "grokbuild",
            Tool::ClaudeDesktop => "claude-desktop",
            Tool::OpenCode => "opencode",
            Tool::OpenClaw => "openclaw",
            Tool::Hermes => "hermes",
            Tool::Pi => "pi",
            Tool::Mcode => "mcode",
        }
    }
}

impl std::str::FromStr for Tool {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "claude" => Ok(Tool::Claude),
            "codex" => Ok(Tool::Codex),
            "gemini" => Ok(Tool::Gemini),
            "grokbuild" => Ok(Tool::GrokBuild),
            "claude-desktop" => Ok(Tool::ClaudeDesktop),
            "opencode" => Ok(Tool::OpenCode),
            "openclaw" => Ok(Tool::OpenClaw),
            "hermes" => Ok(Tool::Hermes),
            "pi" => Ok(Tool::Pi),
            "mcode" => Ok(Tool::Mcode),
            other => Err(AppError::InvalidInput(format!("未知的工具: {other}"))),
        }
    }
}

impl std::fmt::Display for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 官方卡的 `category`。
pub const CATEGORY_OFFICIAL: &str = "official";
/// 托管 OAuth 绑定的提供方（和 cc-switch 的 `meta.authBinding.authProvider` 一致）。
pub const AUTH_PROVIDER_CODEX_OAUTH: &str = "codex_oauth";

impl Serialize for Tool {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Tool {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse()
            .map_err(|_| serde::de::Error::custom(format!("unknown tool: {text}")))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// `"official"` 表示官方卡：Codex 走内置 openai 和它自己的登录，Claude 清空关键字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub settings_config: Value,
    #[serde(default, skip_serializing_if = "ProviderMeta::is_empty")]
    pub meta: ProviderMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i64>,
    /// 毫秒时间戳。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    /// 认不出的字段原样保留（从 cc-switch 导入的行、更新版本写的字段）。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMeta {
    /// 绑定的托管账号（Phase 2：Codex 多 ChatGPT 账号）。有值的 Codex 官方卡切换时写
    /// 托管账号的登录，而不是用户自己的原生登录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_binding: Option<AuthBinding>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ProviderMeta {
    pub fn is_empty(&self) -> bool {
        self.auth_binding.is_none() && self.extra.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthBinding {
    /// 目前只有 [`AUTH_PROVIDER_CODEX_OAUTH`]。
    pub auth_provider: String,
    /// 托管账号 id（ChatGPT account id）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

impl Provider {
    pub fn new(id: impl Into<String>, name: impl Into<String>, settings_config: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            category: None,
            settings_config,
            meta: ProviderMeta::default(),
            sort_index: None,
            created_at: Some(now_millis()),
            extra: Map::new(),
        }
    }

    pub fn official(mut self) -> Self {
        self.category = Some(CATEGORY_OFFICIAL.to_string());
        self
    }

    pub fn is_official_category(&self) -> bool {
        self.category.as_deref() == Some(CATEGORY_OFFICIAL)
    }

    /// 绑定的 Codex 托管账号 id。
    pub fn managed_codex_account(&self) -> Option<&str> {
        self.meta
            .auth_binding
            .as_ref()
            .filter(|binding| binding.auth_provider == AUTH_PROVIDER_CODEX_OAUTH)
            .map(|binding| binding.account_id.as_deref().unwrap_or(""))
    }

    /// Codex 官方卡引用的那份原生登录（登录暂存里的身份，`sub:<用户>|account:<workspace>`）。
    /// 卡里只存身份，不存 token。
    pub fn codex_login_ref(&self) -> Option<&str> {
        self.meta
            .extra
            .get(META_CODEX_LOGIN_REF)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
    }

    pub fn set_codex_login_ref(&mut self, identity: Option<&str>) {
        match identity.map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => {
                self.meta
                    .extra
                    .insert(META_CODEX_LOGIN_REF.into(), Value::String(id.into()));
            }
            None => {
                self.meta.extra.remove(META_CODEX_LOGIN_REF);
            }
        }
    }
}

/// `meta` 里 Codex 官方卡引用登录的键。
pub const META_CODEX_LOGIN_REF: &str = "codexLoginRef";

pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rows_round_trip_with_unknown_fields() {
        let raw = json!({
            "id": "p1",
            "name": "Relay",
            "settingsConfig": {"auth": {}, "config": ""},
            "meta": {"authBinding": {"authProvider": "codex_oauth", "accountId": "acc"}, "x": 1},
            "websiteUrl": "https://example.com"
        });
        let provider: Provider = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(provider.managed_codex_account(), Some("acc"));
        assert_eq!(serde_json::to_value(&provider).unwrap(), raw);
    }

    #[test]
    fn tool_names() {
        for tool in Tool::ALL {
            assert_eq!(tool.as_str().parse::<Tool>().unwrap(), tool);
        }
        assert!("nope".parse::<Tool>().is_err());
    }
}
