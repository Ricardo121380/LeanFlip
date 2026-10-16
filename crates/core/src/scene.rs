//! 场景：一组工具的供应商选择，一次应用到多个工具。
//!
//! 场景存在供应商库（`providers.json` 的 `scenes`）里，只记卡片 id，不复制配置：
//! - 单选式工具记 `current`（用哪张卡）；
//! - 并存式工具记 `enabled`（哪几张卡写在客户端配置里）。应用时只管 LeanFlip 的卡：
//!   没列进来的卡移出客户端配置，用户在工具里自己加的条目不动。Hermes 另有 `current`，
//!   对应顶层 `model:` 指向的那家。
//!
//! 场景里没列的工具不动；工具名不认识的条目（新版本写的）原样保留、应用时跳过。
//! 场景还可以带提醒规则（见 [`crate::scene_rules`]），规则只用来提醒，不会自动应用。
//! 这里只算计划和读写场景本身，真正的切换由调用方逐个工具走现有的切换操作。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::env::Env;
use crate::error::AppError;
use crate::provider::Tool;
use crate::scene_rules::{self, SceneRule};
use crate::store::{self, StoreData};
use crate::{coexist, switch};

/// 场景名的最大长度（字符）。
pub const NAME_MAX: usize = 60;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    /// 保存时生成；新建时传空。
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// 工具名 → 这个工具在场景里的选择。键用字符串，不认识的工具不会让整个库读不出来。
    #[serde(default)]
    pub tools: BTreeMap<String, SceneTool>,
    /// 提醒规则：任意一条满足、而场景还没在用时提醒切换。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<SceneRule>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// 一个场景最多几条提醒规则。
pub const RULES_MAX: usize = 20;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneTool {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl SceneTool {
    pub fn current(id: impl Into<String>) -> Self {
        Self {
            current: Some(id.into()),
            ..Self::default()
        }
    }

    pub fn enabled<I: IntoIterator<Item = S>, S: Into<String>>(ids: I) -> Self {
        Self {
            enabled: Some(ids.into_iter().map(Into::into).collect()),
            ..Self::default()
        }
    }

    /// 场景引用的所有卡片 id。
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.current
            .iter()
            .chain(self.enabled.iter().flatten())
            .map(String::as_str)
    }
}

impl Scene {
    /// 认识的工具和它们的选择（按 [`Tool::ALL`] 的顺序）。
    pub fn entries(&self) -> Vec<(Tool, &SceneTool)> {
        Tool::ALL
            .iter()
            .filter_map(|&tool| self.tools.get(tool.as_str()).map(|entry| (tool, entry)))
            .collect()
    }
}

/// 撤销快照里 Hermes 的顶层 `model:` 整段（见 [`undo_entry`]）。
pub const HERMES_MODEL: &str = "hermesModel";

/// 让一个工具和场景一致要做的一步。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "id")]
pub enum Action {
    /// 单选式工具：切到这张卡。
    Switch(String),
    /// 并存式工具：写进客户端配置。
    Enable(String),
    /// 并存式工具：移出客户端配置（卡留着）。
    Disable(String),
    /// Hermes：写进客户端配置并把顶层 `model:` 指向它。
    Activate(String),
    /// Hermes：把顶层 `model:` 整段写回撤销快照里的样子。
    RestoreDefault(Value),
}

/// 一个工具的计划。`actions` 和 `missing` 都为空时这个工具已经和场景一致。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPlan {
    pub actions: Vec<Action>,
    /// 场景引用、但已经被删掉的卡。
    pub missing: Vec<String>,
}

impl ToolPlan {
    pub fn in_sync(&self) -> bool {
        self.actions.is_empty() && self.missing.is_empty()
    }
}

fn invalid(zh: impl Into<String>, en: impl Into<String>) -> AppError {
    AppError::localized("scene.invalid", zh, en)
}

