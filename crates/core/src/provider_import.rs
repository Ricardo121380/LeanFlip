//! Read-only foreign provider snapshots. Applying a selection only adds cards to LeanFlip.
//! No current pointers, client files, account stores, or executable trust are imported.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ccswitch_export::Reason;
use crate::{ccswitch_import, store, AppError, Env, Provider, Tool};

mod aitoolbox;
mod superba;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    CcSwitch,
    AiToolbox,
    Superba,
}

pub struct Candidate {
    pub tool: Tool,
    pub provider: Provider,
    pub skip_reason: Option<Reason>,
    pub notes: Vec<Reason>,
    pub needs_api_key: bool,
}

pub struct Scan {
    pub directory: PathBuf,
    pub found: bool,
    pub candidates: Vec<Candidate>,
}

/// Deliberately contains no configuration, metadata, credential references, or keys.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    pub index: usize,
    pub tool: Tool,
    pub name: String,
    pub skip_reason: Option<Reason>,
    pub notes: Vec<Reason>,
    pub needs_api_key: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub preview_id: u64,
    pub directory: PathBuf,
    pub found: bool,
    pub items: Vec<PreviewItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub index: usize,
    pub api_key: Option<String>,
}

impl Scan {
    pub fn preview(&self, preview_id: u64) -> Preview {
        Preview {
            preview_id,
            directory: self.directory.clone(),
            found: self.found,
            items: self
                .candidates
                .iter()
                .enumerate()
                .map(|(index, c)| PreviewItem {
                    index,
                    tool: c.tool,
                    name: c.provider.name.clone(),
                    skip_reason: c.skip_reason.clone(),
                    notes: c.notes.clone(),
                    needs_api_key: c.needs_api_key,
                })
                .collect(),
        }
    }
}

pub fn default_dir(env: &Env, source: Source) -> Result<PathBuf, AppError> {
    let data = if cfg!(target_os = "macos") {
        env.home.join("Library/Application Support")
    } else if cfg!(windows) {
        env.home.join("AppData/Roaming")
    } else {
        env.home.join(".local/share")
    };
    match source {
        Source::CcSwitch => ccswitch_import::detected_dir(env),
        Source::AiToolbox => {
            let dir = data.join("com.ai-toolbox");
            let bootstrap = dir.join("app_paths.json");
            if bootstrap.is_file() {
                let config = read_json(&bootstrap)?;
                if let Some(path) = config["data_dir_override"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                {
                    let path = PathBuf::from(path);
                    if !path.is_absolute() {
                        return Err(invalid_format());
                    }
                    return Ok(path);
                }
            }
            Ok(dir)
        }
        Source::Superba => Ok(if cfg!(windows) {
            env.home.join("AppData/Local/krill-code/Superba/data")
        } else if cfg!(target_os = "macos") {
            data.join("com.krill-code.Superba")
        } else {
            data.join("superba")
        }),
    }
}

pub fn scan(env: &Env, source: Source, directory: Option<PathBuf>) -> Result<Scan, AppError> {
    let directory = match directory {
        Some(dir) => dir,
        None => default_dir(env, source)?,
    };
    if !directory.is_absolute() {
        return Err(AppError::InvalidInput(
            "请选择绝对目录路径 / Choose an absolute directory".into(),
        ));
    }
    let (found, mut candidates) = match source {
        Source::CcSwitch => {
            let scan = ccswitch_import::scan_cards(env, &directory)?;
            (
                scan.source.is_some(),
                scan.candidates
                    .into_iter()
                    .map(|c| Candidate {
                        tool: c.tool,
                        provider: c.provider,
                        skip_reason: c.skip_reason,
                        notes: c.notes,
                        needs_api_key: false,
                    })
                    .collect(),
            )
        }
        Source::AiToolbox => aitoolbox::read(&directory)?,
        Source::Superba => superba::read(&directory)?,
    };
    let data = store::load_readonly(env)?;
    let mut seen = HashSet::new();
    for c in &mut candidates {
        if c.skip_reason
            .as_ref()
            .is_some_and(|r| r.code == "unsupported_config")
        {
            c.skip_reason = Some(Reason::new(
                "unsupported_config",
                "配置格式不兼容，请先在来源应用中检查连接设置",
                "Incompatible configuration; review connection settings in the source app",
            ));
        }
        if c.provider.meta.auth_binding.is_some() {
            c.skip_reason = Some(Reason::new(
                "account_binding",
                "依赖托管登录账号，请使用数据导入中的账号迁移",
                "Requires a managed login; use account migration in Data import",
            ));
        }
        if !seen.insert((c.tool, c.provider.id.clone())) {
            c.skip_reason = Some(Reason::new(
                "duplicate_source",
                "来源中有重复 ID",
                "Duplicate ID in source",
            ));
        }
        if c.skip_reason.is_none() {
            // Validate the connection shape with a temporary credential when the UI
            // explicitly requires a key. This value is never saved or returned.
            let mut check = c.provider.clone();
            if c.needs_api_key {
                set_key(c.tool, &mut check, "import-preview-placeholder")?;
            }
            c.skip_reason = safe_skip_reason(&data, c.tool, &check);
        }
    }
    Ok(Scan {
        directory,
        found,
        candidates,
    })
}

pub fn apply(
    env: &Env,
    scan: &Scan,
    selected: &[Selection],
) -> Result<ccswitch_import::Report, AppError> {
    let mut seen = HashSet::new();
    let mut cards = Vec::new();
    for selection in selected {
        let c = scan
            .candidates
            .get(selection.index)
            .ok_or_else(invalid_selection)?;
        if !seen.insert(selection.index) || c.skip_reason.is_some() {
            return Err(invalid_selection());
        }
        let mut provider = c.provider.clone();
        if c.needs_api_key {
            let key = selection
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.len() <= 16384)
                .ok_or_else(|| {
                    AppError::InvalidInput(
                        "请补填所选供应商的 API Key / Enter the API key for selected providers"
                            .into(),
                    )
                })?;
            set_key(c.tool, &mut provider, key)?;
        }
        cards.push((c.tool, provider));
    }
    if cards.is_empty() {
        return Err(invalid_selection());
    }
    store::update(env, |data| {
        let mut report = ccswitch_import::Report::default();
        for (tool, mut provider) in cards {
            if let Some(reason) = safe_skip_reason(data, tool, &provider) {
                report.skipped.push((tool, provider.id, reason));
                continue;
            }
            provider.sort_index = Some(data.providers(tool).len() as i64);
            report.created.push((tool, provider.id.clone()));
            data.app_mut(tool).providers.push(provider);
        }
        Ok(report)
    })
}

