//! `leanflip quota [--refresh]`. Without `--refresh` it shows what is already on this
//! machine (Codex session logs and the quota cache) and never goes online or writes.
//! Output carries card names, numbers and fixed error codes only, never keys or tokens.
use leanflip_service::quota::{QuotaError, QuotaLevel, QuotaOrigin, QuotaSnapshot, QuotaSource};
use leanflip_service::Service;
use serde_json::json;

use crate::args::Language;
use crate::presentation::{clean, table};
use crate::{CliError, Output};

pub fn run(service: &Service, refresh: bool, lang: Language) -> Result<Output, CliError> {
    let view = if refresh {
        service.quota_refresh(None)?
    } else {
        service.quota_overview()?
    };
    let now = view.checked_at;
    let mut text = format!(
        "{} · {} {}",
        lang.text("LeanFlip · 额度", "LeanFlip · Quota"),
        view.sources.len(),
        lang.text("个来源", "source(s)")
    );
    if view.sources.is_empty() {
        text.push_str(lang.text(
            "\n\n还没有额度来源。Codex 官方登录在本机用过之后会出现；中转卡使用 leanflip quota --refresh 自动识别平台并查询余额。",
            "\n\nNo quota sources yet. A Codex ChatGPT sign-in shows up after it is used on this machine; use leanflip quota --refresh to detect relay platforms and check their balances.",
        ));
    } else {
        let rows = view
            .sources
            .iter()
            .map(|source| {
                vec![
                    source_name(source),
                    users(source),
                    source
                        .snapshot
                        .as_ref()
                        .map_or_else(|| "—".into(), |s| amounts(s, lang)),
                    status(source, lang),
                    source
                        .snapshot
                        .as_ref()
                        .map_or_else(|| "—".into(), |s| updated(s, now, lang)),
                ]
            })
            .collect::<Vec<_>>();
        text.push_str(&format!(
            "\n\n{}",
            table(
                &[
                    lang.text("来源", "SOURCE"),
                    lang.text("使用的卡", "USED BY"),
                    lang.text("额度", "QUOTA"),
                    lang.text("状态", "STATE"),
                    lang.text("更新", "UPDATED"),
                ],
                &rows
            )
        ));
        if !refresh {
            text.push_str(lang.text(
                "\n\n联网查询托管账号和中转站：leanflip quota --refresh",
                "\n\nTo query managed accounts and relays online: leanflip quota --refresh",
            ));
        }
    }
    Ok(Output::new(json!(view), text))
}

fn source_name(source: &QuotaSource) -> String {
    let label = source.label.as_deref().map(clean);
    match (&source.adapter, label) {
        (Some(adapter), Some(label)) => {
            format!("{label} ({})", json!(adapter).as_str().unwrap_or(""))
        }
        (Some(adapter), None) => json!(adapter).as_str().unwrap_or("relay").to_string(),
        (None, Some(label)) => label,
        (None, None) => "Codex".into(),
    }
}

fn users(source: &QuotaSource) -> String {
    let mut users = source.users.iter().collect::<Vec<_>>();
    // Cards in use come first and carry a `*`.
    users.sort_by_key(|user| !user.active);
    let names = users
        .iter()
        .map(|user| {
            format!(
                "{} · {}{}",
                user.tool.as_str(),
                clean(&user.provider_name),
                if user.active { " *" } else { "" }
            )
        })
        .collect::<Vec<_>>();
    if names.is_empty() {
        "—".into()
    } else {
        names.join(", ")
    }
}

fn window_label(minutes: i64, lang: Language) -> String {
    const DAY: i64 = 24 * 60;
    match minutes {
        m if m == 7 * DAY => lang.text("周", "week").into(),
        m if m % DAY == 0 => format!("{}{}", m / DAY, lang.text(" 天", "d")),
        m if m % 60 == 0 => format!("{}{}", m / 60, lang.text(" 小时", "h")),
        m => format!("{m}{}", lang.text(" 分钟", "m")),
    }
}

fn money(amount: f64, unit: &str) -> String {
    if unit.eq_ignore_ascii_case("USD") {
        format!("${amount:.2}")
    } else {
        format!("{amount:.2} {}", clean(unit))
    }
}