/// 检查场景本身的形状（不看卡片还在不在）。
pub fn validate(scene: &Scene) -> Result<(), AppError> {
    let name = scene.name.trim();
    if name.is_empty() {
        return Err(invalid("场景名不能为空", "Scene name must not be empty"));
    }
    if name.chars().count() > NAME_MAX {
        return Err(invalid(
            format!("场景名最多 {NAME_MAX} 个字"),
            format!("Scene names are limited to {NAME_MAX} characters"),
        ));
    }
    if scene.rules.len() > RULES_MAX {
        return Err(invalid(
            format!("一个场景最多 {RULES_MAX} 条提醒规则"),
            format!("A scene can have at most {RULES_MAX} reminder rules"),
        ));
    }
    for rule in &scene.rules {
        scene_rules::validate(rule)?;
    }
    let entries = scene.entries();
    if entries.is_empty() {
        return Err(invalid(
            "场景至少要包含一个工具",
            "A scene needs at least one tool",
        ));
    }
    for (tool, entry) in entries {
        if entry.ids().any(|id| id.trim().is_empty()) {
            return Err(invalid(
                format!("{tool} 的供应商 id 不能为空"),
                format!("{tool} provider ids must not be empty"),
            ));
        }
        if !tool.is_coexist() {
            if entry.current.is_none() || entry.enabled.is_some() {
                return Err(invalid(
                    format!("{tool} 在场景里要选一个供应商"),
                    format!("Choose one {tool} provider for the scene"),
                ));
            }
            continue;
        }
        let Some(enabled) = &entry.enabled else {
            return Err(invalid(
                format!("{tool} 在场景里要列出启用的供应商"),
                format!("List the enabled {tool} providers for the scene"),
            ));
        };
        let unique = enabled.iter().collect::<BTreeSet<_>>();
        if unique.len() != enabled.len() {
            return Err(invalid(
                format!("{tool} 的供应商重复了"),
                format!("{tool} providers are listed twice"),
            ));
        }
        match (&entry.current, tool) {
            (None, _) => {}
            (Some(current), Tool::Hermes) if enabled.contains(current) => {}
            (Some(_), Tool::Hermes) => {
                return Err(invalid(
                    "Hermes 的默认供应商必须在启用列表里",
                    "The Hermes default provider must be enabled",
                ))
            }
            (Some(_), _) => {
                return Err(invalid(
                    format!("{tool} 没有默认供应商"),
                    format!("{tool} has no default provider"),
                ))
            }
        }
    }
    Ok(())
}

/// 算出一个工具要做哪些步骤才能和场景一致。只读。
pub fn plan_tool(
    env: &Env,
    data: &StoreData,
    tool: Tool,
    entry: &SceneTool,
) -> Result<ToolPlan, AppError> {
    let mut plan = ToolPlan {
        missing: entry
            .ids()
            .filter(|id| data.provider(tool, id).is_none())
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        ..ToolPlan::default()
    };
    if !tool.is_coexist() {
        let Some(target) = entry
            .current
            .as_deref()
            .and_then(|id| data.provider(tool, id))
        else {
            return Ok(plan);
        };
        let selected = data.current(tool).is_some_and(|p| p.id == target.id);
        // 客户端文件被手动改过、和指针对不上时也要重写一次。
        if !selected || !switch::status::matches_live(env, data, tool, target).unwrap_or(false) {
            plan.actions.push(Action::Switch(target.id.clone()));
        }
        return Ok(plan);
    }
    let live = coexist::live_entries(env, tool)?;
    let wanted = entry.enabled.iter().flatten().collect::<BTreeSet<_>>();
    // 撤销快照带着原来的 `model:` 整段（`null` 表示原来没有）：最后原样写回，不再按
    // `current` 激活。
    let restore = (tool == Tool::Hermes)
        .then(|| entry.extra.get(HERMES_MODEL))
        .flatten()
        .filter(|model| model.is_object() || model.is_null());
    let active_now = coexist::active_entry(env, tool)?;
    let active = entry
        .current
        .as_deref()
        .filter(|id| restore.is_none() && data.provider(tool, id).is_some());
    let activate = active.is_some_and(|id| active_now.as_deref() != Some(id));
    // 「不改默认」或者指定的默认卡已经删了时，现在的默认供应商不能移出：Hermes 的
    // `model:` 会指向不存在的条目。
    let keep = (active.is_none() && restore.is_none())
        .then_some(active_now.as_deref())
        .flatten();
    for card in data.providers(tool) {
        let want = wanted.contains(&card.id);
        let have = live.contains_key(&card.id);
        if want && activate && active == Some(card.id.as_str()) {
            plan.actions.push(Action::Activate(card.id.clone()));
        } else if want && !have {
            plan.actions.push(Action::Enable(card.id.clone()));
        } else if !want && have && keep != Some(card.id.as_str()) {
            plan.actions.push(Action::Disable(card.id.clone()));
        }
    }
    if let Some(model) = restore {
        let live = coexist::hermes_model(env)?.unwrap_or(Value::Null);
        if &live != model {
            plan.actions.push(Action::RestoreDefault(model.clone()));
        }
    }
    Ok(plan)
}

