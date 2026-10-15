//! 场景：列表和状态、预览、应用和撤销。
//!
//! 应用时拿一次写锁，逐个工具走和单独切换一样的路径（恢复、托管账号、警告提示都一样）。
//! 写入记录按工具分开，跨工具做不到全成全败：某个工具失败时继续其他工具，失败的列出来；
//! 应用前给涉及的工具拍一份快照，撤销就是把快照当场景再应用一次。快照只交给调用方，不落盘。
use super::*;
use leanflip_core::scene::{self, Action, Scene, SceneTool};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SceneStatus {
    /// 所有工具都和场景一致。
    Active,
    /// 最近应用的是它，但之后有工具被手动改过。
    Drifted,
    Idle,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SceneMissing {
    pub tool: Tool,
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneView {
    #[serde(flatten)]
    pub scene: Scene,
    pub status: SceneStatus,
    /// 和场景不一致的工具（包括读不了配置的）。
    pub pending: Vec<Tool>,
    /// 场景引用、但已经被删掉的卡。
    pub missing: Vec<SceneMissing>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneList {
    pub scenes: Vec<SceneView>,
    /// 最近一次应用的场景 id。
    pub last: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SceneStep {
    pub action: Action,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPreview {
    pub tool: Tool,
    pub kind: ToolKind,
    /// 单选式工具现在用的卡（名字）。
    pub from: Option<String>,
    pub steps: Vec<SceneStep>,
    pub missing: Vec<String>,
    /// 读不了客户端配置等，这个工具没法预览。
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenePreview {
    pub scene: Scene,
    pub tools: Vec<ToolPreview>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SceneOutcome {
    Applied,
    Unchanged,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutcome {
    pub tool: Tool,
    pub outcome: SceneOutcome,
    pub changed: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub missing: Vec<String>,
    pub error: Option<String>,
}

/// 应用前的状态：撤销时原样交回来。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneUndo {
    #[serde(default)]
    pub tools: BTreeMap<String, SceneTool>,
    #[serde(default)]
    pub last: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneReport {
    pub results: Vec<ToolOutcome>,
    pub undo: SceneUndo,
}

impl SceneReport {
    pub fn failed(&self) -> bool {
        self.results
            .iter()
            .any(|result| result.outcome == SceneOutcome::Failed)
    }
}

impl Service {
    pub fn scenes(&self) -> ApiResult<SceneList> {
        let rt = self.runtime();
        let data = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        let scenes = data
            .scenes
            .iter()
            .map(|scene| {
                let mut pending = Vec::new();
                let mut missing = Vec::new();
                for (tool, entry) in scene.entries() {
                    match scene::plan_tool(&rt.env, &data, tool, entry) {
                        Ok(plan) => {
                            if !plan.in_sync() {
                                pending.push(tool);
                            }
                            missing.extend(
                                plan.missing.into_iter().map(|id| SceneMissing { tool, id }),
                            );
                        }
                        Err(_) => pending.push(tool),
                    }
                }
                let status = if pending.is_empty() {
                    SceneStatus::Active
                } else if data.last_scene.as_deref() == Some(scene.id.as_str()) {
                    SceneStatus::Drifted
                } else {
                    SceneStatus::Idle
                };
                SceneView {
                    scene: scene.clone(),
                    status,
                    pending,
                    missing,
                }
            })
            .collect();
        Ok(SceneList {
            scenes,
            last: data.last_scene,
        })
    }

    /// 把现在的状态记成一个还没保存的场景（id 和名字为空）。`tools` 为空时取所有有卡的工具。
    pub fn scene_capture(&self, tools: Option<Vec<Tool>>) -> ApiResult<Scene> {
        let rt = self.runtime();
        let data = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        let mut scene = Scene::default();
        for tool in tools.unwrap_or_else(|| Tool::ALL.to_vec()) {
            let entry = scene::capture_tool(&rt.env, &data, tool).map_err(|e| self.err(e))?;
            if let Some(entry) = entry {
                scene.tools.insert(tool.as_str().to_string(), entry);
            }
        }
        Ok(scene)
    }

    pub fn save_scene(&self, scene: Scene) -> ApiResult<Scene> {
        let _guard = self.write_guard()?;
        scene::save(&self.env(), scene).map_err(|e| self.err(e))
    }

    pub fn delete_scene(&self, id: &str) -> ApiResult<()> {
        let _guard = self.write_guard()?;
        scene::remove(&self.env(), id)
            .map(|_| ())
            .map_err(|e| self.err(e))
    }

    fn find_scene(&self, data: &store::StoreData, key: &str) -> ApiResult<Scene> {
        scene::find(data, key).cloned().ok_or_else(|| {
            self.err(AppError::localized(
                "scene.not_found",
                format!("找不到场景 {key}"),
                format!("Scene {key} was not found"),
            ))
        })
    }

    /// 应用前看一眼：每个工具要做什么。只读。
    pub fn scene_preview(&self, key: &str) -> ApiResult<ScenePreview> {
        let rt = self.runtime();
        let data = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        let scene = self.find_scene(&data, key)?;
        let name = |tool: Tool, id: &str| {
            data.provider(tool, id)
                .map(|card| card.name.clone())
                .unwrap_or_else(|| id.to_string())
        };
        let tools = scene
            .entries()
            .into_iter()
            .map(|(tool, entry)| {
                let from = (!tool.is_coexist())
                    .then(|| data.current(tool).map(|card| card.name.clone()))
                    .flatten();
                let (steps, missing, error) = match scene::plan_tool(&rt.env, &data, tool, entry) {
                    Ok(plan) => (
                        plan.actions
                            .into_iter()
                            .map(|action| SceneStep {
                                name: name(tool, action_id(&action)),
                                action,
                            })
                            .collect(),
                        plan.missing,
                        None,
                    ),
                    Err(error) => (Vec::new(), Vec::new(), Some(self.err(error).0)),
                };
                ToolPreview {
                    tool,
                    kind: kind(tool),
                    from,
                    steps,
                    missing,
                    error,
                }
            })
            .collect();
        Ok(ScenePreview { scene, tools })
    }

    /// 应用场景（按 id 或名字找）。`tools` 给了时只应用其中的工具。
    pub fn apply_scene(&self, key: &str, tools: Option<Vec<Tool>>) -> ApiResult<SceneReport> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        let data = store::load(&rt.env).map_err(|e| self.err(e))?;
        let scene = self.find_scene(&data, key)?;
        scene::validate(&scene).map_err(|e| self.err(e))?;
        let entries = scene
            .entries()
            .into_iter()
            .filter(|(tool, _)| tools.as_ref().is_none_or(|tools| tools.contains(tool)))
            .map(|(tool, entry)| (tool, entry.clone()))
            .collect();
        let report = self.run_scene(&rt, entries, Some(&scene.id));
        self.noted(report)
    }

    /// 撤销：把应用前的快照再应用一次。返回的报告里又带一份快照（可以再撤销回去）。
    pub fn undo_scene(&self, undo: SceneUndo) -> ApiResult<SceneReport> {
        let snapshot = Scene {
            name: "undo".into(),
            tools: undo.tools,
            ..Scene::default()
        };
        let entries: Vec<_> = snapshot
            .entries()
            .into_iter()
            .map(|(tool, entry)| (tool, entry.clone()))
            .collect();
        if !entries.is_empty() {
            scene::validate(&snapshot).map_err(|e| self.err(e))?;
        }
        let _guard = self.write_guard()?;
        let report = self.run_scene(&self.runtime(), entries, undo.last.as_deref());
        self.noted(report)
    }

    fn run_scene(
        &self,
        rt: &Runtime,
        entries: Vec<(Tool, SceneTool)>,
        last: Option<&str>,
    ) -> ApiResult<SceneReport> {
        let mut results = Vec::new();
        let mut settled = Vec::new();
        for (tool, entry) in entries {
            match self.settle(rt, tool) {
                Ok(()) => settled.push((tool, entry)),
                Err(error) => results.push(failed(tool, error)),
            }
        }
        let data = store::load(&rt.env).map_err(|e| self.err(e))?;
        let mut undo = SceneUndo {
            last: data.last_scene.clone(),
            ..SceneUndo::default()
        };
        for (tool, _) in &settled {
            if let Ok(Some(entry)) = scene::capture_tool(&rt.env, &data, *tool) {
                undo.tools.insert(tool.as_str().to_string(), entry);
            }
        }
        for (tool, entry) in settled {
            results.push(self.apply_tool(rt, tool, &entry));
        }
        results.sort_by_key(|result| result.tool);
        if let Err(error) = scene::set_last(&rt.env, last) {
            log::warn!("last scene not recorded: {error}");
        }
        Ok(SceneReport { results, undo })
    }

    fn apply_tool(&self, rt: &Runtime, tool: Tool, entry: &SceneTool) -> ToolOutcome {
        // 前面的工具切换时可能改过库（指针），每个工具重新读一次。
        let plan = store::load(&rt.env)
            .and_then(|data| scene::plan_tool(&rt.env, &data, tool, entry))
            .map_err(|e| self.err(e));
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => return failed(tool, error),
        };
        let mut outcome = ToolOutcome {
            tool,
            outcome: if plan.actions.is_empty() {
                SceneOutcome::Unchanged
            } else {
                SceneOutcome::Applied
            },
            changed: Vec::new(),
            warnings: Vec::new(),
            missing: plan.missing,
            error: None,
        };
        for action in plan.actions {
            let result = match &action {
                Action::Switch(id) | Action::Activate(id) => self.switch_locked(rt, tool, id),
                Action::Enable(id) => self.set_in_live_locked(rt, tool, id, true),
                Action::Disable(id) => self.set_in_live_locked(rt, tool, id, false),
            };
            match result {
                Ok(result) => {
                    outcome.changed.extend(result.changed);
                    outcome.warnings.extend(result.warnings);
                }
                Err(error) => {
                    outcome.outcome = SceneOutcome::Failed;
                    outcome.error = Some(error.0);
                    break;
                }
            }
        }
        outcome.changed.sort();
        outcome.changed.dedup();
        outcome
    }
}

#[cfg(test)]
mod tests;

fn action_id(action: &Action) -> &str {
    match action {
        Action::Switch(id) | Action::Enable(id) | Action::Disable(id) | Action::Activate(id) => id,
    }
}

fn failed(tool: Tool, error: ApiError) -> ToolOutcome {
    ToolOutcome {
        tool,
        outcome: SceneOutcome::Failed,
        changed: Vec::new(),
        warnings: Vec::new(),
        missing: Vec::new(),
        error: Some(error.0),
    }
}
