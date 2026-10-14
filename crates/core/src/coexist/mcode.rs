//! MiniMax Code custom providers, protected defaults, and cooperative directory locking.
use super::{
    jsonc_entry::entries_at, key_conflict, rejected, yaml_sections::Document, Edit, EntryChange,
    WriteMode,
};
use crate::{
    error::AppError,
    live::patch::{decode_utf8, LivePatch, LiveWriteError},
    provider::Tool,
};
use serde_json::{Map, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub(crate) fn entries(text: &str) -> Result<Map<String, Value>, AppError> {
    let doc = Document::parse(text)?;
    let mut entries = if doc.value.get("custom_provider") == Some(&Value::Null) {
        Map::new()
    } else {
        entries_at(&doc.value, &["custom_provider"])?
    };
    entries.retain(|_, v| {
        v.get("kind")
            .and_then(Value::as_str)
            .is_none_or(|kind| kind == "custom")
    });
    Ok(entries)
}
fn invalid(field: &str) -> AppError {
    AppError::localized(
        "provider.mcode.invalid",
        format!("MCode 配置字段无效：{field}"),
        format!("Invalid MCode configuration field: {field}"),
    )
}

pub(crate) fn http_address(address: &str) -> bool {
    let Some(authority) = address
        .strip_prefix("https://")
        .or_else(|| address.strip_prefix("http://"))
        .and_then(|v| v.split(['/', '?', '#']).next())
    else {
        return false;
    };
    if authority.is_empty()
        || address.chars().any(|c| c.is_whitespace() || c.is_control())
        || authority.contains('@')
    {
        return false;
    }
    if let Some(ipv6) = authority.strip_prefix('[') {
        let Some((ip, tail)) = ipv6.split_once(']') else {
            return false;
        };
        return ip.parse::<std::net::Ipv6Addr>().is_ok()
            && (tail.is_empty()
                || tail
                    .strip_prefix(':')
                    .is_some_and(|p| p.parse::<u16>().is_ok_and(|n| n > 0)));
    }
    let (host, port) = authority
        .split_once(':')
        .map_or((authority, None), |(h, p)| (h, Some(p)));
    !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && port.is_none_or(|p| p.parse::<u16>().is_ok_and(|n| n > 0))
}
pub(crate) fn validate(id: &str, config: &Value) -> Result<(), AppError> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(invalid("id"));
    }
    if config.get("api").is_some_and(|v| {
        !matches!(
            v.as_str(),
            Some("anthropic-messages" | "openai-completions" | "openai-responses")
        )
    }) {
        return Err(invalid("api"));
    }
    if !config
        .pointer("/options/baseURL")
        .and_then(Value::as_str)
        .is_some_and(http_address)
    {
        return Err(invalid("options.baseURL"));
    }
    if config
        .pointer("/options/apiKey")
        .and_then(Value::as_str)
        .is_none_or(|s| s.trim().is_empty())
    {
        return Err(invalid("options.apiKey"));
    }
    if config
        .get("models")
        .and_then(Value::as_object)
        .is_none_or(|m| {
            m.is_empty()
                || m.iter()
                    .any(|(id, value)| id.trim().is_empty() || !value.is_object())
        })
    {
        return Err(invalid("models"));
    }
    Ok(())
}
pub(crate) struct McodePatch<'a> {
    pub change: &'a EntryChange<'a>,
}
impl McodePatch<'_> {
    fn edit(&self, source: &str) -> Result<String, AppError> {
        let doc = Document::parse(source)?;
        let mut providers = if doc.value.get("custom_provider") == Some(&Value::Null) {
            Map::new()
        } else {
            entries_at(&doc.value, &["custom_provider"])?
        };
        let id = self.change.id;
        if providers
            .get(id)
            .and_then(|v| v.get("kind"))
            .and_then(Value::as_str)
            .is_some_and(|s| s != "custom")
        {
            return Err(AppError::InvalidInput(
                "MCode 自有账号供应商不能修改 / Cannot modify an MCode account provider".into(),
            ));
        }
        let next = match &self.change.edit {
            Edit::Upsert(value) => Some(value),
            Edit::Remove => None,
        };
        for key in ["defaultModel", "defaultLightModel"] {
            if let Some(model) = doc
                .value
                .get(key)
                .and_then(Value::as_str)
                .and_then(|s| s.strip_prefix(&format!("custom_provider:{id}/")))
            {
                let retained = next
                    .filter(|v| v.get("enabled") != Some(&Value::Bool(false)))
                    .and_then(|v| v.get("models"))
                    .and_then(|v| v.get(model))
                    .is_some_and(|v| {
                        v.is_object() && v.get("enabled") != Some(&Value::Bool(false))
                    });
                if !retained {
                    return Err(AppError::InvalidInput(
                        "请先在 MCode 中更换默认模型 / Change the MCode default model first".into(),
                    ));
                }
            }
        }
        if let Some(next) = next {
            if self.change.mode == WriteMode::CreateOnly
                && providers.get(id).is_some_and(|v| v != next)
            {
                return Err(key_conflict(Tool::Mcode, id));
            }
            providers.insert(id.into(), next.clone());
        } else {
            providers.shift_remove(id);
        }
        doc.replace(&[("custom_provider", Value::Object(providers))])
    }
}
impl LivePatch for McodePatch<'_> {
    fn apply(&self, path: &Path, before: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        self.edit(decode_utf8(path, before.unwrap_or_default())?)
            .map(String::into_bytes)
            .map_err(rejected)
    }
}
pub(crate) struct DirLock {
    path: PathBuf,
    heartbeat: Option<std::thread::JoinHandle<()>>,
    stop: Option<std::sync::mpsc::Sender<()>>,
}
impl DirLock {
    pub(crate) fn acquire(config: &Path) -> Result<Self, AppError> {
        let parent = config.parent().ok_or_else(|| invalid("config path"))?;
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        let canonical = fs::canonicalize(config).unwrap_or_else(|_| config.to_path_buf());
        let mut name = canonical.as_os_str().to_os_string();
        name.push(".lock");
        let path = PathBuf::from(name);
        fs::create_dir(&path).map_err(|e|if e.kind()==std::io::ErrorKind::AlreadyExists{AppError::Conflict("MCode 配置锁已存在；请等待客户端保存完成 / MCode configuration lock is occupied".into())}else{AppError::io(&path,e)})?;
        // Do not guess that a quiet client is dead and steal its lock. While we
        // own this directory, keep the mtime fresh for proper-lockfile clients.
        let (send, recv) = std::sync::mpsc::channel();
        let location = path.clone();
        let worker = std::thread::spawn(move || {
            while recv
                .recv_timeout(std::time::Duration::from_secs(2))
                .is_err()
            {
                if let Ok(file) = fs::File::open(&location) {
                    let _ = file.set_modified(std::time::SystemTime::now());
                } else {
                    break;
                }
            }
        });
        Ok(Self {
            path,
            heartbeat: Some(worker),
            stop: Some(send),
        })
    }
}
impl Drop for DirLock {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.heartbeat.take() {
            let _ = worker.join();
        }
        let _ = fs::remove_dir(&self.path);
    }
}