/// 把一个工具现在的状态记成场景条目。没有可记的（没有卡；单选式没有当前卡，或者客户端
/// 文件被手动改过、已经不是当前卡）时返回 `None`。客户端配置读不了时报错，不悄悄漏掉。
pub fn capture_tool(
    env: &Env,
    data: &StoreData,
    tool: Tool,
) -> Result<Option<SceneTool>, AppError> {
    if !tool.is_coexist() {
        let Some(card) = data.current(tool) else {
            return Ok(None);
        };
        return Ok(switch::status::matches_live(env, data, tool, card)?
            .then(|| SceneTool::current(&card.id)));
    }
    let cards = data.providers(tool);
    if cards.is_empty() {
        return Ok(None);
    }
    let live = coexist::live_entries(env, tool)?;
    let enabled = cards
        .iter()
        .filter(|card| live.contains_key(&card.id))
        .map(|card| card.id.clone())
        .collect::<Vec<_>>();
    let current = coexist::active_entry(env, tool)?.filter(|id| enabled.contains(id));
    Ok(Some(SceneTool {
        current,
        enabled: Some(enabled),
        extra: Map::new(),
    }))
}

/// 撤销用的快照：和 [`capture_tool`] 一样，Hermes 另外记下顶层 `model:` 整段，撤销时
/// 原样写回（原来的默认供应商可能不是 LeanFlip 的卡）。
pub fn undo_entry(env: &Env, data: &StoreData, tool: Tool) -> Result<Option<SceneTool>, AppError> {
    let mut entry = capture_tool(env, data, tool)?;
    if tool == Tool::Hermes {
        if let Some(entry) = entry.as_mut() {
            let model = coexist::hermes_model(env)?.unwrap_or(Value::Null);
            entry.extra.insert(HERMES_MODEL.into(), model);
        }
    }
    Ok(entry)
}

/// 场景里所有认识的工具是否都已经一致（卡片缺失也算不一致）。读不了客户端配置时算不一致；
/// 一个认识的工具都没有（只有新版本写的工具）时不算一致。
pub fn matches(env: &Env, data: &StoreData, scene: &Scene) -> bool {
    let entries = scene.entries();
    !entries.is_empty()
        && entries
            .into_iter()
            .all(|(tool, entry)| plan_tool(env, data, tool, entry).is_ok_and(|plan| plan.in_sync()))
}

/// 按 id 或名字找场景（名字忽略首尾空白和大小写）。
pub fn find<'a>(data: &'a StoreData, key: &str) -> Option<&'a Scene> {
    let key = key.trim();
    data.scenes
        .iter()
        .find(|scene| scene.id == key)
        .or_else(|| data.scenes.iter().find(|scene| same_name(&scene.name, key)))
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

fn new_id(data: &StoreData) -> Result<String, AppError> {
    loop {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes).map_err(|_| AppError::Message("无法生成场景 id".into()))?;
        let id = format!(
            "scene-{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        if !data.scenes.iter().any(|scene| scene.id == id) {
            return Ok(id);
        }
    }
}

/// 新建（id 为空）或替换同 id 的场景，返回保存后的样子。名字不能和别的场景重复。
pub fn save(env: &Env, mut scene: Scene) -> Result<Scene, AppError> {
    validate(&scene)?;
    scene.name = scene.name.trim().to_string();
    store::update(env, |data| {
        if data
            .scenes
            .iter()
            .any(|other| other.id != scene.id && same_name(&other.name, &scene.name))
        {
            return Err(AppError::localized(
                "scene.duplicate_name",
                format!("已经有一个叫「{}」的场景", scene.name),
                format!("A scene named '{}' already exists", scene.name),
            ));
        }
        if scene.id.is_empty() {
            scene.id = new_id(data)?;
            data.scenes.push(scene.clone());
        } else {
            let slot = data
                .scenes
                .iter_mut()
                .find(|other| other.id == scene.id)
                .ok_or_else(|| not_found(&scene.id))?;
            *slot = scene.clone();
        }
        Ok(scene)
    })
}

fn not_found(id: &str) -> AppError {
    AppError::localized(
        "scene.not_found",
        format!("找不到场景 {id}"),
        format!("Scene {id} was not found"),
    )
}

/// 删除场景。它是最近应用的场景时一并清掉记录。
pub fn remove(env: &Env, id: &str) -> Result<Scene, AppError> {
    store::update(env, |data| {
        let index = data
            .scenes
            .iter()
            .position(|scene| scene.id == id)
            .ok_or_else(|| not_found(id))?;
        if data.last_scene.as_deref() == Some(id) {
            data.last_scene = None;
        }
        Ok(data.scenes.remove(index))
    })
}

/// 记下最近一次应用的场景（界面用它判断「已偏离」）。
pub fn set_last(env: &Env, id: Option<&str>) -> Result<(), AppError> {
    store::update(env, |data| {
        let id = id.filter(|id| data.scenes.iter().any(|scene| scene.id == *id));
        if data.last_scene.as_deref() != id {
            data.last_scene = id.map(str::to_string);
        }
        Ok(())
    })
}
