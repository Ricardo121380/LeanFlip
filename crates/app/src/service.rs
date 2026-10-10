//! 桌面应用的业务层：不依赖 Tauri，命令层只是把这里的方法放到后台线程里调用。
//! 单元测试用临时 HOME 直接测这一层。
//!
//! 安全约定：从这里出去的数据都经过 [`crate::redact`]，ChatGPT 登录凭据不会到前端。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ccsl_core::codex::oauth::transport::OAuthTransport;
use ccsl_core::codex::oauth::{AccountInfo, CodexOAuthManager, DeviceFlow, DeviceFlowPoll};
use ccsl_core::operation::OperationReport;
use ccsl_core::switch::{self, Services};
use ccsl_core::{ccswitch_import, coexist, import, store, AppError, Env, Provider, Tool};
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

impl From<OperationReport> for OpResult {
    fn from(report: OperationReport) -> Self {
        Self {
            changed: report.changed,
            warnings: report.warnings,
        }
    }
}

impl From<Option<OperationReport>> for OpResult {
    fn from(report: Option<OperationReport>) -> Self {
        report.map(Into::into).unwrap_or_default()
    }
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
    /// 目录覆盖改了：重启后生效。
    pub restart_required: bool,
}

pub struct Service {
    env: Env,
    oauth: CodexOAuthManager,
    settings: Mutex<AppSettings>,
    /// 启动时目录覆盖的快照（判断要不要重启）。
    startup_overrides: ccsl_core::env::DirOverrides,
    recovery: Vec<String>,
}

