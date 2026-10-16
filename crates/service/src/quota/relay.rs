//! 中转站的额度：用卡片里已有的 Key，只请求这张卡自己的地址（origin），不跟随重定向。
//!
//! - sub2api：`GET {origin}/v1/usage`（源码 `backend/internal/handler/gateway_handler.go`
//!   的 `Usage`）。三种返回：按 Key 限额（`quota`、5h/1d/7d 的 `rate_limits`）、订阅
//!   （`subscription` 的日/周/月用量和上限）、钱包（`balance`）。金额单位都是 USD。
//! - NewAPI：`GET {origin}/api/usage/token/`（`controller/token.go` 的 `GetTokenUsage`）。
//!   数值是站点的内部额度，默认 500000 = $1，站点可以改，所以换算比例可以在卡片上设置。
//!   令牌允许时还带 `user_quota`，就是账户余额。
//!
//! 很多站点对不认识的路径也回 200 网页，所以按 JSON 的内容判断是哪个平台。
//! 卡片上的设置存在 `meta.leanflipQuota`，和 CC Switch 的 `usage_script` 分开。

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelayAdapter {
    Sub2api,
    Newapi,
}

impl RelayAdapter {
    /// 识别时按这个顺序试。
    const ALL: [RelayAdapter; 2] = [RelayAdapter::Sub2api, RelayAdapter::Newapi];

    fn path(self) -> &'static str {
        match self {
            RelayAdapter::Sub2api => "/v1/usage",
            RelayAdapter::Newapi => "/api/usage/token/",
        }
    }
}

/// 卡片上的额度设置。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSettings {
    /// `sub2api`、`newapi`，或 `off`（不查）。没有时表示还没识别过。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    /// 自动刷新间隔（分钟），0 表示只在点刷新时查。
    #[serde(default)]
    pub refresh_minutes: u32,
    /// NewAPI 多少额度算 $1。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_per_usd: Option<f64>,
}

impl QuotaSettings {
    pub fn of(provider: &Provider) -> Option<Self> {
        serde_json::from_value(provider.meta.extra.get(META_QUOTA)?.clone()).ok()
    }

    pub(crate) fn adapter(&self) -> Option<RelayAdapter> {
        match self.adapter.as_deref()? {
            "sub2api" => Some(RelayAdapter::Sub2api),
            "newapi" => Some(RelayAdapter::Newapi),
            _ => None,
        }
    }

    pub(crate) fn per_usd(&self) -> f64 {
        self.quota_per_usd
            .filter(|n| n.is_finite() && *n > 0.0)
            .unwrap_or(DEFAULT_QUOTA_PER_USD)
    }

    /// 保存前检查：平台只认三个值，间隔 0–1440 分钟，换算比例要是正数。
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(
            self.adapter.as_deref(),
            None | Some("sub2api" | "newapi" | "off")
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
    let root = origin(address)
        .join(adapter.path())
        .expect("static quota path");
    let prefixed = query_base(address)
        .join(adapter.path().trim_start_matches('/'))
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
    let response = client
        .get(url)
        .bearer_auth(key)
        .header(ACCEPT, "application/json")
        .send()
        .map_err(|e| Failure {
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
    let (status, value) = get(client, url, key)?;
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
    /// 没法识别的原因：`noEndpoint`（卡片里没有地址或 Key）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
        remaining: usd(&data["total_available"]),
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

/// 一张设置了平台类型的中转卡。
#[derive(Debug, Clone)]
pub(crate) struct RelayCard {
    pub user: QuotaUser,
    pub key: String,
    pub address: Url,
    pub secret: String,
    pub adapter: RelayAdapter,
    pub per_usd: f64,
    pub refresh_minutes: u32,
}

/// 各工具里设置了平台类型（sub2api / NewAPI）的卡。`active` 判断这张卡是否正在用。
pub(crate) fn relay_cards(
    providers: &[(Tool, &Provider)],
    active: &dyn Fn(Tool, &str) -> bool,
) -> Vec<RelayCard> {
    providers
        .iter()
        .filter(|(_, p)| !p.is_official_category())
        .filter_map(|(tool, p)| {
            let settings = QuotaSettings::of(p)?;
            let adapter = settings.adapter()?;
            let (address, secret) = crate::probe::address_and_key(*tool, &p.settings_config)?;
            Some(RelayCard {
                user: QuotaUser {
                    tool: *tool,
                    provider_id: p.id.clone(),
                    provider_name: p.name.clone(),
                    active: active(*tool, &p.id),
                },
                key: source_key(&query_base(&address), &secret),
                address,
                secret,
                adapter,
                per_usd: settings.per_usd(),
                refresh_minutes: settings.refresh_minutes,
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
            adapter: Some(card.adapter),
            refresh_minutes: interval,
            users: vec![card.user.clone()],
            snapshot,
            error: errors.get(&card.key).filter(|e| e.at > newest).cloned(),
        });
    }
    sources
}