fn invalid_selection() -> AppError {
    AppError::InvalidInput("导入选择无效，请重新预览 / Invalid selection; preview again".into())
}

fn invalid_format() -> AppError {
    // Parser messages can contain secret field values; never return them to the UI.
    AppError::Config("无法读取来源配置，请检查文件格式和版本 / Cannot read source configuration; check its format and version".into())
}

fn read_bytes(path: &Path, limit: u64) -> Result<Vec<u8>, AppError> {
    let file = std::fs::File::open(path).map_err(|e| AppError::io(path, e))?;
    if !file
        .metadata()
        .map_err(|e| AppError::io(path, e))?
        .is_file()
    {
        return Err(invalid_format());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| AppError::io(path, e))?;
    if bytes.len() as u64 > limit {
        return Err(invalid_format());
    }
    Ok(bytes)
}

fn read_json(path: &Path) -> Result<Value, AppError> {
    serde_json::from_slice(&read_bytes(path, 16 * 1024 * 1024)?).map_err(|_| invalid_format())
}

fn candidate(tool: Tool, provider: Provider) -> Candidate {
    Candidate {
        tool,
        notes: crate::switch::provider_warnings(tool, &provider),
        provider,
        skip_reason: None,
        needs_api_key: false,
    }
}

fn set_key(tool: Tool, provider: &mut Provider, key: &str) -> Result<(), AppError> {
    match tool {
        Tool::Codex => provider.settings_config["auth"]["OPENAI_API_KEY"] = key.into(),
        Tool::Claude | Tool::ClaudeDesktop => {
            provider.settings_config["env"]["ANTHROPIC_AUTH_TOKEN"] = key.into()
        }
        Tool::Gemini => provider.settings_config["env"]["GEMINI_API_KEY"] = key.into(),
        _ => return Err(invalid_selection()),
    }
    Ok(())
}

fn safe_skip_reason(data: &store::StoreData, tool: Tool, provider: &Provider) -> Option<Reason> {
    ccswitch_import::skip_reason(data, tool, provider).map(|reason| {
        if reason.code == "unsupported_config" {
            Reason::new(
                "unsupported_config",
                "配置格式不兼容，请先在来源应用中检查连接设置",
                "Incompatible configuration; review connection settings in the source app",
            )
        } else {
            reason
        }
    })
}