/// Every window, then the balance or credits, e.g. `5 小时 42% · 周 18%`.
fn amounts(snapshot: &QuotaSnapshot, lang: Language) -> String {
    let mut parts = snapshot
        .windows
        .iter()
        .map(|w| {
            let label = w
                .label
                .as_ref()
                .map(|label| clean(label))
                .unwrap_or_else(|| window_label(w.minutes, lang));
            if w.reset {
                format!("{} {}", label, lang.text("已重置", "reset"))
            } else {
                format!("{} {:.0}%", label, w.used_percent)
            }
        })
        .collect::<Vec<_>>();
    if let Some(balance) = &snapshot.balance {
        if balance.unlimited {
            parts.push(lang.text("Key 未设限", "Key has no cap").into());
            if balance.account_remaining.is_none() {
                parts.push(
                    lang.text("账户余额未提供", "account balance unavailable")
                        .into(),
                );
            }
        } else if let Some(remaining) = balance.remaining {
            let mut part = format!(
                "{} {}",
                lang.text("余", "left"),
                money(remaining, &balance.unit)
            );
            if let Some(limit) = balance.limit {
                part.push_str(&format!(" / {}", money(limit, &balance.unit)));
            }
            parts.push(part);
        }
        if let Some(remaining) = balance.account_remaining {
            parts.push(format!(
                "{} {}",
                lang.text("账户余额", "account balance"),
                money(remaining, &balance.unit)
            ));
        }
    }
    if let Some(credits) = &snapshot.credits {
        if credits.unlimited {
            parts.push(lang.text("credits 不限", "credits unlimited").into());
        } else if let Some(balance) = credits.balance.filter(|_| credits.has_credits) {
            parts.push(format!("credits {balance:.2}"));
        }
    }
    if parts.is_empty() {
        "—".into()
    } else {
        parts.join(" · ")
    }
}

fn status(source: &QuotaSource, lang: Language) -> String {
    let level = source.snapshot.as_ref().map(|s| match s.level {
        QuotaLevel::Ok => lang.text("正常", "OK"),
        QuotaLevel::Warn => lang.text("快用完", "Low"),
        QuotaLevel::Out => lang.text("已用完", "Out"),
    });
    match (level, &source.error) {
        (Some(level), Some(error)) => {
            format!("{level}{}{}", lang.text("，", "; "), failure(error, lang))
        }
        (Some(level), None) => level.into(),
        (None, Some(error)) => failure(error, lang),
        (None, None) => lang.text("暂无数据", "No data").into(),
    }
}

fn failure(error: &QuotaError, lang: Language) -> String {
    let reason = match error.code.as_str() {
        "reauth" => lang.text("需要重新登录", "sign-in required"),
        "unauthorized" => lang.text("Key 无效或无权限", "key rejected"),
        "krillAuth" => lang.text("请更新 Krill 登录 JWT", "update the Krill login JWT"),
        "rateLimited" => lang.text("请求太频繁", "rate limited"),
        "timeout" => lang.text("查询超时", "timed out"),
        "network" => lang.text("连不上", "unreachable"),
        "token" => lang.text("取不到登录凭据", "no sign-in token"),
        "format" => lang.text("返回内容认不出", "unrecognized response"),
        _ => match error.status {
            Some(status) => return format!("{} HTTP {status}", lang.text("查询失败：", "failed:")),
            None => lang.text("查询失败", "failed"),
        },
    };
    reason.into()
}

fn updated(snapshot: &QuotaSnapshot, now: i64, lang: Language) -> String {
    let origin = match snapshot.origin {
        QuotaOrigin::Local => lang.text("本机", "local"),
        QuotaOrigin::Network => lang.text("联网", "online"),
    };
    let minutes = (now - snapshot.fetched_at).max(0) / 60_000;
    let ago = match minutes {
        0 => lang.text("刚刚", "just now").into(),
        m if m < 60 => format!("{m}{}", lang.text(" 分钟前", "m ago")),
        m if m < 48 * 60 => format!("{}{}", m / 60, lang.text(" 小时前", "h ago")),
        m => format!("{}{}", m / (24 * 60), lang.text(" 天前", "d ago")),
    };
    format!("{origin} · {ago}")
}
