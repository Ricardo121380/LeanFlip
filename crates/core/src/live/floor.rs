//! Client schema names used by LeanFlip's connection editor.
//!
//! Ownership is deliberately narrower than a complete client configuration.
//! Authentication/routing/model selection is replaced; user preferences are retained.
//! References and compatibility decisions are recorded in docs/engineering/independent-core.md.

pub const CLAUDE_PROTOCOL_SELECTORS: &[&str] = &[
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_GATEWAY",
    "CLAUDE_CODE_USE_MANTLE",
    "CLAUDE_CODE_USE_VERTEX",
];
pub const CLAUDE_FLOOR_ENV_PREFIXES: &[&str] = &["ANTHROPIC_", "AWS_", "VERTEX_REGION_"];
pub const CLAUDE_FLOOR_ENV_KEYS: &[&str] = &[
    "CLAUDE_CODE_API_KEY_HELPER_TTL_MS",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
    "CLAUDE_CODE_OAUTH_SCOPES",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL_FORCE",
    "CLOUD_ML_REGION",
    "GOOGLE_APPLICATION_CREDENTIALS",
];
pub const CLAUDE_FLOOR_TOP: &[&str] = &[
    "advisorModel",
    "apiBaseUrl",
    "apiKey",
    "apiKeyHelper",
    "awsAuthRefresh",
    "awsCredentialExport",
    "fallbackModel",
    "gcpAuthRefresh",
    "model",
    "modelOverrides",
    "modelPicker",
    "primaryModel",
    "smallFastModel",
];
pub const CLAUDE_EXCLUSIVE_ENV: &[&str] = &[
    "CLAUDE_CODE_ALWAYS_ENABLE_EFFORT",
    "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
    "CLAUDE_CODE_AUTO_MODE_SERVER",
    "CLAUDE_CODE_DISABLE_1M_CONTEXT",
    "CLAUDE_CODE_DISABLE_ARTIFACT",
    "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS",
    "CLAUDE_CODE_DISABLE_THINKING",
    "CLAUDE_CODE_DISABLE_UNKNOWN_MODEL_WINDOW_ENFORCEMENT",
    "CLAUDE_CODE_EFFORT_LEVEL",
    "CLAUDE_CODE_ENABLE_FINE_GRAINED_TOOL_STREAMING",
    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY",
    "CLAUDE_CODE_EXTRA_BODY",
    "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
    "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
    "DISABLE_INTERLEAVED_THINKING",
    "ENABLE_TOOL_SEARCH",
];
pub fn claude_floor_top(key: &str) -> bool {
    CLAUDE_FLOOR_TOP.contains(&key)
}
pub fn claude_exclusive_env(key: &str) -> bool {
    CLAUDE_EXCLUSIVE_ENV.contains(&key)
}
pub fn claude_floor_env(key: &str) -> bool {
    if CLAUDE_FLOOR_ENV_KEYS.contains(&key) || CLAUDE_PROTOCOL_SELECTORS.contains(&key) {
        return true;
    }
    if let Some(rest) = key.strip_prefix("CLAUDE_CODE_SKIP_") {
        return rest.ends_with("_AUTH");
    }
    CLAUDE_FLOOR_ENV_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
}
pub const CODEX_PROVIDER_TABLE: &[&str] = &["model_providers", "custom"];
pub const CODEX_FLOOR_TOP: &[&str] = &[
    "base_url",
    "disable_response_storage",
    "experimental_bearer_token",
    "model",
    "model_catalog_json",
    "model_provider",
    "model_reasoning_effort",
    "openai_base_url",
    "plan_mode_reasoning_effort",
    "review_model",
    "wire_api",
];
pub const CODEX_FLOOR_NESTED: &[&[&str]] = &[
    &["agents", "default_subagent_model"],
    &["agents", "default_subagent_reasoning_effort"],
    &["memories", "extract_model"],
    &["memories", "consolidation_model"],
];
pub const CODEX_EXCLUSIVE_TOP: &[&str] = &[
    "model_auto_compact_token_limit",
    "model_context_window",
    "model_supports_reasoning_summaries",
    "model_verbosity",
    "web_search",
];
pub fn gemini_floor_env(key: &str) -> bool {
    key.starts_with("GOOGLE_")
        || [
            "CODE_ASSIST_API_VERSION",
            "CODE_ASSIST_ENDPOINT",
            "GEMINI_API_KEY",
            "GEMINI_API_KEY_AUTH_MECHANISM",
            "GEMINI_CLI_CUSTOM_HEADERS",
            "GEMINI_CLI_USE_COMPUTE_ADC",
            "GEMINI_DEFAULT_AUTH_TYPE",
            "GEMINI_MODEL",
        ]
        .contains(&key)
}
pub const GEMINI_FLOOR_SETTINGS: &[&[&str]] =
    &[&["security", "auth", "selectedType"], &["model", "name"]];
pub const DESKTOP_PROFILE_FLOOR: &[&str] = &[
    "inferenceGatewayApiKey",
    "inferenceGatewayAuthScheme",
    "inferenceGatewayBaseUrl",
    "inferenceModels",
    "inferenceProvider",
];
pub const DESKTOP_PROFILE_SEED: &[&str] =
    &["disableDeploymentModeChooser", "coworkEgressAllowedHosts"];
pub fn desktop_profile_floor(key: &str) -> bool {
    DESKTOP_PROFILE_FLOOR.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn user_options_are_not_connection_settings() {
        for name in [
            "API_TIMEOUT_MS",
            "CLAUDE_CODE_USE_POWERSHELL_TOOL",
            "DISABLE_TELEMETRY",
        ] {
            assert!(!claude_floor_env(name), "{name}");
        }
        for name in ["GEMINI_CLI_HOME", "GEMINI_SANDBOX", "GEMINI_SYSTEM_MD"] {
            assert!(!gemini_floor_env(name), "{name}");
        }
        assert!(claude_floor_env("ANTHROPIC_AUTH_TOKEN"));
        assert!(gemini_floor_env("GOOGLE_GEMINI_BASE_URL"));
    }
}
