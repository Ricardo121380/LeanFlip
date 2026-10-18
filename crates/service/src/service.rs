//! 桌面和 CLI 共用的业务层，不依赖 Tauri。
//! 单元测试用临时 HOME 直接测这一层。
//!
//! 安全约定：从这里出去的数据都经过 [`crate::redact`]，ChatGPT 登录凭据不会到前端。

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use leanflip_core::codex::oauth::transport::OAuthTransport;
use leanflip_core::codex::oauth::{AccountInfo, CodexOAuthManager, DeviceFlow, DeviceFlowPoll};
use leanflip_core::operation::{OperationReport, RecoveryOutcome};
use leanflip_core::switch::{self, Services};
use leanflip_core::{
    ccswitch_export, ccswitch_import, coexist, import, store, AppError, Env, Provider, Tool,
};
use serde::{Deserialize, Serialize};

use crate::quota;
use crate::redact;
use crate::settings::{self, AppSettings};
use crate::usage;

mod account_quota;
mod backup;
mod connection;
mod krill_login;
mod scene;
pub use backup::{BackupItem, BackupPreview, RestoreOptions, RestoreReport, MAX_BACKUP_BYTES};
pub use connection::{connection_info, ApiKeyChange, ConnectionInfo, ConnectionUpdate};
pub use krill_login::KrillLogin;
pub use scene::{
    rule_label, SceneHintUpdate, SceneList, SceneMissing, SceneOutcome, ScenePreview, SceneReport,
    SceneStatus, SceneStep, SceneSuggestion, SceneUndo, SceneView, ToolOutcome, ToolPreview,
};

/// 给前端的错误：稳定错误码用于恢复指引，原始消息保留用于诊断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError(pub String, &'static str);

impl Serialize for ApiError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut value = serializer.serialize_struct("ApiError", 2)?;
        value.serialize_field("message", &self.0)?;
        value.serialize_field("code", self.1)?;
        value.end()
    }
}

impl ApiError {
    pub fn from_core(error: AppError, english: bool) -> Self {
        let code = match &error {
            AppError::Config(_) => "config",
            AppError::InvalidInput(_) => "invalid_input",
            AppError::Conflict(_) => "conflict",
            AppError::Io { .. } | AppError::IoContext { .. } => "io",
            AppError::Json { .. } => "json",
            AppError::JsonSerialize { .. } => "serialization",
            AppError::NotFound(_) => "not_found",
            AppError::Localized { key, .. } => *key,
            AppError::Message(_) => "operation_failed",
        };
        let message = match error {
            AppError::Localized { zh, en, .. } => {
                if english {
                    en
                } else {
                    zh
                }
            }
            other => other.to_string(),
        };
        Self(message, code)
    }

    pub fn message(message: String) -> Self {
        Self(message, "operation_failed")
    }

    pub fn code(&self) -> &'static str {
        self.1
    }
}

impl std::error::Error for ApiError {}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
pub type AccountRefreshResults = Vec<(String, Result<bool, String>)>;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToolKind {
    /// 同一时间只有一张卡生效（当前指针）。
    Switch,
    /// 多张卡同时写在客户端配置里（开关）。
    Coexist,
}