impl Service {
    /// `base` 是没套覆盖的环境（`Env::from_system()` 或测试的临时 HOME）。会先补完 /
    /// 放弃上次没做完的写入。
    pub fn start(base: Env, transport: Option<Arc<dyn OAuthTransport>>) -> Result<Self, AppError> {
        let app_settings = settings::load(&base.data_dir).unwrap_or_else(|err| {
            log::warn!("app settings unreadable, using defaults: {err}");
            AppSettings::default()
        });
        let env = base.with_overrides(&app_settings.dir_overrides);
        let recovery = switch::recover_on_startup(&env)
            .into_iter()
            .map(|(app, outcome)| match outcome {
                Ok(outcome) => format!("{app}: {outcome:?}"),
                Err(err) => format!("{app}: {err}"),
            })
            .collect::<Vec<_>>();
        for line in &recovery {
            log::info!("startup recovery {line}");
        }
        let oauth = CodexOAuthManager::new(&env, transport)?;
        Ok(Self {
            env,
            oauth,
            startup_overrides: app_settings.dir_overrides.clone(),
            settings: Mutex::new(app_settings),
            recovery,
        })
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    pub fn oauth(&self) -> &CodexOAuthManager {
        &self.oauth
    }

    pub fn recovery(&self) -> &[String] {
        &self.recovery
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

    fn services(&self) -> Services<'_> {
        Services {
            codex_oauth: Some(&self.oauth),
        }
    }

    fn live_ids(&self, tool: Tool) -> Result<Vec<String>, AppError> {
        if !tool.is_coexist() {
            return Ok(Vec::new());
        }
        Ok(coexist::live_entries(&self.env, tool)?
            .keys()
            .cloned()
            .collect())
    }

    // ---- 查询 ----

    pub fn overview(&self) -> ApiResult<Vec<ToolSummary>> {
        let data = store::load(&self.env).map_err(|e| self.err(e))?;
        Ok(Tool::ALL
            .iter()
            .map(|&tool| {
                let paths = self.env.live_paths(tool);
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
        let data = store::load(&self.env).map_err(|e| self.err(e))?;
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
            paths: self.env.live_paths(tool),
        })
    }

    // ---- 切换 / 卡片 ----

    pub fn switch_provider(&self, tool: Tool, id: &str) -> ApiResult<OpResult> {
        switch::switch_with(&self.env, self.services(), tool, id)
            .map(Into::into)
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
        self.check_incoming(&provider)?;
        if tool.is_coexist() {
            return coexist::add_provider(&self.env, tool, provider, add_to_live)
                .map(Into::into)
                .map_err(|e| self.err(e));
        }
        let run = || -> Result<OpResult, AppError> {
            switch::settle(&self.env, tool)?;
            let data = store::load(&self.env)?;
            switch::preflight(&data, tool, &provider)?;
            let id = provider.id.clone();
            store::add_provider(&self.env, tool, provider)?;
            if add_to_live {
                return switch::switch_with(&self.env, self.services(), tool, &id).map(Into::into);
            }
            Ok(OpResult::default())
        };
        run().map_err(|e| self.err(e))
    }

    pub fn update_provider(&self, tool: Tool, mut provider: Provider) -> ApiResult<OpResult> {
        let data = store::load(&self.env).map_err(|e| self.err(e))?;
        let stored = data.provider(tool, &provider.id).cloned();
        redact::restore(tool, &mut provider, stored.as_ref());
        self.check_incoming(&provider)?;
        switch::update_provider_with(&self.env, self.services(), tool, provider)
            .map(Into::into)
            .map_err(|e| self.err(e))
    }

    pub fn delete_provider(&self, tool: Tool, id: &str) -> ApiResult<()> {
        let result = if tool.is_coexist() {
            coexist::delete_provider(&self.env, tool, id)
        } else {
            store::remove_provider(&self.env, tool, id)
        };
        result.map(|_| ()).map_err(|e| self.err(e))
    }

    /// 并存式工具：写进 / 移出客户端配置。
    pub fn set_in_live(&self, tool: Tool, id: &str, enabled: bool) -> ApiResult<OpResult> {
        let result = if enabled {
            coexist::write_to_live(&self.env, tool, id).map(Into::into)
        } else {
            coexist::remove_from_live(&self.env, tool, id).map(Into::into)
        };
        result.map_err(|e| self.err(e))
    }

    pub fn import_live(&self, tool: Tool) -> ApiResult<ImportLiveResult> {
        import::import_live(&self.env, tool)
            .map(|r| ImportLiveResult { created: r.created })
            .map_err(|e| self.err(e))
    }

    // ---- Codex 托管账号 / 本机登录 ----

    pub fn codex_accounts(&self) -> Vec<AccountInfo> {
        self.oauth.list_accounts()
    }

    pub fn codex_start_login(&self, target: Option<&str>) -> ApiResult<DeviceFlow> {
        self.oauth
            .start_device_flow(target)
            .map_err(|e| self.err(e))
    }

    pub fn codex_poll_login(&self, device_code: &str) -> ApiResult<PollResult> {
        match self.oauth.poll_device_flow(device_code) {
            Ok(DeviceFlowPoll::Pending) => Ok(PollResult::Pending),
            Ok(DeviceFlowPoll::Added(account)) => Ok(PollResult::Added { account }),
            Err(e) => Err(self.err(e)),
        }
    }

    pub fn codex_cancel_login(&self, device_code: &str) -> bool {
        self.oauth.cancel_device_flow(device_code)
    }

    pub fn codex_set_default(&self, id: &str) -> ApiResult<()> {
        self.oauth.set_default_account(id).map_err(|e| self.err(e))
    }

    pub fn codex_remove_account(&self, id: &str) -> ApiResult<()> {
        self.oauth.remove_account(id).map_err(|e| self.err(e))
    }

    /// 本机 `auth.json` / 登录暂存里的 ChatGPT 登录：只有身份信息，没有 token。
    pub fn codex_logins(&self) -> ApiResult<Vec<ccsl_core::codex::login::LoginSummary>> {
        switch::codex::available_logins(&self.env).map_err(|e| self.err(e))
    }

    pub fn codex_set_login_ref(&self, id: &str, identity: Option<&str>) -> ApiResult<OpResult> {
        switch::codex::set_login_ref(&self.env, self.services(), id, identity)
            .map(Into::into)
            .map_err(|e| self.err(e))
    }

    /// 后台刷新：太久没换过 token 的托管账号各刷新一次。
    pub fn refresh_stale_accounts(&self) -> Vec<(String, Result<bool, String>)> {
        let hours = self.app_settings().background_refresh_hours;
        if hours == 0 {
            return Vec::new();
        }
        let max_age_ms = i64::from(hours) * 3_600_000;
        self.oauth
            .refresh_stale(max_age_ms)
            .into_iter()
            .map(|(id, r)| (id, r.map_err(|e| e.to_string())))
            .collect()
    }

    // ---- 设置 ----

    pub fn settings(&self) -> ApiResult<SettingsView> {
        let data = store::load(&self.env).map_err(|e| self.err(e))?;
        Ok(SettingsView {
            app: self.app_settings(),
            keep_codex_login: data.settings.preserve_codex_official_auth_on_switch,
            data_dir: self.env.data_dir.clone(),
            home: self.env.home.clone(),
            live_paths: Tool::ALL
                .iter()
                .map(|&t| (t.as_str().to_string(), self.env.live_paths(t)))
                .collect(),
            ccswitch_dir: ccswitch_import::default_dir(&self.env),
        })
    }

    pub fn save_settings(&self, input: SettingsInput) -> ApiResult<SaveSettingsResult> {
        let mut app = input.app;
        if let Some(lang) = app.language.as_deref() {
            if lang != "zh" && lang != "en" {
                app.language = None;
            }
        }
        settings::save(&self.env.data_dir, &app).map_err(|e| self.err(e))?;
        store::update(&self.env, |data| {
            data.settings.preserve_codex_official_auth_on_switch = input.keep_codex_login;
            Ok(())
        })
        .map_err(|e| self.err(e))?;
        let restart_required = app.dir_overrides != self.startup_overrides;
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = app;
        Ok(SaveSettingsResult { restart_required })
    }

    // ---- 从 cc-switch 导入（只读） ----

    fn ccswitch_dir(&self, dir: Option<PathBuf>) -> PathBuf {
        dir.filter(|d| d.is_absolute())
            .unwrap_or_else(|| ccswitch_import::default_dir(&self.env))
    }

    pub fn ccswitch_scan(&self, dir: Option<PathBuf>) -> ApiResult<ccswitch_import::Scan> {
        let mut scan =
            ccswitch_import::scan(&self.env, &self.ccswitch_dir(dir)).map_err(|e| self.err(e))?;
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
        ccswitch_import::apply(
            &self.env,
            &self.ccswitch_dir(dir),
            &tools,
            Some(&self.oauth),
        )
        .map_err(|e| self.err(e))
    }
}

#[cfg(test)]
mod tests;
