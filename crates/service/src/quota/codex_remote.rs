//! 托管 ChatGPT 账号的额度查询：`GET https://chatgpt.com/backend-api/wham/usage`。
//!
//! 这不是公开接口，Codex 客户端自己用它显示限额。必须带 `ChatGPT-Account-Id`，否则返回的是
//! 不分工作区的汇总数字。只在用户点刷新时调用；不跟随重定向；错误里只有状态码，不含响应
//! 内容和凭据。字段是 snake_case，也认 camelCase，免得服务端改写法时整个失效。

use std::io::Read;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use serde_json::Value;

use super::{parse_credits, QuotaOrigin, QuotaSnapshot, QuotaWindow};
use crate::oauth_http::USER_AGENT;

pub(crate) const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub code: &'static str,
    pub status: Option<u16>,
}

impl Failure {
    fn new(code: &'static str) -> Self {
        Self { code, status: None }
    }
}

pub(crate) fn client() -> Result<Client, Failure> {
    Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(8))
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| Failure::new("network"))
}

pub(crate) fn fetch(
    client: &Client,
    url: &str,
    access_token: &str,
    workspace: &str,
    now_ms: i64,
) -> Result<QuotaSnapshot, Failure> {
    let response = client
        .get(url)
        .bearer_auth(access_token)
        .header("ChatGPT-Account-Id", workspace)
        .header(ACCEPT, "application/json")
        .send()
        .map_err(|e| Failure::new(if e.is_timeout() { "timeout" } else { "network" }))?;
    let status = response.status().as_u16();
    match status {
        200..=299 => {}
        401 | 403 => {
            return Err(Failure {
                code: "unauthorized",
                status: Some(status),
            })
        }
        429 => {
            return Err(Failure {
                code: "rateLimited",
                status: Some(status),
            })
        }
        _ => {
            return Err(Failure {
                code: "http",
                status: Some(status),
            })
        }
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_BODY + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::new("network"))?;
    if bytes.len() as u64 > MAX_BODY {
        return Err(Failure::new("format"));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| Failure::new("format"))?;
    parse(&value, now_ms).ok_or(Failure::new("format"))
}

fn field<'a>(value: &'a Value, snake: &str, camel: &str) -> &'a Value {
    match &value[snake] {
        Value::Null => &value[camel],
        found => found,
    }
}

pub(crate) fn parse(value: &Value, now_ms: i64) -> Option<QuotaSnapshot> {
    let limits = field(value, "rate_limit", "rateLimit");
    let windows = [
        ("primary_window", "primaryWindow"),
        ("secondary_window", "secondaryWindow"),
    ]
    .iter()
    .filter_map(|(snake, camel)| window(field(limits, snake, camel), now_ms))
    .collect::<Vec<_>>();
    let credits = parse_credits(&value["credits"]);
    if windows.is_empty() && credits.is_none() {
        return None;
    }
    let limit_reached = field(limits, "limit_reached", "limitReached").as_bool() == Some(true)
        || limits["allowed"].as_bool() == Some(false);
    let plan = field(value, "plan_type", "planType")
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(QuotaSnapshot::new(
        windows,
        credits,
        plan,
        now_ms,
        QuotaOrigin::Network,
        limit_reached,
    ))
}

fn window(value: &Value, now_ms: i64) -> Option<QuotaWindow> {
    let seconds = field(value, "limit_window_seconds", "limitWindowSeconds")
        .as_i64()
        .filter(|s| *s > 0)?;
    let used_percent = field(value, "used_percent", "usedPercent").as_f64()?;
    let resets_at = field(value, "reset_at", "resetAt").as_i64().or_else(|| {
        field(value, "reset_after_seconds", "resetAfterSeconds")
            .as_i64()
            .map(|after| now_ms / 1000 + after)
    });
    Some(QuotaWindow {
        label: None,
        unit: None,
        minutes: seconds / 60,
        used_percent,
        resets_at,
        reset: resets_at.is_some_and(|at| at.saturating_mul(1000) <= now_ms),
        used_amount: None,
        limit_amount: None,
    })
}
