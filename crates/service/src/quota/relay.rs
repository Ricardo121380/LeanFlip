//! 中转站额度：sub2api / NewAPI 使用模型 Key 或独立用户令牌和同一主机；Krill 使用官网登录。
//! 所有请求均不跟随重定向。
//!
//! - sub2api：`GET {origin}/v1/usage`（源码 `backend/internal/handler/gateway_handler.go`
//!   的 `Usage`）。三种返回：按 Key 限额（`quota`、5h/1d/7d 的 `rate_limits`）、订阅
//!   （`subscription` 的日/周/月用量和上限）、钱包（`balance`）。金额单位都是 USD。
//! - NewAPI：`GET {origin}/api/usage/token/`（`controller/token.go` 的 `GetTokenUsage`）。
//!   数值是站点的内部额度，默认 500000 = $1，站点可以改，所以换算比例可以在卡片上设置。
//!   令牌允许时还带 `user_quota`，就是账户余额。
//! - Krill：`GET https://www.krill-code.com/api/subscription`，网站登录 JWT，见 [`krill`]。
//!
//! 很多站点对不认识的路径也回 200 网页，所以按 JSON 的内容判断是哪个平台。
//! 卡片上的设置存在 `meta.leanflipQuota`，和 CC Switch 的 `usage_script` 分开。

pub(crate) mod account;
mod krill;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::Read;

use leanflip_core::live::engine::digest;
use leanflip_core::{Provider, Tool};
use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::codex_remote::Failure;
use super::{
    QuotaBalance, QuotaError, QuotaKind, QuotaOrigin, QuotaSnapshot, QuotaSource, QuotaUser,
    QuotaWindow,
};

pub const META_QUOTA: &str = "leanflipQuota";
pub const DEFAULT_QUOTA_PER_USD: f64 = 500_000.0;
const MAX_BODY: u64 = 4 * 1024 * 1024;
pub(crate) const KRILL_URL: &str = "https://www.krill-code.com/api/subscription";

/// 只识别已知官方模型入口，不用卡片名称、任意子域或 URL 中的字符串匹配。
pub(crate) fn is_krill_address(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "api.krill-code.net"
                    | "api-slb.krill-code.net"
                    | "api-slb.krill-ai.net"
                    | "api.cdn-krill-ai.com"
                    | "api.krill-ai.com"
            )
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelayAdapter {
    Sub2api,
    Newapi,
    Krill,
}

impl RelayAdapter {
    /// 使用模型 Key 识别时按这个顺序试。Krill 使用独立的官网登录凭据。
    const ALL: [RelayAdapter; 2] = [RelayAdapter::Sub2api, RelayAdapter::Newapi];

    fn path(self) -> &'static str {
        match self {
            RelayAdapter::Sub2api => "/v1/usage",
            RelayAdapter::Newapi => "/api/usage/token/",
            RelayAdapter::Krill => "/api/subscription",
        }
    }
}

/// 卡片上的额度设置。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSettings {
    /// `sub2api`、`newapi`、`krill`，或 `off`（不查）。没有时自动识别。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    /// 自动刷新间隔（分钟），0 表示只在点刷新时查。
    #[serde(default)]
    pub refresh_minutes: u32,
    /// NewAPI 多少额度算 $1。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_per_usd: Option<f64>,
    /// Krill 网站登录 JWT；仅用于固定官网的额度查询，发给前端时脱敏。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub krill_jwt: Option<String>,
    /// 网站用户访问令牌，与模型调用 Key 分开；返回界面时脱敏。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_token: Option<String>,
    /// 兼容要求 New-Api-User 请求头的旧版 NewAPI。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_user_id: Option<u64>,
}

impl QuotaSettings {
    pub fn of(provider: &Provider) -> Option<Self> {
        serde_json::from_value(provider.meta.extra.get(META_QUOTA)?.clone()).ok()
    }

    pub(crate) fn adapter(&self) -> Option<RelayAdapter> {
        match self.adapter.as_deref()? {
            "sub2api" => Some(RelayAdapter::Sub2api),
            "newapi" => Some(RelayAdapter::Newapi),
            "krill" => Some(RelayAdapter::Krill),
            _ => None,
        }
    }

    pub(crate) fn per_usd(&self) -> f64 {
        self.quota_per_usd
            .filter(|n| n.is_finite() && *n > 0.0)
            .unwrap_or(DEFAULT_QUOTA_PER_USD)
    }

