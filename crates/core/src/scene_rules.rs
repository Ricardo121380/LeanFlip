//! 场景的提醒规则：时间段、本机网段、内网域名。规则满足、而场景还没在用时，界面提醒用户
//! 切换；不会自动切换。
//!
//! 匹配是纯函数：当前时间、本机地址和域名解析都由调用方传进来（见 [`RuleContext`]）。
//! 不认识的规则（新版本写的）原样保留，永远不匹配。

use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;

/// 场景里的一条规则。认识的规则解析成 [`Rule`]，其他形状原样保留。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SceneRule {
    Known(Rule),
    Other(Value),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Rule {
    /// 每周这几天（1 = 周一 … 7 = 周日）的这段本地时间，`HH:MM`。`end` 早于 `start`
    /// 表示跨午夜：开始那天的 `start` 到第二天的 `end`。
    Time {
        days: Vec<u8>,
        start: String,
        end: String,
    },
    /// 本机地址在这个网段里，例如 `10.20.0.0/16`。
    Subnet { cidr: String },
    /// 这个主机名能解析（只在内网 DNS 里有的名字）。
    Host { host: String },
}

const KINDS: [&str; 3] = ["time", "subnet", "host"];

/// 匹配时用到的外部状态。
pub struct RuleContext<'a> {
    /// 今天星期几，1 = 周一 … 7 = 周日。
    pub weekday: u8,
    /// 现在是当天第几分钟（本地时间）。
    pub minute: u16,
    pub addresses: &'a [IpAddr],
    pub resolves: &'a dyn Fn(&str) -> bool,
}

fn invalid(zh: impl Into<String>, en: impl Into<String>) -> AppError {
    AppError::localized("scene.invalid", zh, en)
}

/// `HH:MM` → 当天第几分钟。
pub fn parse_time(text: &str) -> Option<u16> {
    let (hour, minute) = text.split_once(':')?;
    if hour.len() != 2 || minute.len() != 2 {
        return None;
    }
    let (hour, minute) = (hour.parse::<u16>().ok()?, minute.parse::<u16>().ok()?);
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

/// `地址/前缀长度`。地址不必是网段的起点，只比较前缀。
pub fn parse_cidr(text: &str) -> Option<(IpAddr, u8)> {
    let (address, prefix) = text.trim().split_once('/')?;
    let address: IpAddr = address.parse().ok()?;
    let prefix: u8 = prefix.parse().ok()?;
    let max = if address.is_ipv4() { 32 } else { 128 };
    (prefix <= max).then_some((address, prefix))
}

fn in_subnet(address: IpAddr, (network, prefix): (IpAddr, u8)) -> bool {
    let bits = |address: IpAddr| -> (u128, u32) {
        match address {
            IpAddr::V4(v4) => (u32::from(v4) as u128, 32),
            IpAddr::V6(v6) => (u128::from(v6), 128),
        }
    };
    let ((a, width), (n, other)) = (bits(address), bits(network));
    if width != other {
        return false;
    }
    let shift = width - u32::from(prefix);
    shift >= width || (a >> shift) == (n >> shift)
}

/// 主机名：1–253 个字符，字母、数字、`-` 和 `.`，不带协议和端口。
pub fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with(['.', '-'])
        && host
            .split('.')
            .all(|label| !label.is_empty() && label.len() <= 63)
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

pub fn validate(rule: &SceneRule) -> Result<(), AppError> {
    let rule = match rule {
        SceneRule::Known(rule) => rule,
        SceneRule::Other(value) => {
            // 认识的种类却解析不了，说明字段写错了；真正不认识的种类留给新版本。
            let kind = value.get("kind").and_then(Value::as_str);
            if kind.is_none_or(|kind| KINDS.contains(&kind)) {
                return Err(invalid(
                    "提醒规则的格式不对",
                    "A reminder rule is malformed",
                ));
            }
            return Ok(());
        }
    };
    match rule {
        Rule::Time { days, start, end } => {
            if days.is_empty() || days.iter().any(|day| !(1..=7).contains(day)) {
                return Err(invalid(
                    "时间规则至少要选一天",
                    "Choose at least one day for a time rule",
                ));
            }
            match (parse_time(start), parse_time(end)) {
                (Some(start), Some(end)) if start != end => Ok(()),
                (Some(_), Some(_)) => Err(invalid(
                    "时间规则的开始和结束不能相同",
                    "A time rule must start and end at different times",
                )),
                _ => Err(invalid("时间要写成 HH:MM", "Write times as HH:MM")),
            }
        }
        Rule::Subnet { cidr } => parse_cidr(cidr).map(|_| ()).ok_or_else(|| {
            invalid(
                format!("网段 {cidr} 要写成 10.0.0.0/8 这样"),
                format!("Write the network {cidr} like 10.0.0.0/8"),
            )
        }),
        Rule::Host { host } => valid_host(host).then_some(()).ok_or_else(|| {
            invalid(
                "内网域名只能包含字母、数字、- 和 .",
                "Hostnames may contain letters, digits, - and . only",
            )
        }),
    }
}

/// 这条规则现在是否满足。
pub fn matches(rule: &SceneRule, context: &RuleContext<'_>) -> bool {
    let SceneRule::Known(rule) = rule else {
        return false;
    };
    match rule {
        Rule::Time { days, start, end } => {
            let (Some(start), Some(end)) = (parse_time(start), parse_time(end)) else {
                return false;
            };
            let today = days.contains(&context.weekday);
            let yesterday = days.contains(
                &(if context.weekday == 1 {
                    7
                } else {
                    context.weekday - 1
                }),
            );
            let now = context.minute;
            if start < end {
                today && (start..end).contains(&now)
            } else {
                (today && now >= start) || (yesterday && now < end)
            }
        }
        Rule::Subnet { cidr } => parse_cidr(cidr).is_some_and(|network| {
            context
                .addresses
                .iter()
                .any(|&address| in_subnet(address, network))
        }),
        Rule::Host { host } => valid_host(host) && (context.resolves)(host),
    }
}
