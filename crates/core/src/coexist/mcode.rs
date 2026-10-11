//! MiniMax Code（`~/.minimax/config.yaml`，TUI 和桌面端共用）：自定义供应商在
//! `custom_provider.<key>`（`kind` 缺省或为 `custom`）；`kind` 是别的值的是 MCode 自己的
//! 账号供应商，不碰。MCode 自己管默认模型：正被 `defaultModel` / `defaultLightModel`
//! 引用的供应商或模型不能删掉或停用。
//!
//! 移植自 cc-switch `mcode_config.rs`：整份 YAML 重写（会丢注释，和原版一致），写时拿
//! MCode 的目录锁 `config.yaml.lock`（proper-lockfile 同款，10 秒算过期）。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};

use super::{key_conflict, rejected, Edit, EntryChange, WriteMode};
use crate::error::AppError;
use crate::live::patch::{decode_utf8, LivePatch, LiveWriteError};
use crate::provider::Tool;

fn parse(text: &str) -> Result<serde_yaml::Value, AppError> {
    let value: serde_yaml::Value = serde_yaml::from_str(text)
        .map_err(|_| AppError::Config("MCode 配置不是合法的 YAML".into()))?;
    match value {
        serde_yaml::Value::Null => Ok(serde_yaml::Value::Mapping(Default::default())),
        serde_yaml::Value::Mapping(_) => Ok(value),
        _ => Err(AppError::Config("MCode 配置的根必须是映射".into())),
    }
}

pub(crate) fn entries(text: &str) -> Result<Map<String, Value>, AppError> {
    let document = parse(text)?;
    match document.get("custom_provider") {
        None | Some(serde_yaml::Value::Null) => Ok(Map::new()),
        Some(value) => {
            let mut providers: Map<String, Value> = serde_yaml::from_value(value.clone())
                .map_err(|_| AppError::Config("MCode 的 custom_provider 配置不合法".into()))?;
            providers.retain(|_, provider| {
                provider
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_none_or(|kind| kind == "custom")
            });
            Ok(providers)
        }
    }
}

fn valid_http_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    !host.is_empty() && !host.starts_with(':') && !host.contains(char::is_whitespace)
}

pub(crate) fn validate(id: &str, config: &Value) -> Result<(), AppError> {
    if id.is_empty()
        || id
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && !matches!(c, '-' | '_'))
    {
        return Err(AppError::localized(
            "provider.mcode.invalid_key",
            "MCode 供应商的键只能包含字母、数字、- 和 _",
            "MCode provider keys may only contain letters, digits, - and _",
        ));
    }
    if !matches!(
        config
            .get("api")
            .map_or(Some("anthropic-messages"), Value::as_str),
        Some("anthropic-messages" | "openai-completions" | "openai-responses")
    ) {
        return Err(AppError::localized(
            "provider.mcode.api_unsupported",
            "MCode 不支持这个 API 格式",
            "MCode does not support this API format",
        ));
    }
    let base_url = config
        .pointer("/options/baseURL")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !valid_http_url(base_url) {
        return Err(AppError::localized(
            "provider.mcode.invalid_url",
            "MCode 的接口地址不是合法的 http(s) URL",
            "The MCode base URL is not a valid http(s) URL",
        ));
    }
    if config
        .pointer("/options/apiKey")
        .and_then(Value::as_str)
        .is_none_or(|key| key.trim().is_empty())
    {
        return Err(AppError::localized(
            "provider.mcode.missing_key",
            "MCode 供应商缺少 API Key",
            "The MCode provider is missing an API key",
        ));
    }
    if config
        .get("models")
        .and_then(Value::as_object)
        .is_none_or(|models| models.is_empty() || models.keys().any(|id| id.trim().is_empty()))
    {
        return Err(AppError::localized(
            "provider.mcode.no_models",
            "MCode 供应商至少要有一个模型",
            "An MCode provider needs at least one model",
        ));
    }
    Ok(())
}

