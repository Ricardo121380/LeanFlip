//! 桌面应用的业务层：不依赖 Tauri，命令层只是把这里的方法放到后台线程里调用。
//! 单元测试用临时 HOME 直接测这一层。
//!
//! 安全约定：从这里出去的数据都经过 [`crate::redact`]，ChatGPT 登录凭据不会到前端。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use ccsl_core::codex::oauth::transport::OAuthTransport;
use ccsl_core::codex::oauth::{AccountInfo, CodexOAuthManager, DeviceFlow, DeviceFlowPoll};
use ccsl_core::operation::{OperationReport, RecoveryOutcome};
use ccsl_core::switch::{self, Services};
use ccsl_core::{
    ccswitch_export, ccswitch_import, coexist, import, store, AppError, Env, Provider, Tool,
};
use serde::{Deserialize, Serialize};

use crate::redact;
use crate::settings::{self, AppSettings};

/// 给前端的错误：一句话（按界面语言选中文 / 英文）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApiError(pub String);

impl ApiError {
    pub fn from_core(error: AppError, english: bool) -> Self {
        match error {
            AppError::Localized { zh, en, .. } => ApiError(if english { en } else { zh }),
            other => ApiError(other.to_string()),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

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
    pub current_name: Option<String>,
    pub in_live: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolProviders {
    pub tool: Tool,
    pub kind: ToolKind,
    pub providers: Vec<Provider>,
    pub current: Option<String>,
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
        let app_settings = settings::load(&base.data_dir).unwrap_or_else(|err| {
            log::warn!("app settings unreadable, using defaults: {err}");
            AppSettings::default()
        });
        let env = base.clone().with_overrides(&app_settings.dir_overrides);
        let recovery = recover(&env, app_settings.language.as_deref() == Some("en"));
        let oauth = Arc::new(CodexOAuthManager::new(&env, transport.clone())?);
        Ok(Self {
            base,
            transport,
            runtime: RwLock::new(Arc::new(Runtime { env, oauth })),
            settings: Mutex::new(app_settings),
            recovery: Mutex::new(recovery),
            staged_export: Mutex::new(None),
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

    /// 套用新的目录覆盖，立即生效。Codex 目录没变时沿用同一个账号管理器（进行中的设备码
    /// 登录不受影响）；变了就新建一个（进行中的登录要重新开始）。返回目录是否有变化。
    fn apply_overrides(&self, overrides: &ccsl_core::env::DirOverrides) -> Result<bool, AppError> {
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
            return Ok(Vec::new());
        }
        Ok(coexist::live_entries(&rt.env, tool)?
            .keys()
            .cloned()
            .collect())
    }

    // ---- 查询 ----

    pub fn overview(&self) -> ApiResult<Vec<ToolSummary>> {
        let rt = self.runtime();
        let data = store::load(&rt.env).map_err(|e| self.err(e))?;
        Ok(Tool::ALL
            .iter()
            .map(|&tool| {
                let paths = rt.env.live_paths(tool);
                let current = data.current(tool);
                ToolSummary {
                    tool,
                    kind: kind(tool),
                    detected: paths.iter().any(|p| p.exists()),
                    paths,
                    card_count: data.providers(tool).len(),
                    current: current.map(|p| p.id.clone()),
                    current_name: current.map(|p| p.name.clone()),
                    in_live: self.live_ids(tool).unwrap_or_default(),
                }
            })
            .collect())
    }

    pub fn list_providers(&self, tool: Tool) -> ApiResult<ToolProviders> {
        let rt = self.runtime();
        let data = store::load(&rt.env).map_err(|e| self.err(e))?;
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
            current: data.current(tool).map(|p| p.id.clone()),
            in_live,
            live_error,
            paths: rt.env.live_paths(tool),
        })
    }

    // ---- 切换 / 卡片 ----

    pub fn switch_provider(&self, tool: Tool, id: &str) -> ApiResult<OpResult> {
        let rt = self.runtime();
        self.settle(&rt, tool)?;
        switch::switch_with(&rt.env, services(&rt), tool, id)
            .map(|report| self.op_result(report))
            .map_err(|e| self.err(e))
    }

    fn check_incoming(&self, provider: &Provider) -> ApiResult<()> {
        if provider.id.trim().is_empty() {
            return Err(ApiError(if self.english() {
                "Provider id must not be empty".into()
            } else {
                "供应商 id 不能为空".into()
            }));
        }
        if redact::contains_placeholder(&provider.settings_config) {
            return Err(ApiError(if self.english() {
                "Redacted login values cannot be saved into another card".into()
            } else {
                "隐藏的登录凭据不能保存到别的卡里".into()
            }));
        }
        Ok(())
    }

    pub fn add_provider(
        &self,
        tool: Tool,
        provider: Provider,
        add_to_live: bool,
    ) -> ApiResult<OpResult> {
        let rt = self.runtime();
        self.check_incoming(&provider)?;
        self.settle(&rt, tool)?;
        if tool.is_coexist() {
            return coexist::add_provider(&rt.env, tool, provider, add_to_live)
                .map(|report| self.op_result(report))
                .map_err(|e| self.err(e));
        }
        let run = || -> Result<OpResult, AppError> {
            let data = store::load(&rt.env)?;
            switch::preflight(&data, tool, &provider)?;
            let id = provider.id.clone();
            store::add_provider(&rt.env, tool, provider)?;
            if add_to_live {
                return switch::switch_with(&rt.env, services(&rt), tool, &id)
                    .map(|report| self.op_result(report));
            }
            Ok(OpResult::default())
        };
        run().map_err(|e| self.err(e))
    }

    pub fn update_provider(&self, tool: Tool, mut provider: Provider) -> ApiResult<OpResult> {
        let rt = self.runtime();
        self.settle(&rt, tool)?;
        let data = store::load(&rt.env).map_err(|e| self.err(e))?;
        let stored = data.provider(tool, &provider.id).cloned();
        redact::restore(tool, &mut provider, stored.as_ref());
        self.check_incoming(&provider)?;
        switch::update_provider_with(&rt.env, services(&rt), tool, provider)
            .map(|report| self.op_result(report))
            .map_err(|e| self.err(e))
    }

    pub fn delete_provider(&self, tool: Tool, id: &str) -> ApiResult<()> {
        let rt = self.runtime();
        let result = if tool.is_coexist() {
            self.settle(&rt, tool)?;
            coexist::delete_provider(&rt.env, tool, id)
        } else {
            store::remove_provider(&rt.env, tool, id)
        };
        result.map(|_| ()).map_err(|e| self.err(e))
    }

    /// 并存式工具：写进 / 移出客户端配置。
    pub fn set_in_live(&self, tool: Tool, id: &str, enabled: bool) -> ApiResult<OpResult> {
        let rt = self.runtime();
        self.settle(&rt, tool)?;
        let result = if enabled {
            coexist::write_to_live(&rt.env, tool, id).map(|report| self.op_result(report))
        } else {
            coexist::remove_from_live(&rt.env, tool, id).map(|report| self.op_result(report))
        };
        result.map_err(|e| self.err(e))
    }

    pub fn import_live(&self, tool: Tool) -> ApiResult<ImportLiveResult> {
        let rt = self.runtime();
        self.settle(&rt, tool)?;
        import::import_live(&rt.env, tool)
            .map(|r| ImportLiveResult { created: r.created })
            .map_err(|e| self.err(e))
    }

    // ---- Codex 托管账号 / 本机登录 ----

    pub fn codex_accounts(&self) -> Vec<AccountInfo> {
        let rt = self.runtime();
        rt.oauth.list_accounts()
    }

    pub fn codex_start_login(&self, target: Option<&str>) -> ApiResult<DeviceFlow> {
        let rt = self.runtime();
        rt.oauth.start_device_flow(target).map_err(|e| self.err(e))
    }

    pub fn codex_poll_login(&self, device_code: &str) -> ApiResult<PollResult> {
        let rt = self.runtime();
        match rt.oauth.poll_device_flow(device_code) {
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
        let rt = self.runtime();
        rt.oauth.set_default_account(id).map_err(|e| self.err(e))
    }

    pub fn codex_remove_account(&self, id: &str) -> ApiResult<()> {
        let rt = self.runtime();
        self.settle(&rt, Tool::Codex)?;
        rt.oauth.remove_account(id).map_err(|e| self.err(e))
    }

    /// 本机 `auth.json` / 登录暂存里的 ChatGPT 登录：只有身份信息，没有 token。
    pub fn codex_logins(&self) -> ApiResult<Vec<ccsl_core::codex::login::LoginSummary>> {
        let rt = self.runtime();
        self.settle(&rt, Tool::Codex)?;
        switch::codex::available_logins(&rt.env).map_err(|e| self.err(e))
    }

    pub fn codex_set_login_ref(&self, id: &str, identity: Option<&str>) -> ApiResult<OpResult> {
        let rt = self.runtime();
        self.settle(&rt, Tool::Codex)?;
        switch::codex::set_login_ref(&rt.env, services(&rt), id, identity)
            .map(|report| self.op_result(report))
            .map_err(|e| self.err(e))
    }

    /// 后台刷新：太久没换过 token 的托管账号各刷新一次。
    pub fn refresh_stale_accounts(&self) -> Vec<(String, Result<bool, String>)> {
        let rt = self.runtime();
        let hours = self.app_settings().background_refresh_hours;
        if hours == 0 {
            return Vec::new();
        }
        let max_age_ms = i64::from(hours) * 3_600_000;
        rt.oauth
            .refresh_stale(max_age_ms)
            .into_iter()
            .map(|(id, r)| (id, r.map_err(|e| e.to_string())))
            .collect()
    }

    // ---- 设置 ----

    pub fn settings(&self) -> ApiResult<SettingsView> {
        let rt = self.runtime();
        let data = store::load(&rt.env).map_err(|e| self.err(e))?;
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
        let rt = self.runtime();
        let mut app = input.app;
        if let Some(lang) = app.language.as_deref() {
            if lang != "zh" && lang != "en" {
                app.language = None;
            }
        }
        settings::save(&rt.env.data_dir, &app).map_err(|e| self.err(e))?;
        store::update(&rt.env, |data| {
            data.settings.preserve_codex_official_auth_on_switch = input.keep_codex_login;
            Ok(())
        })
        .map_err(|e| self.err(e))?;
        let dirs_changed = self
            .apply_overrides(&app.dir_overrides)
            .map_err(|e| self.err(e))?;
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = app;
        Ok(SaveSettingsResult { dirs_changed })
    }

    // ---- 从 cc-switch 导入（只读） ----

    fn ccswitch_dir(&self, dir: Option<PathBuf>) -> PathBuf {
        let rt = self.runtime();
        dir.filter(|d| d.is_absolute())
            .unwrap_or_else(|| ccswitch_import::default_dir(&rt.env))
    }

    pub fn ccswitch_scan(&self, dir: Option<PathBuf>) -> ApiResult<ccswitch_import::Scan> {
        let rt = self.runtime();
        let mut scan =
            ccswitch_import::scan(&rt.env, &self.ccswitch_dir(dir)).map_err(|e| self.err(e))?;
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
        let rt = self.runtime();
        ccswitch_import::apply(&rt.env, &self.ccswitch_dir(dir), &tools, Some(&rt.oauth))
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