    /// 保存前检查：已识别平台、刷新间隔、换算比例和用户 ID。
    pub fn validate(&self) -> Result<(), String> {
        if self.account_user_id == Some(0) {
            return Err("用户 ID 必须大于 0 / User ID must be positive".into());
        }
        if !matches!(
            self.adapter.as_deref(),
            None | Some("sub2api" | "newapi" | "krill" | "off")
        ) {
            return Err("unknown quota adapter".into());
        }
        if self.refresh_minutes > 1440 {
            return Err("refresh interval is too long".into());
        }
        if self
            .quota_per_usd
            .is_some_and(|n| !n.is_finite() || n <= 0.0)
        {
            return Err("quota per USD must be positive".into());
        }
        // 平台由地址自动识别；尚未登录也允许保存卡片，查询时再引导连接。
        Ok(())
    }
}

/// 只保留 scheme、主机和端口，作为默认查询位置。
pub(crate) fn origin(url: &Url) -> Url {
    let mut origin = url.clone();
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);
    let _ = origin.set_username("");
    let _ = origin.set_password(None);
    origin
}

/// 保留子路径部署的前缀，但去掉客户端 API 版本、查询参数和 URL 中的凭据。
pub(crate) fn query_base(address: &Url) -> Url {
    let path = address.path().trim_end_matches('/');
    let prefix = path
        .strip_suffix("/v1")
        .or_else(|| path.strip_suffix("/v1beta"))
        .unwrap_or(path);
    let mut base = origin(address);
    base.set_path(&format!("{prefix}/"));
    base
}

fn endpoints(adapter: RelayAdapter, address: &Url) -> Vec<Url> {
    if adapter == RelayAdapter::Krill {
        return vec![Url::parse(KRILL_URL).expect("static Krill URL")];
    }
    path_endpoints(address, adapter.path())
}

fn path_endpoints(address: &Url, path: &str) -> Vec<Url> {
    let root = origin(address).join(path).expect("static quota path");
    let prefixed = query_base(address)
        .join(path.trim_start_matches('/'))
        .expect("static quota path");
    if prefixed == root {
        vec![root]
    } else {
        vec![root, prefixed]
    }
}

/// 只在路径不匹配时试同一主机的子路径；限流、网络错误和重定向不重试。
fn try_next_endpoint(failure: &Failure) -> bool {
    failure.code == "format" || matches!(failure.status, Some(401 | 403 | 404 | 405))
}

/// 来源标识只存 Key 的哈希前 16 位，不存 Key。
pub(crate) fn source_key(origin: &Url, key: &str) -> String {
    let hash = digest(Some(key.as_bytes())).unwrap_or_default();
    format!(
        "relay:{}:{}",
        origin.as_str().trim_end_matches('/'),
        &hash[..16.min(hash.len())]
    )
}

fn get(client: &Client, url: Url, key: &str) -> Result<(u16, Value), Failure> {
    get_with_user(client, url, key, None)
}

fn get_with_user(
    client: &Client,
    url: Url,
    key: &str,
    user_id: Option<u64>,
) -> Result<(u16, Value), Failure> {
    let mut request = client
        .get(url)
        .bearer_auth(key)
        .header(ACCEPT, "application/json");
    if let Some(id) = user_id {
        request = request.header("New-Api-User", id.to_string());
    }
    let response = request.send().map_err(|e| Failure {
        code: if e.is_timeout() { "timeout" } else { "network" },
        status: None,
    })?;
    let status = response.status().as_u16();
    let failure = |code| Failure {
        code,
        status: Some(status),
    };
    match status {
        200..=299 => {}
        401 | 403 => return Err(failure("unauthorized")),
        429 => return Err(failure("rateLimited")),
        _ => return Err(failure("http")),
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_BODY + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure {
            code: "network",
            status: None,
        })?;
    if bytes.len() as u64 > MAX_BODY {
        return Err(failure("format"));
    }
    serde_json::from_slice(&bytes)
        .map(|value| (status, value))
        .map_err(|_| failure("format"))
}

pub(crate) fn fetch(
    client: &Client,
    adapter: RelayAdapter,
    address: &Url,
    key: &str,
    per_usd: f64,
    now_ms: i64,
) -> Result<QuotaSnapshot, Failure> {
    let urls = endpoints(adapter, address);
    let mut result = fetch_at(client, adapter, urls[0].clone(), key, per_usd, now_ms);
    if result.as_ref().is_err_and(try_next_endpoint) && urls.len() > 1 {
        result = fetch_at(client, adapter, urls[1].clone(), key, per_usd, now_ms);
    }
    result
}