pub(crate) struct McodePatch<'a> {
    pub change: &'a EntryChange<'a>,
}

impl McodePatch<'_> {
    fn edit(&self, text: &str) -> Result<String, AppError> {
        let id = self.change.id;
        let provider = match &self.change.edit {
            Edit::Upsert(value) => Some(value),
            Edit::Remove => None,
        };
        let mut document = parse(text)?;
        let prefix = format!("custom_provider:{id}/");
        if ["defaultModel", "defaultLightModel"].iter().any(|field| {
            let selected = document
                .get(*field)
                .and_then(serde_yaml::Value::as_str)
                .and_then(|model| model.strip_prefix(&prefix));
            selected.is_some_and(|model| {
                !provider.is_some_and(|provider| {
                    provider.get("enabled") != Some(&Value::Bool(false))
                        && provider
                            .get("models")
                            .and_then(|models| models.get(model))
                            .is_some_and(|model| {
                                model.is_object()
                                    && model.get("enabled") != Some(&Value::Bool(false))
                            })
                })
            })
        }) {
            return Err(AppError::InvalidInput(
                "这个供应商 / 模型正被 MCode 用作默认模型，先在 MCode 里换一个默认模型".into(),
            ));
        }
        let root = document
            .as_mapping_mut()
            .expect("parse guarantees a mapping");
        let entry = root
            .entry("custom_provider".into())
            .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
        if entry.is_null() {
            *entry = serde_yaml::Value::Mapping(Default::default());
        }
        let providers = entry
            .as_mapping_mut()
            .ok_or_else(|| AppError::Config("MCode 的 custom_provider 配置不合法".into()))?;
        let key = serde_yaml::Value::from(id);
        if providers
            .get(&key)
            .and_then(|value| value.get("kind"))
            .and_then(serde_yaml::Value::as_str)
            .is_some_and(|kind| kind != "custom")
        {
            return Err(AppError::InvalidInput(
                "这是 MCode 自己的账号供应商，不能改".into(),
            ));
        }
        match provider {
            Some(provider) => {
                let value = serde_yaml::to_value(provider)
                    .map_err(|_| AppError::Config("MCode 供应商配置不合法".into()))?;
                if self.change.mode == WriteMode::CreateOnly
                    && providers
                        .get(&key)
                        .is_some_and(|existing| *existing != value)
                {
                    return Err(key_conflict(Tool::Mcode, id));
                }
                providers.insert(key, value);
            }
            None => {
                providers.remove(&key);
            }
        }
        serde_yaml::to_string(&document)
            .map_err(|_| AppError::Config("MCode 配置序列化失败".into()))
    }
}

impl LivePatch for McodePatch<'_> {
    fn apply(&self, path: &Path, pre: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let text = match pre {
            Some(bytes) => decode_utf8(path, bytes)?,
            None => "",
        };
        self.edit(text).map(String::into_bytes).map_err(rejected)
    }
}

/// MCode 的目录锁：`config.yaml.lock` 目录存在表示 MCode 正在写；超过 10 秒算过期。
pub(crate) struct DirLock(PathBuf);

impl DirLock {
    pub(crate) fn acquire(config: &Path) -> Result<Self, AppError> {
        let parent = config
            .parent()
            .expect("MCode config has a parent directory");
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        let target = fs::canonicalize(config).unwrap_or_else(|_| config.to_path_buf());
        let lock = PathBuf::from(format!("{}.lock", target.display()));
        if let Err(error) = fs::create_dir(&lock) {
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(AppError::io(&lock, error));
            }
            let stale = fs::metadata(&lock)
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|mtime| mtime.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(10));
            if !stale {
                return Err(AppError::Conflict("MCode 正在保存配置，稍后再试".into()));
            }
            fs::remove_dir(&lock).map_err(|e| AppError::io(&lock, e))?;
            fs::create_dir(&lock).map_err(|e| AppError::io(&lock, e))?;
        }
        Ok(Self(lock))
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}