fn kind(tool: Tool) -> ToolKind {
    if tool.is_coexist() {
        ToolKind::Coexist
    } else {
        ToolKind::Switch
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSummary {
    pub tool: Tool,
    pub kind: ToolKind,
    pub paths: Vec<PathBuf>,
    /// 客户端配置文件至少有一个存在。
    pub detected: bool,
    pub card_count: usize,
    pub current: Option<String>,
    pub selected: Option<String>,
    pub current_name: Option<String>,
    pub in_live: Vec<String>,
    /// false 表示并存配置无法读取，不能把空列表当作全部停用。
    pub live_readable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolProviders {
    pub tool: Tool,
    pub kind: ToolKind,
    pub providers: Vec<Provider>,
    pub current: Option<String>,
    pub selected: Option<String>,
    pub in_live: Vec<String>,
    /// 客户端配置读不了（格式坏了等）时的说明；开关状态这时不可信。
    pub live_error: Option<String>,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpResult {
    pub changed: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

fn recovery_message(outcome: &RecoveryOutcome, english: bool) -> String {
    let (zh, en, paths) = match outcome {
        RecoveryOutcome::Discarded => (
            "已丢弃尚未开始写入的上次操作，客户端文件未被改动。",
            "Discarded an unfinished operation that had not started writing. Client files were unchanged.",
            &[][..],
        ),
        RecoveryOutcome::RolledForward => (
            "已补完上次未完成的写入。",
            "Completed the previous unfinished write.",
            &[][..],
        ),
        RecoveryOutcome::RolledForwardExcept { paths } => (
            "上次写入仅部分恢复。以下文件被外部修改或临时文件不可用，已保持原样，请检查当前配置：",
            "The previous write was only partially recovered. These files were changed externally or their staged files were unavailable; they were left unchanged. Check the current config:",
            paths.as_slice(),
        ),
        RecoveryOutcome::Abandoned { paths } => (
            "上次操作在写入前遇到外部修改，已放弃。以下文件保持原样：",
            "Abandoned the previous operation because files changed before writing began. These files were left unchanged:",
            paths.as_slice(),
        ),
    };
    std::iter::once(if english {
        en.to_string()
    } else {
        zh.to_string()
    })
    .chain(paths.iter().map(|p| p.display().to_string()))
    .collect::<Vec<_>>()
    .join("\n")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportLiveResult {
    pub created: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum PollResult {
    Pending,
    Added { account: AccountInfo },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub app: AppSettings,
    pub keep_codex_login: bool,
    pub data_dir: PathBuf,
    pub home: PathBuf,
    /// 当前生效的各工具配置文件（套过覆盖）。
    pub live_paths: BTreeMap<String, Vec<PathBuf>>,
    /// 当前生效的各目录，键和 `DirOverrides` 的字段一致（设置页的占位文字）。
    pub dirs: BTreeMap<String, PathBuf>,
    pub ccswitch_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInput {
    pub app: AppSettings,
    pub keep_codex_login: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSettingsResult {
    /// 目录覆盖改了（已经生效；调用方要重新配置文件监听）。
    pub dirs_changed: bool,
}

fn effective_dirs(env: &Env) -> BTreeMap<String, PathBuf> {
    [
        ("claudeDir", &env.claude_dir),
        ("codexDir", &env.codex_dir),
        ("geminiDir", &env.gemini_dir),
        ("grokDir", &env.grok_dir),
        ("claudeDesktopRoot", &env.claude_desktop_root),
        ("opencodeDir", &env.opencode_dir),
        ("openclawDir", &env.openclaw_dir),
        ("hermesDir", &env.hermes_dir),
        ("piAgentDir", &env.pi_agent_dir),
        ("mcodeDir", &env.mcode_dir),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.clone()))
    .collect()
}

/// 当前生效的环境（套过目录覆盖）和托管账号管理器。改目录覆盖时整体换掉，正在进行的
/// 操作继续用它拿到的那一份。
pub struct Runtime {
    pub env: Env,
    pub oauth: Arc<CodexOAuthManager>,
}

pub struct Service {
    /// 没套覆盖的环境。
    base: Env,
    transport: Option<Arc<dyn OAuthTransport>>,
    runtime: RwLock<Arc<Runtime>>,
    settings: Mutex<AppSettings>,
    recovery: Mutex<Vec<String>>,
    /// 读进来、等用户确认的 cc-switch 导出文件。
    staged_export: Mutex<Option<ccswitch_export::ParsedExport>>,
    staged_backup: Arc<Mutex<backup::BackupCache>>,
    /// 最近一次联网查询额度失败，按来源记。只在内存里，重启后清空。
    quota_errors: Mutex<HashMap<String, quota::QuotaError>>,
    /// 每个托管账号上次联网查询额度的时间，用来挡住连续点击。
    quota_attempts: Mutex<HashMap<String, i64>>,
    krill_logins: Mutex<HashMap<String, krill_login::PendingLogin>>,
    quota_url: String,
    scene_hints: Mutex<scene::SceneHints>,
}

fn services(rt: &Runtime) -> Services<'_> {
    Services {
        codex_oauth: Some(&rt.oauth),
    }
}

fn recover(env: &Env, english: bool) -> Vec<String> {
    let lines = switch::recover_on_startup(env)
        .into_iter()
        .map(|(app, outcome)| match outcome {
            Ok(outcome) => format!("{app}: {}", recovery_message(&outcome, english)),
            Err(err) => recovery_error(&app, ApiError::from_core(err, english), english),
        })
        .collect::<Vec<_>>();
    for line in &lines {
        log::info!("recovery {line}");
    }
    lines
}

fn recovery_error(app: &str, error: ApiError, english: bool) -> String {
    if english {
        format!("{app}: Recovery failed; check the client config before retrying. {error}")
    } else {
        format!("{app}: 恢复失败，请检查客户端配置后重试。{error}")
    }
}

impl Service {
    /// `base` 是没套覆盖的环境（`Env::from_system()` 或测试的临时 HOME）。会先补完 /
    /// 放弃上次没做完的写入。
    pub fn start(base: Env, transport: Option<Arc<dyn OAuthTransport>>) -> Result<Self, AppError> {
        // Do not recover or write against default client paths when overrides cannot be read.
        let app_settings = settings::load(&base.data_dir)?;
        let service = Self::initialize(base, transport, app_settings)?;
        let env = service.env();
        let _guard = env.device_store().lock_app("service")?;
        let notices = recover(&env, service.english());
        service
            .recovery
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(notices);
        Ok(service)
    }

    /// CLI 打开环境时只读设置和账号快照。查询不恢复 pending，也不创建目录或锁文件。
    /// 设置损坏时停止，不能悄悄使用默认路径写入另一套配置。
    pub fn open(base: Env, transport: Option<Arc<dyn OAuthTransport>>) -> Result<Self, AppError> {
        let app_settings = settings::load(&base.data_dir)?;
        Self::initialize(base, transport, app_settings)
    }

    fn initialize(
        base: Env,
        transport: Option<Arc<dyn OAuthTransport>>,
        app_settings: AppSettings,
    ) -> Result<Self, AppError> {
        let env = base.clone().with_overrides(&app_settings.dir_overrides);
        let oauth = Arc::new(CodexOAuthManager::new(&env, transport.clone())?);
        Ok(Self {
            base,
            transport,
            runtime: RwLock::new(Arc::new(Runtime { env, oauth })),
            settings: Mutex::new(app_settings),
            recovery: Mutex::new(Vec::new()),
            staged_export: Mutex::new(None),
            staged_backup: Arc::new(Mutex::new(backup::BackupCache::default())),
            quota_errors: Mutex::new(HashMap::new()),
            quota_attempts: Mutex::new(HashMap::new()),
            krill_logins: Mutex::new(HashMap::new()),
            quota_url: quota::codex_remote::USAGE_URL.to_string(),
            scene_hints: Mutex::new(scene::SceneHints::default()),
        })
    }

    /// 当前生效的运行环境（快照）。
    pub fn runtime(&self) -> Arc<Runtime> {
        self.runtime
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn env(&self) -> Env {
        self.runtime().env.clone()
    }

    /// 串行整个高层操作（读取当前卡、OAuth 准备、写 live、落库），不只保护最后的 rename。
    /// App 的后台刷新也用同一把跨进程锁。锁顺序：service → OAuth/tool → state/store。
    fn write_guard(&self) -> ApiResult<leanflip_core::live::engine::AppWriteGuard> {
        self.env()
            .device_store()
            .lock_app("service")
            .map_err(|e| self.err(e))
    }

    pub fn recover_tool(&self, tool: Tool) -> ApiResult<Vec<String>> {
        let _guard = self.write_guard()?;
        let before = self.recovery().len();
        self.settle(&self.runtime(), tool)?;
        Ok(self.recovery()[before..].to_vec())
    }

    pub fn recover_all(&self) -> ApiResult<Vec<String>> {
        let _guard = self.write_guard()?;
        let before = self.recovery().len();
        let rt = self.runtime();
        for tool in Tool::ALL {
            self.settle(&rt, tool)?;
        }
        Ok(self.recovery()[before..].to_vec())
    }

    /// 套用新的目录覆盖，立即生效。Codex 目录没变时沿用同一个账号管理器（进行中的设备码
    /// 登录不受影响）；变了就新建一个（进行中的登录要重新开始）。返回目录是否有变化。
    fn apply_overrides(
        &self,
        overrides: &leanflip_core::env::DirOverrides,
    ) -> Result<bool, AppError> {
        let current = self.runtime();
        let env = self.base.clone().with_overrides(overrides);
        if env == current.env {
            return Ok(false);
        }
        let oauth = if env.codex_dir == current.env.codex_dir {
            current.oauth.clone()
        } else {
            Arc::new(CodexOAuthManager::new(&env, self.transport.clone())?)
        };
        // 新目录下如果有别的实例留下的未完成操作，先补完 / 放弃。
        let notices = recover(&env, self.english());
        self.recovery
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(notices);
        *self.runtime.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(Runtime { env, oauth });
        Ok(true)
    }

    /// 非破坏性读取：重试、窗口重新挂载和查询刷新不会丢失恢复提示。
    pub fn recovery(&self) -> Vec<String> {
        self.recovery
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 入口先落定再读取卡片；恢复提示独立保存，后续操作失败也不会丢失。
    fn settle(&self, rt: &Runtime, tool: Tool) -> ApiResult<()> {
        let line = match switch::settle(&rt.env, tool) {
            Ok(None) => return Ok(()),
            Ok(Some(outcome)) => format!("{tool}: {}", recovery_message(&outcome, self.english())),
            Err(err) => {
                let error = self.err(err);
                let line = recovery_error(tool.as_str(), error.clone(), self.english());
                let mut notices = self.recovery.lock().unwrap_or_else(|e| e.into_inner());
                if notices.last() != Some(&line) {
                    notices.push(line);
                }
                return Err(error);
            }
        };
        self.recovery
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(line);
        Ok(())
    }

    fn op_result(&self, report: impl Into<Option<OperationReport>>) -> OpResult {
        let Some(report) = report.into() else {
            return OpResult::default();
        };
        let mut warnings = report.warnings;
        if let Some(outcome) = report.recovered {
            warnings.push(recovery_message(&outcome, self.english()));
        }
        OpResult {
            changed: report.changed,
            warnings,
        }
    }

    fn provider_notices(&self, tool: Tool, provider: &Provider) -> Vec<String> {
        switch::provider_warnings(tool, provider)
            .into_iter()
            .map(|reason| if self.english() { reason.en } else { reason.zh })
            .collect()
    }

    pub fn credential_commands(
        &self,
        tool: Tool,
        settings: &serde_json::Value,
    ) -> ApiResult<leanflip_core::command_trust::CommandPreview> {
        leanflip_core::command_trust::preview(&self.env(), tool, settings).map_err(|e| self.err(e))
    }

    pub fn approve_credential_commands(
        &self,
        tool: Tool,
        settings: &serde_json::Value,
        fingerprint: &str,
    ) -> ApiResult<()> {
        let _guard = self.write_guard()?;
        leanflip_core::command_trust::approve(&self.env(), tool, settings, fingerprint)
            .map_err(|e| self.err(e))
    }

    /// 编辑器实时提示：这份配置里不会写入客户端的字段。只读计算，不碰磁盘。
    pub fn claude_onboarding_path(&self) -> PathBuf {
        self.env().home.join(".claude.json")
    }
    pub fn complete_claude_onboarding(
        &self,
        expected_path: &std::path::Path,
    ) -> ApiResult<OpResult> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        if expected_path != rt.env.home.join(".claude.json") {
            return Err(self.err(AppError::Conflict(
                "Claude onboarding path changed; review again".into(),
            )));
        }
        self.settle(&rt, Tool::Claude)?;
        self.noted(
            switch::claude::complete_onboarding(&rt.env)
                .map(|report| self.op_result(report))
                .map_err(|e| self.err(e)),
        )
    }

    pub fn ignored_fields(&self, tool: Tool, settings_config: &serde_json::Value) -> Vec<String> {
        match tool {
            Tool::Claude => leanflip_core::live::project::claude::ignored_keys(settings_config),
            _ => Vec::new(),
        }
    }

    /// 测连通：只访问请求地址，不带凭据。由用户点击触发。
    pub fn probe_latency(
        &self,
        tool: Tool,
        settings_config: &serde_json::Value,
    ) -> crate::probe::ProbeResult {
        crate::probe::latency(tool, settings_config)
    }

    /// 获取模型列表：用配置里的密钥请求供应商的 models 接口。
    pub fn probe_models(
        &self,
        tool: Tool,
        settings_config: &serde_json::Value,
    ) -> crate::probe::ProbeResult {
        crate::probe::models(tool, settings_config)
    }

    /// 检测可用的推理强度：每一档各发一次最短请求。
    pub fn probe_efforts(
        &self,
        tool: Tool,
        settings_config: &serde_json::Value,
        model: Option<&str>,
    ) -> crate::probe::ProbeResult {
        crate::probe::efforts(tool, settings_config, model)
    }

    /// 检查模型：发送一次最短的真实请求。
    pub fn probe_check(
        &self,
        tool: Tool,
        settings_config: &serde_json::Value,
        model: Option<&str>,
    ) -> crate::probe::ProbeResult {
        crate::probe::check(tool, settings_config, model)
    }

    pub fn app_settings(&self) -> AppSettings {
        self.settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn english(&self) -> bool {
        self.app_settings().language.as_deref() == Some("en")
    }

    fn err(&self, error: AppError) -> ApiError {
        ApiError::from_core(error, self.english())
    }

    fn live_ids(&self, tool: Tool) -> Result<Vec<String>, AppError> {
        let rt = self.runtime();
        if !tool.is_coexist() {
            let data = store::load_readonly(&rt.env)?;
            let Some(provider) = data.current(tool) else {
                return Ok(Vec::new());
            };
            return Ok(
                if switch::status::matches_live(&rt.env, &data, tool, provider)? {
                    vec![provider.id.clone()]
                } else {
                    Vec::new()
                },
            );
        }
        Ok(coexist::live_entries(&rt.env, tool)?
            .keys()
            .cloned()
            .collect())
    }

    // ---- 查询 ----

    pub fn overview(&self) -> ApiResult<Vec<ToolSummary>> {
        let rt = self.runtime();
        let data = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        Ok(Tool::ALL
            .iter()
            .map(|&tool| {
                let paths = rt.env.live_paths(tool);
                let selected = data.current(tool);
                let live_ids = self.live_ids(tool);
                let current =
                    selected.filter(|p| live_ids.as_ref().is_ok_and(|ids| ids.contains(&p.id)));
                ToolSummary {
                    tool,
                    kind: kind(tool),
                    detected: paths.iter().any(|p| p.exists()),
                    paths,
                    card_count: data.providers(tool).len(),
                    selected: selected.map(|p| p.id.clone()),
                    current: current.map(|p| p.id.clone()),
                    current_name: current.map(|p| p.name.clone()),
                    live_readable: live_ids.is_ok(),
                    in_live: live_ids.unwrap_or_default(),
                }
            })
            .collect())
    }

    pub fn list_providers(&self, tool: Tool) -> ApiResult<ToolProviders> {
        let rt = self.runtime();
        let data = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        let (in_live, live_error) = match self.live_ids(tool) {
            Ok(ids) => (ids, None),
            Err(err) => (Vec::new(), Some(self.err(err).0)),
        };
        Ok(ToolProviders {
            tool,
            kind: kind(tool),
            providers: data
                .providers(tool)
                .iter()
                .map(|p| redact::redact(tool, p))
                .collect(),
            selected: data.current(tool).map(|p| p.id.clone()),
            current: data
                .current(tool)
                .filter(|p| tool.is_coexist() || in_live.contains(&p.id))
                .map(|p| p.id.clone()),
            in_live,
            live_error,
            paths: rt.env.live_paths(tool),
        })
    }

    // ---- 切换 / 卡片 ----

    pub fn switch_provider(&self, tool: Tool, id: &str) -> ApiResult<OpResult> {
        let _guard = self.write_guard()?;
        let result = self.switch_locked(&self.runtime(), tool, id);
        self.noted(result)
    }

    /// [`Self::switch_provider`] 的本体：调用方已经拿着写锁，也负责记切换日志。
    fn switch_locked(&self, rt: &Runtime, tool: Tool, id: &str) -> ApiResult<OpResult> {
        self.settle(rt, tool)?;
        let data = store::load(&rt.env).map_err(|e| self.err(e))?;
        let warnings = data
            .provider(tool, id)
            .map(|p| self.provider_notices(tool, p))
            .unwrap_or_default();
        switch::switch_with(&rt.env, services(rt), tool, id)
            .map(|report| {
                let mut result = self.op_result(report);
                result.warnings.extend(warnings);
                result
            })
            .map_err(|e| self.err(e))
    }

    fn check_incoming(&self, provider: &Provider) -> ApiResult<()> {
        if provider.id.trim().is_empty() {
            return Err(ApiError::message(if self.english() {
                "Provider id must not be empty".into()
            } else {
                "供应商 id 不能为空".into()
            }));
        }
        if redact::contains_placeholder(&provider.settings_config)
            || provider
                .meta
                .extra
                .get(quota::relay::META_QUOTA)
                .is_some_and(redact::contains_placeholder)
        {
            return Err(ApiError::message(if self.english() {
                "Redacted login values cannot be saved into another provider".into()
            } else {
                "隐藏的登录凭据不能保存到其他供应商".into()
            }));
        }
        if let Some(settings) = quota::relay::QuotaSettings::of(provider) {
            settings.validate().map_err(ApiError::message)?;
        }
        Ok(())
    }

    pub fn add_provider(
        &self,
        tool: Tool,
        mut provider: Provider,
        add_to_live: bool,
    ) -> ApiResult<OpResult> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        self.resolve_krill_login(&mut provider)?;
        self.check_incoming(&provider)?;
        if let (Some(settings), Some(address)) = (
            quota::relay::QuotaSettings::of(&provider),
            quota::relay::account::address(tool, &provider),
        ) {
            settings
                .query_address(&address)
                .map_err(ApiError::message)?;
        }
        if tool == Tool::Codex {
            switch::codex::validate_login_ref(&rt.env, &provider).map_err(|e| self.err(e))?;
        }
        self.settle(&rt, tool)?;
        let warnings = self.provider_notices(tool, &provider);
        if tool.is_coexist() {
            let result = coexist::add_provider(&rt.env, tool, provider, add_to_live)
                .map(|report| self.op_result(report))
                .map_err(|e| self.err(e));
            return self.noted(result);
        }
        let run = || -> Result<OpResult, AppError> {
            let data = store::load(&rt.env)?;
            switch::preflight(&data, tool, &provider)?;
            let id = provider.id.clone();
            store::add_provider(&rt.env, tool, provider)?;
            if add_to_live {
                return match switch::switch_with(&rt.env, services(&rt), tool, &id) {
                    Ok(report) => {
                        let mut result = self.op_result(report);
                        result.warnings.extend(warnings);
                        Ok(result)
                    }
                    Err(error) => {
                        if leanflip_core::operation::pending_published(
                            &rt.env.device_store(),
                            tool.as_str(),
                        )? != Some(true)
                        {
                            store::remove_provider(&rt.env, tool, &id)?;
                        }
                        Err(error)
                    }
                };
            }
            Ok(OpResult {
                warnings,
                ..OpResult::default()
            })
        };
        self.noted(run().map_err(|e| self.err(e)))
    }

    /// Attach a sync group without rewriting provider credentials or live files.
    /// Check the dialog's snapshot before any target cards are added.
    pub fn link_provider(
        &self,
        tool: Tool,
        expected: Provider,
        link: String,
    ) -> ApiResult<OpResult> {
        if link.is_empty() || link.len() > 200 {
            return Err(self.err(AppError::InvalidInput("Invalid provider link".into())));
        }
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        store::update(&rt.env, |data| {
            let stored = data
                .app_mut(tool)
                .providers
                .iter_mut()
                .find(|p| p.id == expected.id)
                .ok_or_else(|| AppError::Conflict("Source provider no longer exists".into()))?;
            let mut current = redact::redact(tool, stored);
            // A just-saved editor still holds a temporary Krill login reference;
            // compare both snapshots in their public, redacted representation.
            let mut expected = redact::redact(tool, &expected);
            // A partial sync may already have attached this exact group.
            let old_link = current.meta.extra.remove("leanflipLink");
            expected.meta.extra.remove("leanflipLink");
            if current != expected || old_link.as_ref().is_some_and(|v| v.as_str() != Some(&link)) {
                return Err(AppError::localized(
                    "conflict",
                    "来源供应商已变化，请重新打开同步窗口。",
                    "The source provider changed. Reopen the sync dialog.",
                ));
            }
            stored.meta.extra.insert(
                "leanflipLink".into(),
                serde_json::Value::String(link.clone()),
            );
            Ok(())
        })
        .map_err(|e| self.err(e))?;
        Ok(OpResult::default())
    }

    pub fn update_provider(&self, tool: Tool, mut provider: Provider) -> ApiResult<OpResult> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        self.settle(&rt, tool)?;
        let data = store::load(&rt.env).map_err(|e| self.err(e))?;
        let stored = data.provider(tool, &provider.id).cloned();
        self.check_account_origin(tool, &provider, stored.as_ref())?;
        redact::restore(tool, &mut provider, stored.as_ref());
        self.resolve_krill_login(&mut provider)?;
        self.check_incoming(&provider)?;
        if let (Some(settings), Some(address)) = (
            quota::relay::QuotaSettings::of(&provider),
            quota::relay::account::address(tool, &provider),
        ) {
            settings
                .query_address(&address)
                .map_err(ApiError::message)?;
        }
        if tool == Tool::Codex {
            switch::codex::validate_login_ref(&rt.env, &provider).map_err(|e| self.err(e))?;
        }
        let warnings = self.provider_notices(tool, &provider);
        switch::update_provider_with(&rt.env, services(&rt), tool, provider)
            .map(|report| {
                let mut result = self.op_result(report);
                result.warnings.extend(warnings);
                result
            })
            .map_err(|e| self.err(e))
    }

    pub fn delete_provider(&self, tool: Tool, id: &str) -> ApiResult<()> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        self.settle(&rt, tool)?;
        let result = if tool.is_coexist() {
            coexist::delete_provider(&rt.env, tool, id)
        } else {
            store::remove_provider(&rt.env, tool, id)
        };
        self.noted(result.map(|_| ()).map_err(|e| self.err(e)))
    }

    /// 并存式工具：写进 / 移出客户端配置。
    pub fn set_in_live(&self, tool: Tool, id: &str, enabled: bool) -> ApiResult<OpResult> {
        let _guard = self.write_guard()?;
        let result = self.set_in_live_locked(&self.runtime(), tool, id, enabled);
        self.noted(result)
    }

    /// [`Self::set_in_live`] 的本体：调用方已经拿着写锁，也负责记切换日志。
    fn set_in_live_locked(
        &self,
        rt: &Runtime,
        tool: Tool,
        id: &str,
        enabled: bool,
    ) -> ApiResult<OpResult> {
        self.settle(rt, tool)?;
        let result = if enabled {
            coexist::write_to_live(&rt.env, tool, id).map(|report| self.op_result(report))
        } else {
            coexist::remove_from_live(&rt.env, tool, id).map(|report| self.op_result(report))
        };
        result.map_err(|e| self.err(e))
    }

    pub fn import_live(&self, tool: Tool) -> ApiResult<ImportLiveResult> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        self.settle(&rt, tool)?;
        let result = import::import_live(&rt.env, tool)
            .map(|r| ImportLiveResult {
                created: r.created,
                warnings: r.warnings,
            })
            .map_err(|e| self.err(e));
        self.noted(result)
    }

    fn observed_usage_data(&self, mut data: store::StoreData) -> store::StoreData {
        for tool in Tool::ALL {
            if !tool.is_coexist()
                && data.current(tool).is_some()
                && self
                    .live_ids(tool)
                    .is_ok_and(|ids| !data.current(tool).is_some_and(|p| ids.contains(&p.id)))
            {
                data.app_mut(tool).current = None;
            }
        }
        data
    }

    /// 操作成功后记下切换记录的变化。记录失败只写日志，不影响刚完成的操作。
    fn noted<T>(&self, result: ApiResult<T>) -> ApiResult<T> {
        if result.is_ok() {
            let rt = self.runtime();
            let recorded = store::load_readonly(&rt.env).and_then(|data| {
                usage::record_state(&rt.env, &self.observed_usage_data(data), &|tool| {
                    self.live_ids(tool).ok()
                })
            });
            if let Err(error) = recorded {
                log::warn!("switch log not updated: {error}");
            }
        }
        result
    }

    // ---- 用量统计 ----

    /// 扫描各工具新增的会话记录，返回最近 180 天的汇总和切换记录。只读数字，不读对话内容。
    pub fn usage_report(&self) -> ApiResult<usage::UsageReport> {
        let rt = self.runtime();
        let data = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        usage::load(&rt.env, &self.observed_usage_data(data), &|tool| {
            self.live_ids(tool).ok()
        })
        .map_err(|e| self.err(e))
    }

    // ---- 额度 ----

    /// 额度总览：Codex 官方登录的窗口和 credits（本机会话记录加缓存），以及设置了平台类型
    /// 的中转卡的余额（缓存）。不联网。
    pub fn quota_overview(&self) -> ApiResult<quota::QuotaView> {
        self.quota_view(Vec::new(), Vec::new())
    }

    /// 总览，再加上刚查到、可能没能写进缓存的结果。
    fn quota_view(
        &self,
        fresh: Vec<quota::Observation>,
        relays: Vec<quota::cache::RelayEntry>,
    ) -> ApiResult<quota::QuotaView> {
        let rt = self.runtime();
        let data = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        let logins = switch::codex::available_logins(&rt.env).unwrap_or_else(|err| {
            log::warn!("quota: Codex logins unreadable: {err}");
            Vec::new()
        });
        let holders = quota::holders(&rt.oauth, &logins);
        let default = rt.oauth.try_default_account_id().ok().flatten();
        let cards = quota::codex_cards(&data, &holders, default.as_deref());
        let now = quota::now_ms();
        let mut seen = quota::codex_log::read(&rt.env.codex_dir, now);
        seen.extend(quota::cache::load(&rt.env, now));
        seen.extend(fresh);
        let errors = self
            .quota_errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut view = quota::build_view(holders, cards, seen, &errors, now);
        let mut entries = quota::cache::load_relay_entries(&rt.env, now);
        entries.extend(relays.into_iter().map(|e| (e.key.clone(), e)));
        let cached = entries
            .iter()
            .map(|(key, e)| (key.clone(), e.snapshot.clone()))
            .collect();
        let mut cards = self.relay_cards(&data);
        for card in &mut cards {
            card.adapter = card.adapter.or_else(|| {
                entries
                    .get(&card.key)
                    .and_then(|e| e.adapter)
                    .filter(|a| *a != quota::relay::RelayAdapter::Krill)
            });
        }
        view.sources
            .extend(quota::relay::relay_sources(&cards, &cached, &errors));
        view.busiest = quota::busiest(&view.sources);
        Ok(view)
    }

    /// 可查询的中转卡（包括自动识别）。切换式工具看当前指针，并存式工具看是否写进了客户端。
    fn relay_cards(&self, data: &store::StoreData) -> Vec<quota::relay::RelayCard> {
        let coexist: HashMap<Tool, Vec<String>> = Tool::ALL
            .into_iter()
            .filter(|tool| tool.is_coexist())
            .map(|tool| (tool, self.live_ids(tool).unwrap_or_default()))
            .collect();
        let providers: Vec<(Tool, &Provider)> = Tool::ALL
            .into_iter()
            .flat_map(|tool| data.providers(tool).iter().map(move |p| (tool, p)))
            .collect();
        let active = |tool: Tool, id: &str| match coexist.get(&tool) {
            Some(live) => live.iter().any(|x| x == id),
            None => data.current(tool).is_some_and(|p| p.id == id),
        };
        quota::relay::relay_cards(&providers, &active)
    }

    /// 同一来源一分钟内只联网查一次。
    fn quota_attempt(&self, key: &str) -> bool {
        const MIN_INTERVAL_MS: i64 = 60_000;
        let now = quota::now_ms();
        let mut attempts = self
            .quota_attempts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if attempts
            .get(key)
            .is_some_and(|last| now - last < MIN_INTERVAL_MS)
        {
            return false;
        }
        attempts.insert(key.to_string(), now);
        true
    }

    fn quota_outcome(&self, key: String, outcome: Result<(), quota::QuotaError>) {
        let mut errors = self.quota_errors.lock().unwrap_or_else(|e| e.into_inner());
        match outcome {
            Ok(()) => {
                errors.remove(&key);
            }
            Err(error) => {
                log::info!("quota: {key} refresh failed: {}", error.code);
                errors.insert(key, error);
            }
        }
    }

    /// 联网查询托管账号和中转卡；未设置平台时自动识别并缓存结果（`keys` 为空时查全部，否则只查这些来源），
    /// 然后返回最新的总览。需要重新登录的账号不查。
    pub fn quota_refresh(&self, keys: Option<Vec<String>>) -> ApiResult<quota::QuotaView> {
        let rt = self.runtime();
        let wanted = |key: &str| {
            keys.as_ref()
                .is_none_or(|keys| keys.iter().any(|k| k == key))
        };
        let failure = |code: &str, status: Option<u16>| quota::QuotaError {
            code: code.to_string(),
            status,
            at: quota::now_ms(),
        };
        let client = quota::codex_remote::client().ok();
        let mut fresh = Vec::new();
        let infos = rt.oauth.try_list_accounts().map_err(|e| self.err(e))?;
        for info in infos {
            let Some(identity) = quota::account_identity(&rt.oauth, &info.id) else {
                continue;
            };
            let key = identity.key();
            if !wanted(&key) || !self.quota_attempt(&key) {
                continue;
            }
            let outcome = if info.reauth_required {
                Err(failure("reauth", None))
            } else {
                // 取 token 可能要刷新登录：和切换时一样在服务锁里做，联网查询放在锁外。
                let bundle = self.write_guard().ok().and_then(|_guard| {
                    rt.oauth
                        .token_bundle(&info.id)
                        .map_err(|err| log::warn!("quota: no token for {key}: {err}"))
                        .ok()
                });
                match (bundle, client.as_ref()) {
                    (None, _) => Err(failure("token", None)),
                    (_, None) => Err(failure("network", None)),
                    (Some(bundle), Some(client)) => quota::codex_remote::fetch(
                        client,
                        &self.quota_url,
                        &bundle.access_token,
                        &bundle.workspace,
                        quota::now_ms(),
                    )
                    .map_err(|f| failure(f.code, f.status)),
                }
            };
            self.quota_outcome(
                key,
                outcome.map(|snapshot| fresh.push(quota::Observation { identity, snapshot })),
            );
        }

        let data = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        let cached = quota::cache::load_relay_entries(&rt.env, quota::now_ms());
        let mut relays: Vec<quota::cache::RelayEntry> = Vec::new();
        let mut done: Vec<String> = Vec::new();
        for card in self.relay_cards(&data) {
            if done.contains(&card.key) || !wanted(&card.key) {
                continue;
            }
            done.push(card.key.clone());
            if !self.quota_attempt(&card.key) {
                continue;
            }
            let adapter = card.adapter.or_else(|| {
                cached
                    .get(&card.key)
                    .and_then(|e| e.adapter)
                    .filter(|a| *a != quota::relay::RelayAdapter::Krill)
            });
            let outcome = match (client.as_ref(), adapter) {
                (None, _) => Err(failure("network", None)),
                (Some(client), Some(adapter)) if card.account_query => {
                    quota::relay::account::fetch(
                        client,
                        adapter,
                        &card.address,
                        &card.secret,
                        card.account_user_id,
                        card.per_usd,
                        quota::now_ms(),
                    )
                    .map(|snapshot| (adapter, snapshot))
                    .map_err(|f| failure(f.code, f.status))
                }
                (Some(client), Some(adapter)) => quota::relay::fetch(
                    client,
                    adapter,
                    &card.address,
                    &card.secret,
                    card.per_usd,
                    quota::now_ms(),
                )
                .map(|snapshot| (adapter, snapshot))
                .map_err(|f| failure(f.code, f.status)),
                (Some(client), None) => {
                    let (steps, found) = if card.account_query {
                        quota::relay::account::detect(
                            client,
                            &card.address,
                            &card.secret,
                            card.account_user_id,
                            card.per_usd,
                            quota::now_ms(),
                        )
                    } else {
                        quota::relay::detect(
                            client,
                            &card.address,
                            &card.secret,
                            card.per_usd,
                            quota::now_ms(),
                        )
                    };
                    found.ok_or_else(|| {
                        let last = steps.iter().rev().find(|s| s.code.is_some());
                        failure(
                            last.and_then(|s| s.code.as_deref()).unwrap_or("format"),
                            last.and_then(|s| s.status),
                        )
                    })
                }
            };
            let key = card.key.clone();
            self.quota_outcome(
                key.clone(),
                outcome.map(|(adapter, snapshot)| {
                    relays.push(quota::cache::RelayEntry {
                        key,
                        snapshot,
                        adapter: Some(adapter),
                    })
                }),
            );
        }
        // 缓存写不进去（磁盘满、目录不可写）时，这次返回的总览仍然用刚查到的结果。
        if let Err(err) = quota::cache::save(&rt.env, &fresh, &relays, quota::now_ms()) {
            log::warn!("quota: cache not saved: {err}");
        }
        self.quota_view(fresh, relays)
    }

    /// 自动刷新：设了刷新间隔的中转来源里，到点的联网查一次（共用一个来源的几张卡取最短的
    /// 间隔）；设置里开了托管账号自动刷新时，不用重新登录的托管 Codex 账号也一样。距上次
    /// 结果（本机会话记录里的也算）和上次尝试都满一个间隔才算到点，出错时不会每分钟重试。
    /// 查过任何来源时返回 `true`。
    pub fn quota_refresh_due(&self) -> ApiResult<bool> {
        let rt = self.runtime();
        let data = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        let mut intervals: HashMap<String, u32> = HashMap::new();
        for card in self.relay_cards(&data) {
            if card.refresh_minutes == 0 {
                continue;
            }
            let entry = intervals.entry(card.key).or_insert(card.refresh_minutes);
            *entry = (*entry).min(card.refresh_minutes);
        }
        let codex_minutes = self.app_settings().codex_quota_refresh_minutes;
        let mut accounts = Vec::new();
        if codex_minutes > 0 {
            for info in rt.oauth.try_list_accounts().unwrap_or_default() {
                if info.reauth_required {
                    continue;
                }
                if let Some(identity) = quota::account_identity(&rt.oauth, &info.id) {
                    intervals.insert(identity.key(), codex_minutes);
                    accounts.push(identity);
                }
            }
        }
        if intervals.is_empty() {
            return Ok(false);
        }
        let now = quota::now_ms();
        let mut cached: HashMap<String, i64> = quota::cache::load_relays(&rt.env, now)
            .into_iter()
            .map(|(key, snapshot)| (key, snapshot.fetched_at))
            .collect();
        if !accounts.is_empty() {
            let mut seen = quota::codex_log::read(&rt.env.codex_dir, now);
            seen.extend(quota::cache::load(&rt.env, now));
            // 只知道工作区的账号（`codex:<工作区>:*`）也认领带用户的记录，和总览里一样。
            for identity in &accounts {
                let latest = seen
                    .iter()
                    .filter(|observation| identity.covers(&observation.identity))
                    .map(|observation| observation.snapshot.fetched_at)
                    .max();
                if let Some(latest) = latest {
                    let entry = cached.entry(identity.key()).or_insert(i64::MIN);
                    *entry = (*entry).max(latest);
                }
            }
        }
        let due: Vec<String> = {
            let attempts = self
                .quota_attempts
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            intervals
                .into_iter()
                .filter(|(key, minutes)| {
                    let last = cached
                        .get(key)
                        .copied()
                        .max(attempts.get(key).copied())
                        .unwrap_or(i64::MIN);
                    now.saturating_sub(last) >= i64::from(*minutes) * 60_000
                })
                .map(|(key, _)| key)
                .collect()
        };
        if due.is_empty() {
            return Ok(false);
        }
        self.quota_refresh(Some(due))?;
        Ok(true)
    }

    /// 识别中转站的平台：用这份配置里的地址和 Key 依次试各平台的额度接口。认出来时把结果
    /// 存进缓存，但不改卡片设置（保存时再写）。只在用户点击时调用。
    pub fn quota_detect(
        &self,
        tool: Tool,
        settings_config: &serde_json::Value,
        quota_per_usd: Option<f64>,
    ) -> ApiResult<quota::relay::QuotaDetect> {
        self.quota_detect_with_base(tool, settings_config, quota_per_usd, None)
    }
    pub fn quota_detect_with_base(
        &self,
        tool: Tool,
        settings_config: &serde_json::Value,
        quota_per_usd: Option<f64>,
        query_base_url: Option<String>,
    ) -> ApiResult<quota::relay::QuotaDetect> {
        let Some((address, secret)) = crate::probe::address_and_key(tool, settings_config) else {
            return Ok(quota::relay::QuotaDetect {
                steps: Vec::new(),
                adapter: None,
                snapshot: None,
                key: None,
                error: Some("noEndpoint".into()),
            });
        };
        if quota::relay::is_krill_address(&address) {
            return Ok(quota::relay::QuotaDetect {
                steps: Vec::new(),
                adapter: None,
                snapshot: None,
                key: None,
                error: Some("noKrillJwt".into()),
            });
        }
        let address = quota::relay::QuotaSettings {
            query_base_url,
            ..Default::default()
        }
        .query_address(&address)
        .map_err(ApiError::message)?;
        let key = address.cache_key(&secret);
        let per_usd = quota::relay::QuotaSettings {
            quota_per_usd,
            ..Default::default()
        }
        .per_usd();
        let client =
            quota::codex_remote::client().map_err(|_| ApiError::message("network".into()))?;
        let (steps, found) =
            quota::relay::detect(&client, &address, &secret, per_usd, quota::now_ms());
        if let Some((adapter, snapshot)) = &found {
            let entry = quota::cache::RelayEntry {
                key: key.clone(),
                snapshot: snapshot.clone(),
                adapter: Some(*adapter),
            };
            if let Err(err) = quota::cache::save(&self.env(), &[], &[entry], quota::now_ms()) {
                log::warn!("quota: cache not saved: {err}");
            }
            self.quota_outcome(key.clone(), Ok(()));
        }
        Ok(quota::relay::QuotaDetect {
            steps,
            adapter: found.as_ref().map(|(adapter, _)| *adapter),
            snapshot: found.map(|(_, snapshot)| snapshot),
            key: Some(key),
            error: None,
        })
    }

    /// Krill 只能使用官网 JWT，不复用模型 Key，也不参与站点自动识别。
    pub fn quota_krill_detect(
        &self,
        tool: Tool,
        provider_id: Option<&str>,
        jwt: Option<&str>,
    ) -> ApiResult<quota::relay::QuotaDetect> {
        let stored;
        let jwt = if jwt == Some(redact::PLACEHOLDER) {
            stored = store::load_readonly(&self.env()).map_err(|e| self.err(e))?;
            provider_id
                .and_then(|id| stored.provider(tool, id))
                .and_then(|p| p.meta.extra.get(quota::relay::META_QUOTA))
                .and_then(|q| q["krillJwt"].as_str())
        } else {
            jwt
        }
        .map(str::trim)
        .filter(|jwt| !jwt.is_empty() && *jwt != redact::PLACEHOLDER);
        let Some(jwt) = jwt else {
            return Ok(quota::relay::QuotaDetect {
                steps: Vec::new(),
                adapter: None,
                snapshot: None,
                key: None,
                error: Some("noKrillJwt".into()),
            });
        };
        let jwt = self.krill_credential(jwt)?;
        let client =
            quota::codex_remote::client().map_err(|_| ApiError::message("network".into()))?;
        let result = quota::relay::detect_krill(&client, &jwt, quota::now_ms());
        if let (Some(key), Some(snapshot)) = (&result.key, &result.snapshot) {
            let entry = quota::cache::RelayEntry {
                key: key.clone(),
                snapshot: snapshot.clone(),
                adapter: Some(quota::relay::RelayAdapter::Krill),
            };
            if let Err(err) = quota::cache::save(&self.env(), &[], &[entry], quota::now_ms()) {
                log::warn!("quota: cache not saved: {err}");
            }
            self.quota_outcome(key.clone(), Ok(()));
        }
        Ok(result)
    }

    /// 改一张卡的额度设置（`None` 清除）。只改 `meta.leanflipQuota`，不动客户端配置。
    pub fn quota_configure(
        &self,
        tool: Tool,
        id: &str,
        mut settings: Option<quota::relay::QuotaSettings>,
    ) -> ApiResult<()> {
        if let Some(jwt) = settings.as_mut().and_then(|s| s.krill_jwt.as_mut()) {
            *jwt = self.krill_credential(jwt)?.to_string();
        }
        if let Some(settings) = &settings {
            settings.validate().map_err(ApiError::message)?;
        }
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        store::update(&rt.env, |data| {
            let provider = data
                .app_mut(tool)
                .providers
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| {
                    AppError::InvalidInput(format!("未找到供应商 / Provider not found: {id}"))
                })?;
            match &settings {
                Some(settings) => {
                    let value = serde_json::to_value(settings)
                        .map_err(|source| AppError::JsonSerialize { source })?;
                    let stored = provider.clone();
                    provider
                        .meta
                        .extra
                        .insert(quota::relay::META_QUOTA.into(), value);
                    redact::restore(tool, provider, Some(&stored));
                    if let Some(settings) = quota::relay::QuotaSettings::of(provider) {
                        settings.validate().map_err(AppError::InvalidInput)?;
                        if let Some(address) = quota::relay::account::address(tool, provider) {
                            settings
                                .query_address(&address)
                                .map_err(AppError::InvalidInput)?;
                        }
                    }
                }
                None => {
                    provider.meta.extra.remove(quota::relay::META_QUOTA);
                }
            }
            Ok(())
        })
        .map_err(|e| self.err(e))
    }

    #[cfg(test)]
    pub(crate) fn set_quota_url(&mut self, url: &str) {
        self.quota_url = url.to_string();
    }

    #[cfg(test)]
    pub(crate) fn forget_quota_attempts(&self) {
        self.quota_attempts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    // ---- Codex 托管账号 / 本机登录 ----

    pub fn codex_accounts(&self) -> ApiResult<Vec<AccountInfo>> {
        self.codex_accounts_checked()
    }

    pub fn codex_accounts_checked(&self) -> ApiResult<Vec<AccountInfo>> {
        let rt = self.runtime();
        rt.oauth.try_list_accounts().map_err(|e| self.err(e))
    }

    pub fn codex_refresh_account(&self, id: &str) -> ApiResult<()> {
        let _guard = self.write_guard()?;
        self.runtime()
            .oauth
            .refresh_account(id)
            .map_err(|e| self.err(e))
    }

    pub fn codex_start_login(&self, target: Option<&str>) -> ApiResult<DeviceFlow> {
        let rt = self.runtime();
        rt.oauth.start_device_flow(target).map_err(|e| self.err(e))
    }

    pub fn codex_poll_login(&self, device_code: &str) -> ApiResult<PollResult> {
        let rt = self.runtime();
        match rt
            .oauth
            .poll_device_flow_with_guard(device_code, || rt.env.device_store().lock_app("service"))
        {
            Ok(DeviceFlowPoll::Pending) => Ok(PollResult::Pending),
            Ok(DeviceFlowPoll::Added(account)) => Ok(PollResult::Added { account }),
            Err(e) => Err(self.err(e)),
        }
    }

    pub fn codex_cancel_login(&self, device_code: &str) -> bool {
        let rt = self.runtime();
        rt.oauth.cancel_device_flow(device_code)
    }

    pub fn codex_set_default(&self, id: &str) -> ApiResult<()> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        rt.oauth.set_default_account(id).map_err(|e| self.err(e))
    }

    pub fn codex_remove_account(&self, id: &str) -> ApiResult<()> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        self.settle(&rt, Tool::Codex)?;
        rt.oauth.remove_account(id).map_err(|e| self.err(e))
    }

    /// 本机 `auth.json` / 登录暂存里的 ChatGPT 登录：只有身份信息，没有 token。
    pub fn codex_logins(&self) -> ApiResult<Vec<leanflip_core::codex::login::LoginSummary>> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        self.settle(&rt, Tool::Codex)?;
        switch::codex::available_logins(&rt.env).map_err(|e| self.err(e))
    }

    pub fn codex_set_login_ref(&self, id: &str, identity: Option<&str>) -> ApiResult<OpResult> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        self.settle(&rt, Tool::Codex)?;
        switch::codex::set_login_ref(&rt.env, services(&rt), id, identity)
            .map(|report| self.op_result(report))
            .map_err(|e| self.err(e))
    }

    /// 后台刷新：太久没换过 token 的托管账号各刷新一次。
    pub fn refresh_stale_accounts(&self) -> ApiResult<AccountRefreshResults> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        let hours = self.app_settings().background_refresh_hours;
        if hours == 0 {
            return Ok(Vec::new());
        }
        let max_age_ms = i64::from(hours) * 3_600_000;
        Ok(rt
            .oauth
            .refresh_stale(max_age_ms)
            .map_err(|e| self.err(e))?
            .into_iter()
            .map(|(id, r)| (id, r.map_err(|e| e.to_string())))
            .collect())
    }

    pub fn common_config(
        &self,
        tool: Tool,
    ) -> ApiResult<leanflip_core::common_config::CommonConfig> {
        leanflip_core::common_config::read(&self.runtime().env, tool).map_err(|e| self.err(e))
    }
    pub fn save_common_config(
        &self,
        tool: Tool,
        revision: &str,
        config: serde_json::Value,
    ) -> ApiResult<OpResult> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        self.settle(&rt, tool)?;
        leanflip_core::common_config::save(&rt.env, tool, revision, config)
            .map(|r| self.op_result(r))
            .map_err(|e| self.err(e))
    }

    // ---- 设置 ----

    pub fn settings(&self) -> ApiResult<SettingsView> {
        let rt = self.runtime();
        let data = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        Ok(SettingsView {
            app: self.app_settings(),
            keep_codex_login: data.settings.preserve_codex_official_auth_on_switch,
            data_dir: rt.env.data_dir.clone(),
            home: rt.env.home.clone(),
            live_paths: Tool::ALL
                .iter()
                .map(|&t| (t.as_str().to_string(), rt.env.live_paths(t)))
                .collect(),
            dirs: effective_dirs(&rt.env),
            ccswitch_dir: ccswitch_import::default_dir(&rt.env),
        })
    }

    pub fn save_settings(&self, input: SettingsInput) -> ApiResult<SaveSettingsResult> {
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        let mut app = input.app;
        settings::normalize(&mut app);
        settings::validate(&app).map_err(|e| self.err(e))?;
        // Prepare everything that can fail before changing either persisted setting.
        let previous = store::load_readonly(&rt.env).map_err(|e| self.err(e))?;
        let next_env = self.base.clone().with_overrides(&app.dir_overrides);
        let dirs_changed = next_env != rt.env;
        let oauth = if next_env.codex_dir == rt.env.codex_dir {
            rt.oauth.clone()
        } else {
            Arc::new(
                CodexOAuthManager::new(&next_env, self.transport.clone())
                    .map_err(|e| self.err(e))?,
            )
        };
        let settings_path = settings::path(&rt.env.data_dir);
        let before = leanflip_core::live::engine::read_current(&settings_path)
            .map_err(|e| self.err(e.into()))?;
        settings::save(&rt.env.data_dir, &app).map_err(|e| self.err(e))?;
        if let Err(error) = store::update(&rt.env, |data| {
            if data != &previous {
                return Err(AppError::Config(
                    "Settings changed; retry / 设置已变化，请重试".into(),
                ));
            }
            data.settings.preserve_codex_official_auth_on_switch = input.keep_codex_login;
            Ok(())
        }) {
            let rollback = match before {
                Some(bytes) => leanflip_core::fsutil::atomic_write_private(&settings_path, &bytes),
                None => std::fs::remove_file(&settings_path)
                    .map_err(|e| AppError::io(&settings_path, e)),
            };
            rollback.map_err(|e| {
                ApiError::message(format!("设置回滚失败 / Settings rollback failed: {e}"))
            })?;
            return Err(self.err(error));
        }
        if dirs_changed {
            self.recovery
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(recover(&next_env, self.english()));
            *self.runtime.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(Runtime {
                env: next_env,
                oauth,
            });
        }
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = app;
        Ok(SaveSettingsResult { dirs_changed })
    }

    // ---- 从 cc-switch 导入（只读） ----

    fn ccswitch_dir(&self, dir: Option<PathBuf>) -> ApiResult<PathBuf> {
        let rt = self.runtime();
        match dir {
            Some(dir) if dir.is_absolute() => Ok(dir),
            Some(_) => Err(self.err(AppError::InvalidInput(
                "Choose an absolute CC Switch data directory".into(),
            ))),
            None => ccswitch_import::detected_dir(&rt.env).map_err(|e| self.err(e)),
        }
    }

    pub fn ccswitch_scan(&self, dir: Option<PathBuf>) -> ApiResult<ccswitch_import::Scan> {
        let rt = self.runtime();
        let mut scan =
            ccswitch_import::scan(&rt.env, &self.ccswitch_dir(dir)?).map_err(|e| self.err(e))?;
        for candidate in &mut scan.candidates {
            candidate.provider = redact::redact(candidate.tool, &candidate.provider);
        }
        Ok(scan)
    }

    pub fn ccswitch_apply(
        &self,
        dir: Option<PathBuf>,
        tools: Vec<Tool>,
    ) -> ApiResult<ccswitch_import::Report> {
        self.ccswitch_apply_options(dir, tools, true)
    }

    pub fn ccswitch_apply_options(
        &self,
        dir: Option<PathBuf>,
        tools: Vec<Tool>,
        accounts: bool,
    ) -> ApiResult<ccswitch_import::Report> {
        let directory = self.ccswitch_dir(dir)?;
        let _guard = self.write_guard()?;
        let rt = self.runtime();
        ccswitch_import::apply(
            &rt.env,
            &directory,
            &tools,
            accounts.then_some(rt.oauth.as_ref()),
        )
        .map_err(|e| self.err(e))
    }

    // ---- 导入 cc-switch 的导出文件（只进卡片库） ----

    /// 解析导出文件并暂存，返回预览。文件内容不落盘。
    pub fn export_load(
        &self,
        bytes: &[u8],
        policy: ccswitch_export::DuplicatePolicy,
    ) -> ApiResult<ccswitch_export::Preview> {
        let parsed = ccswitch_export::parse(bytes).map_err(|e| self.err(e))?;
        let preview = ccswitch_export::preview(&self.runtime().env, &parsed, policy)
            .map_err(|e| self.err(e))?;
        *self.staged_export.lock().unwrap_or_else(|e| e.into_inner()) = Some(parsed);
        Ok(preview)
    }

    fn staged(&self) -> ApiResult<ccswitch_export::ParsedExport> {
        self.staged_export
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                self.err(AppError::localized(
                    "ccswitch_export.none",
                    "请先选择导出文件",
                    "Choose an export file first",
                ))
            })
    }

    pub fn export_preview(
        &self,
        policy: ccswitch_export::DuplicatePolicy,
    ) -> ApiResult<ccswitch_export::Preview> {
        ccswitch_export::preview(&self.runtime().env, &self.staged()?, policy)
            .map_err(|e| self.err(e))
    }

    pub fn export_apply(
        &self,
        tools: Vec<Tool>,
        policy: ccswitch_export::DuplicatePolicy,
    ) -> ApiResult<ccswitch_export::Report> {
        let _guard = self.write_guard()?;
        let report = ccswitch_export::apply(&self.runtime().env, &self.staged()?, &tools, policy)
            .map_err(|e| self.err(e))?;
        self.export_discard();
        Ok(report)
    }

    pub fn export_discard(&self) {
        *self.staged_export.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[cfg(test)]
mod tests;
