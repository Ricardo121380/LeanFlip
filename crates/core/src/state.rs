//! Persistent write-journal schema and serialized updates for LeanFlip.
//! Field names remain a storage contract, including unknown extension fields.
use crate::{
    fsutil,
    live::engine::{lock_file, DeviceStore},
    AppError,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, fs, io::Write, path::PathBuf};

pub const STATE_VERSION: u32 = 1;
fn version() -> u32 {
    STATE_VERSION
}
fn unset(value: &bool) -> bool {
    !value
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveState {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default)]
    pub apps: BTreeMap<String, AppLiveState>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl Default for LiveState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            apps: BTreeMap::new(),
            extra: Map::new(),
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppLiveState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written: Option<Written>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Written {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub op: String,
    pub files: Vec<PendingFile>,
    #[serde(default)]
    pub target: PendingTarget,
    #[serde(default, skip_serializing_if = "unset")]
    pub published: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingFile {
    pub path: PathBuf,
    /// Actual publication path; `path` retains the user's configured link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<PathBuf>,
    pub pre: Option<String>,
    #[serde(default)]
    pub planned: Option<String>,
    #[serde(default)]
    pub staged: Option<PathBuf>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PendingTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written: Option<Written>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl PendingTarget {
    pub fn pointer(pointer: Option<String>) -> Self {
        Self {
            pointer,
            ..Self::default()
        }
    }
    pub fn is_empty(&self) -> bool {
        self.pointer.is_none() && self.written.is_none() && self.extra.is_empty()
    }
}
pub mod op {
    pub const SWITCH: &str = "switch";
    pub const APPLY: &str = "apply";
}

fn read_locked(store: &DeviceStore) -> Result<LiveState, AppError> {
    let path = store.state_path();
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LiveState::default()),
        Err(e) => return Err(AppError::io(&path, e)),
    };
    match serde_json::from_slice::<LiveState>(&bytes) {
        Ok(state) if state.version == STATE_VERSION => Ok(state),
        Ok(_) => Err(AppError::Config("Unsupported write-journal version".into())),
        Err(_) => {
            // Archive invalid bytes under a collision-free name before resetting.
            // Parsing diagnostics are deliberately excluded from logs.
            let parent = path
                .parent()
                .ok_or_else(|| AppError::Config("Invalid journal path".into()))?;
            let mut archive = tempfile::Builder::new()
                .prefix("live-state.json.corrupt-")
                .tempfile_in(parent)
                .map_err(|e| AppError::io(parent, e))?;
            archive
                .write_all(&bytes)
                .map_err(|e| AppError::io(&path, e))?;
            archive
                .as_file()
                .sync_all()
                .map_err(|e| AppError::io(&path, e))?;
            archive.keep().map_err(|e| AppError::io(&path, e.error))?;
            fsutil::delete_file(&path)?;
            log::warn!("An invalid LeanFlip journal was archived before recovery");
            Ok(LiveState::default())
        }
    }
}

pub fn load(store: &DeviceStore) -> Result<LiveState, AppError> {
    if !store
        .state_path()
        .parent()
        .is_some_and(|path| path.is_dir())
    {
        return Ok(LiveState::default());
    }
    let _file = lock_file(&store.file("live-state.json.lock"), true)?;
    read_locked(store)
}
pub fn update<R>(
    store: &DeviceStore,
    change: impl FnOnce(&mut LiveState) -> R,
) -> Result<R, AppError> {
    let _file = lock_file(&store.file("live-state.json.lock"), true)?;
    let mut data = read_locked(store)?;
    let result = change(&mut data);
    data.apps
        .retain(|_, app| app.pending.is_some() || app.written.is_some() || !app.extra.is_empty());
    let bytes =
        serde_json::to_vec_pretty(&data).map_err(|source| AppError::JsonSerialize { source })?;
    fsutil::atomic_write_private(&store.state_path(), &bytes)?;
    Ok(result)
}
pub fn pending(store: &DeviceStore, app: &str) -> Result<Option<Pending>, AppError> {
    Ok(load(store)?
        .apps
        .get_mut(app)
        .and_then(|entry| entry.pending.take()))
}
pub fn written(store: &DeviceStore, app: &str) -> Result<Option<Written>, AppError> {
    Ok(load(store)?
        .apps
        .get_mut(app)
        .and_then(|entry| entry.written.take()))
}
pub fn set_pending(
    store: &DeviceStore,
    app: &str,
    intent: Option<Pending>,
) -> Result<(), AppError> {
    update(store, |data| {
        data.apps.entry(app.into()).or_default().pending = intent
    })
}
pub fn apps_with_pending(store: &DeviceStore) -> Result<Vec<String>, AppError> {
    Ok(load(store)?
        .apps
        .into_iter()
        .filter_map(|(key, app)| app.pending.map(|_| key))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_tool_updates_and_unknown_fields_survive() {
        let root = tempfile::tempdir().unwrap();
        let store = DeviceStore::at(root.path());
        fs::write(
            store.state_path(),
            br#"{"version":1,"future":{"enabled":true},"apps":{"one":{"other":42}}}"#,
        )
        .unwrap();
        update(&store, |data| {
            data.apps.entry("two".into()).or_default().written = Some(Written::default())
        })
        .unwrap();
        let data = load(&store).unwrap();
        assert_eq!(data.apps["one"].extra["other"], 42);
        assert_eq!(data.extra["future"]["enabled"], true);
        assert!(data.apps["two"].written.is_some());
    }
    #[test]
    fn corrupt_bytes_are_retained_and_newer_versions_are_not_reset() {
        let root = tempfile::tempdir().unwrap();
        let store = DeviceStore::at(root.path());
        fs::write(store.state_path(), b"incomplete journal {").unwrap();
        assert!(load(&store).unwrap().apps.is_empty());
        let archive = fs::read_dir(root.path())
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("live-state.json.corrupt-")
            })
            .unwrap();
        assert_eq!(fs::read(archive.path()).unwrap(), b"incomplete journal {");
        fs::write(store.state_path(), br#"{"version":99,"apps":{}}"#).unwrap();
        assert!(load(&store).is_err());
        assert_eq!(
            fs::read(store.state_path()).unwrap(),
            br#"{"version":99,"apps":{}}"#
        );
    }
}
