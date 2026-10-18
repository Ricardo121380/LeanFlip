use super::*;
use serde_json::json;

pub(super) fn read(dir: &Path) -> Result<(bool, Vec<Candidate>), AppError> {
    let path = dir.join("profiles.json");
    if !path.try_exists().map_err(|e| AppError::io(&path, e))? {
        return Ok((false, vec![]));
    }
    let root = read_json(&path)?;
    if root.get("schema").is_some_and(|v| v != 1) {
        return Err(invalid_format());
    }
    let profiles = root["profiles"].as_array().ok_or_else(invalid_format)?;
    let mut candidates = Vec::new();
    for raw in profiles {
        let id = raw["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(invalid_format)?;
        let name = raw["name"].as_str().ok_or_else(invalid_format)?;
        let mut c = candidate(
            Tool::Codex,
            Provider::new(
                format!("superba:{id}"),
                name,
                json!({"auth": {}, "config": ""}),
            ),
        );
        if !raw["builtin"].is_null() || raw["auth_mode"] == "chatgpt" {
            c.skip_reason = Some(Reason::new(
                "login",
                "此项是配置恢复点或 ChatGPT 登录，不是 API Provider",
                "This is a restore point or ChatGPT login, not an API provider",
            ));
        } else if raw["auth"] == "pat" || (raw["auth"].is_null() && raw["auth_mode"] == "pat") {
            c.skip_reason = Some(Reason::new(
                "pat",
                "手动 PAT 模式依赖 Superba 的认证流程，请先改为 API Key 模式",
                "Manual PAT mode needs Superba authentication; change it to API key mode first",
            ));
        }
        let base = base_url(&root, raw);
        if c.skip_reason.is_none() && base.is_none() {
            c.skip_reason = Some(Reason::new(
                "endpoint",
                "无法确定线路地址，请先在 Superba 中指定完整 Base URL",
                "Cannot resolve the endpoint; set a full Base URL in Superba first",
            ));
        }
        if let Some(base) = base {
            let mut doc = toml_edit::DocumentMut::new();
            doc["model_provider"] = toml_edit::value("superba");
            doc["model_providers"]["superba"]["name"] = toml_edit::value(name);
            doc["model_providers"]["superba"]["base_url"] = toml_edit::value(base);
            doc["model_providers"]["superba"]["wire_api"] = toml_edit::value("responses");
            doc["model_providers"]["superba"]["requires_openai_auth"] = toml_edit::value(true);
            for (from, to) in [
                ("model", "model"),
                ("reasoning_effort", "model_reasoning_effort"),
            ] {
                if let Some(s) = raw["model"][from].as_str() {
                    let value = if from == "model" {
                        root["custom_sites"]
                            .as_array()
                            .and_then(|sites| {
                                sites.iter().find(|site| site["id"] == raw["site_id"])
                            })
                            .and_then(|site| site["model_map"][s].as_str())
                            .unwrap_or(s)
                    } else {
                        s
                    };
                    doc[to] = toml_edit::value(value);
                }
            }
            for (from, to) in [
                ("context_window", "model_context_window"),
                ("auto_compact", "model_auto_compact_token_limit"),
            ] {
                if let Some(n) = raw["model"][from].as_i64() {
                    doc[to] = toml_edit::value(n);
                }
            }
            let speed = match raw["model"]["speed"].as_str() {
                Some("fast") => "fast",
                Some("ultrafast") => "ultrafast",
                _ => "default",
            };
            doc["service_tier"] = toml_edit::value(speed);
            c.provider.settings_config["config"] = doc.to_string().into();
        }
        if c.skip_reason.is_none() {
            let key = file_key(dir, &raw["key_ref"])?;
            c.needs_api_key = key.is_none();
            if let Some(key) = key {
                c.provider.settings_config["auth"]["OPENAI_API_KEY"] = key.into();
            }
            if raw["site_id"] == "krill" && raw["base_url_override"].is_null() {
                c.notes.push(Reason::new(
                    "krill_endpoint",
                    "内置 Krill 线路使用 API Key 端点 /codex/v1",
                    "Built-in Krill lines use the API-key endpoint /codex/v1",
                ));
            }
            c.notes.push(Reason::new("api_key_mode", "按 API Key 直连导入所选主线路；不迁移自动认证、备用线路、路由器或额外 Codex 设置", "Imports the primary endpoint as a direct API-key provider; automatic authentication, fallback lines, router and extra Codex settings are excluded"));
        }
        candidates.push(c);
    }
    Ok((true, candidates))
}

fn base_url(root: &Value, raw: &Value) -> Option<String> {
    if let Some(url) = raw["base_url_override"]
        .as_str()
        .filter(|s| s.starts_with("https://") || s.starts_with("http://"))
    {
        return Some(url.to_owned());
    }
    // Built-in line definitions verified against krill-lab/superba; unknown/new lines require manual configuration.
    if raw["site_id"] == "krill" {
        return match raw["line_id"].as_str().unwrap_or("overseas") {
            "overseas" => Some("https://api.krill-code.net/codex/v1".into()),
            "cn-fast" => Some("https://api-slb.krill-code.net/codex/v1".into()),
            "cdn" => Some("https://api.cdn-krill-ai.com/codex/v1".into()),
            _ => None,
        };
    }
    let site = root["custom_sites"]
        .as_array()?
        .iter()
        .find(|s| s["id"] == raw["site_id"])?;
    let lines = site["lines"].as_array()?;
    let line_id = raw["line_id"]
        .as_str()
        .or_else(|| site["default_line"].as_str());
    let line = if let Some(id) = line_id {
        lines.iter().find(|l| l["id"] == id)?
    } else {
        lines.first()?
    };
    let origin = line["origin"].as_str()?;
    if !origin.starts_with("https://") && !origin.starts_with("http://") {
        return None;
    }
    Some(format!("{}/v1", origin.trim_end_matches('/')))
}

/// Only the selected app's private secret directory is eligible. Keyring/env references
/// are never dereferenced: the user supplies that key explicitly in the preview.
fn file_key(dir: &Path, reference: &Value) -> Result<Option<String>, AppError> {
    if reference["kind"] != "file" {
        return Ok(None);
    }
    let Some(raw) = reference["path"].as_str() else {
        return Ok(None);
    };
    let path = Path::new(raw);
    let (Ok(root), Ok(path)) = (dir.join("secrets").canonicalize(), path.canonicalize()) else {
        return Ok(None);
    };
    let Ok(source_root) = dir.canonicalize() else {
        return Ok(None);
    };
    if !root.starts_with(source_root)
        || path.parent() != Some(root.as_path())
        || !path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.starts_with("profile-") && s.ends_with(".secret"))
    {
        return Ok(None);
    }
    let bytes = read_bytes(&path, 16384)?;
    let key = String::from_utf8(bytes).map_err(|_| invalid_format())?;
    Ok((!key.trim().is_empty()).then_some(key))
}