fn fetch_at(
    client: &Client,
    adapter: RelayAdapter,
    url: Url,
    key: &str,
    per_usd: f64,
    now_ms: i64,
) -> Result<QuotaSnapshot, Failure> {
    let (status, value) = get(client, url, key).map_err(|mut failure| {
        if adapter == RelayAdapter::Krill && failure.code == "unauthorized" {
            failure.code = "krillAuth";
        }
        failure
    })?;
    parse(adapter, &value, per_usd, now_ms).ok_or(Failure {
        code: "format",
        status: Some(status),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StepOutcome {
    /// 返回内容认得出：就是这个平台。
    Hit,
    /// 有回应但不是这个平台（或这个 Key 在这里被拒绝）。
    Miss,
    /// 网络不通或超时，没法判断。
    Error,
    /// 前面已经认出来了，没再试。
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectStep {
    pub adapter: RelayAdapter,
    /// 不含 Key。
    pub url: String,
    pub outcome: StepOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaDetect {
    pub steps: Vec<DetectStep>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<RelayAdapter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<QuotaSnapshot>,
    /// 来源标识（同一地址加同一个 Key 只算一个来源）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// 没法识别的原因：`noEndpoint`（无地址或 Key）或 `noKrillJwt`（无网站凭据）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(crate) fn detect_krill(client: &Client, jwt: &str, now_ms: i64) -> QuotaDetect {
    let url = Url::parse(KRILL_URL).expect("static Krill URL");
    let result = fetch(
        client,
        RelayAdapter::Krill,
        &url,
        jwt,
        DEFAULT_QUOTA_PER_USD,
        now_ms,
    );
    let (outcome, status, code) = match &result {
        Ok(_) => (StepOutcome::Hit, Some(200), None),
        Err(f) => (
            if matches!(f.code, "network" | "timeout") {
                StepOutcome::Error
            } else {
                StepOutcome::Miss
            },
            f.status,
            Some(f.code.to_string()),
        ),
    };
    QuotaDetect {
        steps: vec![DetectStep {
            adapter: RelayAdapter::Krill,
            url: KRILL_URL.into(),
            outcome,
            status,
            code,
        }],
        adapter: result.is_ok().then_some(RelayAdapter::Krill),
        snapshot: result.ok(),
        key: Some(source_key(&origin(&url), jwt)),
        error: None,
    }
}

/// 依次试各平台的额度接口，认出一个就停。
pub(crate) fn detect(
    client: &Client,
    address: &Url,
    key: &str,
    per_usd: f64,
    now_ms: i64,
) -> (Vec<DetectStep>, Option<(RelayAdapter, QuotaSnapshot)>) {
    let mut steps = Vec::new();
    let mut found = None;
    for adapter in RelayAdapter::ALL {
        let urls = endpoints(adapter, address);
        if found.is_some() {
            steps.push(DetectStep {
                adapter,
                url: urls[0].to_string(),
                outcome: StepOutcome::Skipped,
                status: None,
                code: None,
            });
            continue;
        }
        for url in urls {
            let step = |outcome, status, code: Option<&str>| DetectStep {
                adapter,
                url: url.to_string(),
                outcome,
                status,
                code: code.map(str::to_string),
            };
            match fetch_at(client, adapter, url.clone(), key, per_usd, now_ms) {
                Ok(snapshot) => {
                    steps.push(step(StepOutcome::Hit, Some(200), None));
                    found = Some((adapter, snapshot));
                    break;
                }
                Err(f) => {
                    let outcome = if matches!(f.code, "network" | "timeout") {
                        StepOutcome::Error
                    } else {
                        StepOutcome::Miss
                    };
                    steps.push(step(outcome, f.status, Some(f.code)));
                    if !try_next_endpoint(&f) {
                        break;
                    }
                }
            }
        }
    }
    (steps, found)
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n: &f64| n.is_finite())
}

fn time(value: &Value) -> Option<i64> {
    match value {
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|t| t.timestamp()),
        Value::Number(n) => n.as_i64().filter(|t| *t > 0),
        _ => None,
    }
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn parse(
    adapter: RelayAdapter,
    value: &Value,
    per_usd: f64,
    now_ms: i64,
) -> Option<QuotaSnapshot> {
    match adapter {
        RelayAdapter::Sub2api => parse_sub2api(value, now_ms),
        RelayAdapter::Newapi => parse_newapi(value, per_usd, now_ms),
        RelayAdapter::Krill => krill::parse(value, now_ms),
    }
}

fn amount_window(
    minutes: i64,
    used: f64,
    limit: f64,
    resets_at: Option<i64>,
    now_ms: i64,
) -> QuotaWindow {
    QuotaWindow {
        minutes,
        label: None,
        unit: None,
        used_percent: used / limit * 100.0,
        resets_at,
        reset: resets_at.is_some_and(|at| at.saturating_mul(1000) <= now_ms),
        used_amount: Some(used),
        limit_amount: Some(limit),
    }
}

fn parse_sub2api(v: &Value, now_ms: i64) -> Option<QuotaSnapshot> {
    let recognized = matches!(v["mode"].as_str(), Some("quota_limited" | "unrestricted"))
        || v["isValid"].is_boolean();
    let has_data = [
        "remaining",
        "quota",
        "rate_limits",
        "subscription",
        "balance",
    ]
    .iter()
    .any(|k| !v[*k].is_null());
    if !recognized || !has_data {
        return None;
    }
    let unit = text(&v["unit"])
        .or_else(|| text(&v["quota"]["unit"]))
        .unwrap_or_else(|| "USD".into());
    let mut windows = Vec::new();
    for entry in v["rate_limits"].as_array().into_iter().flatten() {
        let minutes = match entry["window"].as_str() {
            Some("5h") => 300,
            Some("1d") => 1440,
            Some("7d") => 10080,
            _ => continue,
        };
        let (Some(used), Some(limit)) = (number(&entry["used"]), number(&entry["limit"])) else {
            continue;
        };
        if limit > 0.0 {
            windows.push(amount_window(
                minutes,
                used,
                limit,
                time(&entry["reset_at"]),
                now_ms,
            ));
        }
    }
    let sub = &v["subscription"];
    let weekly_reset = time(&sub["weekly_window_start"]).map(|start| start + 7 * 86_400);
    for (period, minutes, resets_at) in [
        ("daily", 1440, None),
        ("weekly", 10080, weekly_reset),
        ("monthly", 30 * 1440, None),
    ] {
        let used = number(&sub[format!("{period}_usage_usd")]);
        let limit = number(&sub[format!("{period}_limit_usd")]).filter(|l| *l > 0.0);
        if let (Some(used), Some(limit)) = (used, limit) {
            windows.push(amount_window(minutes, used, limit, resets_at, now_ms));
        }
    }
    let quota = &v["quota"];
    let balance = if quota.is_object() {
        Some(QuotaBalance {
            unit,
            remaining: number(&quota["remaining"]),
            limit: number(&quota["limit"]),
            used: number(&quota["used"]),
            unlimited: false,
            expires_at: time(&v["expires_at"]),
            account_remaining: None,
        })
    } else {
        number(&v["remaining"])
            .or_else(|| number(&v["balance"]))
            .map(|remaining| QuotaBalance {
                unit,
                remaining: Some(remaining),
                limit: None,
                used: None,
                unlimited: false,
                expires_at: time(&v["expires_at"]).or_else(|| time(&sub["expires_at"])),
                account_remaining: None,
            })
    };
    let reached = v["isValid"].as_bool() == Some(false)
        || matches!(v["status"].as_str(), Some("quota_exhausted" | "expired"));
    Some(
        QuotaSnapshot::new(
            windows,
            None,
            text(&v["planName"]),
            now_ms,
            QuotaOrigin::Network,
            reached,
        )
        .with_balance(balance),
    )
}

fn parse_newapi(v: &Value, per_usd: f64, now_ms: i64) -> Option<QuotaSnapshot> {
    let data = &v["data"];
    if v["code"].as_bool() != Some(true)
        || !(data["object"] == "token_usage" || number(&data["total_available"]).is_some())
    {
        return None;
    }
    let usd = |value: &Value| number(value).map(|n| n / per_usd);
    let unlimited = data["unlimited_quota"].as_bool().unwrap_or(false);
    let expires_at = number(&data["expires_at"])
        .map(|t| t as i64)
        .filter(|t| *t > 0);
    let balance = QuotaBalance {
        unit: "USD".into(),
        // 不限额 Key 的 RemainQuota 可能为负数，不能当成可用账户余额。
        remaining: usd(&data["total_available"]).filter(|_| !unlimited),
        limit: usd(&data["total_granted"]).filter(|_| !unlimited),
        used: usd(&data["total_used"]),
        unlimited,
        expires_at,
        account_remaining: usd(&data["user_quota"]["total_available"]),
    };
    let expired = expires_at.is_some_and(|t| t.saturating_mul(1000) <= now_ms);
    Some(
        QuotaSnapshot::new(
            Vec::new(),
            None,
            text(&data["name"]),
            now_ms,
            QuotaOrigin::Network,
            expired,
        )
        .with_balance(Some(balance)),
    )
}

/// 一张可查询的中转卡；平台为空时自动识别。
#[derive(Debug, Clone)]
pub(crate) struct RelayCard {
    pub user: QuotaUser,
    pub key: String,
    pub address: Url,
    pub secret: String,
    pub adapter: Option<RelayAdapter>,
    pub per_usd: f64,
    pub refresh_minutes: u32,
    pub account_query: bool,
    pub account_user_id: Option<u64>,
}

/// 各工具里未关闭额度查询、且有地址和 Key 的中转卡。`active` 判断这张卡是否正在用。
pub(crate) fn relay_cards(
    providers: &[(Tool, &Provider)],
    active: &dyn Fn(Tool, &str) -> bool,
) -> Vec<RelayCard> {
    providers
        .iter()
        .filter(|(_, p)| !p.is_official_category())
        .filter_map(|(tool, p)| {
            let settings = QuotaSettings::of(p).unwrap_or_default();
            if settings.adapter.as_deref() == Some("off") {
                return None;
            }
            let model_address = account::address(*tool, p)?;
            let adapter = if is_krill_address(&model_address) {
                Some(RelayAdapter::Krill)
            } else {
                settings.adapter().filter(|a| *a != RelayAdapter::Krill)
            };
            let account_token = settings
                .account_token
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != crate::redact::PLACEHOLDER);
            let account_query = adapter != Some(RelayAdapter::Krill) && account_token.is_some();
            let (address, secret) = if adapter == Some(RelayAdapter::Krill) {
                let jwt = settings.krill_jwt.as_deref()?.trim();
                if jwt.is_empty() || jwt == crate::redact::PLACEHOLDER {
                    return None;
                }
                (Url::parse(KRILL_URL).ok()?, jwt.to_string())
            } else if account_query {
                (account::address(*tool, p)?, account_token?.to_string())
            } else {
                let (address, model_key) =
                    crate::probe::address_and_key(*tool, &p.settings_config)?;
                (address, model_key)
            };
            Some(RelayCard {
                user: QuotaUser {
                    tool: *tool,
                    provider_id: p.id.clone(),
                    provider_name: p.name.clone(),
                    active: active(*tool, &p.id),
                },
                key: if account_query {
                    account::key(&address, &secret, settings.account_user_id)
                } else if adapter == Some(RelayAdapter::Krill) {
                    source_key(&origin(&address), &secret)
                } else {
                    source_key(&query_base(&address), &secret)
                },
                address,
                secret,
                adapter,
                per_usd: settings.per_usd(),
                refresh_minutes: settings.refresh_minutes,
                account_query,
                account_user_id: settings.account_user_id,
            })
        })
        .collect()
}

/// 按来源合并中转卡，配上缓存的数据和比它更新的错误。
pub(crate) fn relay_sources(
    cards: &[RelayCard],
    cached: &HashMap<String, QuotaSnapshot>,
    errors: &HashMap<String, QuotaError>,
) -> Vec<QuotaSource> {
    let mut sources: Vec<QuotaSource> = Vec::new();
    for card in cards {
        let interval = (card.refresh_minutes > 0).then_some(card.refresh_minutes);
        if let Some(source) = sources.iter_mut().find(|s| s.key == card.key) {
            source.users.push(card.user.clone());
            source.refresh_minutes = match (source.refresh_minutes, interval) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            continue;
        }
        let snapshot = cached.get(&card.key).cloned();
        let newest = snapshot.as_ref().map_or(i64::MIN, |s| s.fetched_at);
        sources.push(QuotaSource {
            key: card.key.clone(),
            kind: QuotaKind::Relay,
            label: card.address.host_str().map(str::to_string),
            account_id: None,
            login: None,
            adapter: card.adapter,
            refresh_minutes: interval,
            users: vec![card.user.clone()],
            snapshot,
            error: errors.get(&card.key).filter(|e| e.at > newest).cloned(),
        });
    }
    sources
}
