//! Validation at the native-file write boundary for curated numeric controls.
//! Unknown enum values remain forward compatible; unrelated legacy values survive edits.
use crate::{AppError, Tool};
use serde_json::Value;

const SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

fn number(
    value: Option<&Value>,
    path: &str,
    min: f64,
    max: f64,
    ratio: bool,
) -> Result<(), AppError> {
    let Some(value) = value else { return Ok(()) };
    if value.as_f64().is_some_and(|n| {
        n.is_finite()
            && n >= min
            && n <= max
            && if ratio {
                n > 0.0
            } else {
                value.is_i64() || value.is_u64()
            }
    }) {
        return Ok(());
    }
    Err(AppError::Config(format!(
        "配置数值无效 / Invalid configuration number: {path}"
    )))
}

pub(crate) fn common(tool: Tool, before: &Value, after: &Value) -> Result<(), AppError> {
    let rules: &[(&str, f64, f64, bool)] = match tool {
        Tool::Gemini => &[
            ("/model/compressionThreshold", 0.0, 1.0, true),
            ("/model/maxSessionTurns", -1.0, SAFE_INTEGER, false),
        ],
        Tool::GrokBuild => &[("/session/auto_compact_threshold_percent", 0.0, 100.0, false)],
        Tool::OpenCode => &[
            ("/compaction/reserved", 0.0, SAFE_INTEGER, false),
            ("/compaction/buffer", 0.0, SAFE_INTEGER, false),
            ("/compaction/keep/tokens", 0.0, SAFE_INTEGER, false),
        ],
        Tool::OpenClaw => &[
            (
                "/agents/defaults/compaction/keepRecentTokens",
                1.0,
                SAFE_INTEGER,
                false,
            ),
            ("/agents/defaults/maxConcurrent", 1.0, SAFE_INTEGER, false),
            ("/agents/defaults/timeoutSeconds", 0.0, SAFE_INTEGER, false),
        ],
        Tool::Hermes => &[
            ("/compression/threshold", 0.0, 1.0, true),
            ("/compression/threshold_tokens", 1.0, SAFE_INTEGER, false),
        ],
        Tool::Pi => &[
            ("/compaction/reserveTokens", 0.0, SAFE_INTEGER, false),
            ("/compaction/keepRecentTokens", 0.0, SAFE_INTEGER, false),
            ("/retry/maxRetries", 0.0, SAFE_INTEGER, false),
        ],
        Tool::Mcode => &[
            (
                "/toolResultCompaction/maxInlineKiB",
                1.0,
                8_796_093_022_207.0,
                false,
            ),
            (
                "/toolResultCompaction/keepRecentRounds",
                1.0,
                SAFE_INTEGER,
                false,
            ),
        ],
        _ => &[],
    };
    for &(path, min, max, ratio) in rules {
        let value = after.pointer(path);
        if value == before.pointer(path)
            || (tool == Tool::Hermes
                && path == "/compression/threshold_tokens"
                && value == Some(&Value::Null))
        {
            continue;
        }
        number(value, path, min, max, ratio)?;
    }
    let booleans: &[&str] = match tool {
        Tool::GrokBuild => &["/cli/auto_update"],
        Tool::OpenCode => &["/compaction/auto", "/compaction/prune"],
        Tool::OpenClaw => &[
            "/agents/defaults/compaction/enabled",
            "/agents/defaults/compaction/memoryFlush/enabled",
        ],
        Tool::Hermes => &[
            "/compression/enabled",
            "/memory/memory_enabled",
            "/memory/user_profile_enabled",
        ],
        Tool::Pi => &["/compaction/enabled", "/retry/enabled"],
        Tool::Mcode => &["/toolResultCompaction/enabled"],
        _ => &[],
    };
    for path in booleans {
        if after.pointer(path) != before.pointer(path)
            && after.pointer(path).is_some_and(|v| !v.is_boolean())
        {
            return Err(AppError::Config(format!(
                "配置必须是布尔值 / Configuration must be boolean: {path}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn provider(tool: Tool, config: &Value) -> Result<(), AppError> {
    match tool {
        Tool::OpenCode | Tool::Mcode => {
            if let Some(models) = config.get("models").and_then(Value::as_object) {
                for model in models.values() {
                    for path in ["/limit/context", "/limit/output"] {
                        number(model.pointer(path), path, 1.0, SAFE_INTEGER, false)?;
                    }
                }
            }
        }
        Tool::OpenClaw => {
            if let Some(models) = config.get("models").and_then(Value::as_array) {
                for model in models {
                    number(
                        model.get("contextWindow"),
                        "contextWindow",
                        1.0,
                        SAFE_INTEGER,
                        false,
                    )?;
                }
            }
        }
        Tool::GrokBuild => {
            let parsed = config
                .get("config")
                .and_then(Value::as_str)
                .and_then(|text| text.parse::<toml::Value>().ok())
                .and_then(|value| serde_json::to_value(value).ok());
            if let Some(models) = parsed
                .as_ref()
                .and_then(|v| v.get("model"))
                .and_then(Value::as_object)
            {
                let selected = parsed
                    .as_ref()
                    .and_then(|v| v.pointer("/models/default"))
                    .and_then(Value::as_str);
                // Only the selected table is written; unrelated tables retain their own settings.
                let model = selected.and_then(|id| models.get(id)).or_else(|| {
                    (models.len() == 1)
                        .then(|| models.values().next())
                        .flatten()
                });
                if let Some(model) = model {
                    for key in ["context_window", "max_completion_tokens"] {
                        number(model.get(key), key, 1.0, SAFE_INTEGER, false)?;
                    }
                    number(
                        model.get("auto_compact_threshold_percent"),
                        "auto_compact_threshold_percent",
                        0.0,
                        100.0,
                        false,
                    )?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
