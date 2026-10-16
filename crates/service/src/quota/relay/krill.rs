//! Krill 官网 `/api/subscription`，使用独立的网站登录 JWT。
//! 字段核对自官网 DashboardPage / TutorialPage 的公开客户端（2026-10-16）。
//! 只读取当前有效套餐；金额已是 USD。日套餐的 total_* 包含结转，不能只看当天额度。

use super::*;

#[cfg(test)]
mod tests;

pub(super) fn parse(value: &Value, now_ms: i64) -> Option<QuotaSnapshot> {
    if value["success"].as_bool() == Some(false) {
        return None;
    }
    let data = &value["data"];
    let subscriptions = data["subscriptions"].as_array()?;
    let account = number(&data["credit_balance_usd"]);
    let mut windows = Vec::new();
    let mut names = Vec::new();
    let mut request_plans = Vec::new();
    for subscription in subscriptions {
        if subscription["status"] != "active"
            || time(&subscription["subscription_end_at"])
                .is_some_and(|at| at.saturating_mul(1000) <= now_ms)
        {
            continue;
        }
        let plan = &subscription["plan"];
        let name = text(&plan["name"]);
        let quota = &subscription["quota"];
        let (minutes, used, limit, reset) = match plan["billing_type"].as_str() {
            Some("usd_daily") => {
                // 官网日套餐优先显示可用总额，包含结转；没有 total_* 的旧返回使用当天额度。
                if let (Some(used), Some(limit)) = (
                    number(&subscription["total_used_usd"]),
                    number(&subscription["total_limit_usd"]),
                ) {
                    let days = plan["duration_days"]
                        .as_i64()
                        .filter(|days| *days > 0)
                        .unwrap_or(1);
                    (days.saturating_mul(1440), Some(used), Some(limit), None)
                } else {
                    (
                        1440,
                        number(&quota["used_usd"]),
                        number(&quota["daily_limit_usd"]),
                        time(&quota["window_reset_at"]),
                    )
                }
            }
            Some("usd_weekly" | "usd_monthly") => (
                if plan["billing_type"] == "usd_weekly" {
                    10080
                } else {
                    43200
                },
                number(&quota["used_usd"]),
                number(&quota["daily_limit_usd"]),
                time(&quota["window_reset_at"]),
            ),
            Some("request_count") => {
                request_plans.push(name.unwrap_or_else(|| "Krill".into()));
                continue;
            }
            _ => continue,
        };
        let (Some(used), Some(limit)) = (used, limit.filter(|limit| *limit > 0.0)) else {
            continue;
        };
        let mut window = amount_window(minutes, used, limit, reset, now_ms);
        window.label = name.clone();
        windows.push(window);
        if let Some(name) = name {
            names.push(name);
        }
    }
    // 按次额度是账户共享的窗口，多个按次套餐不能重复计入。
    if !request_plans.is_empty() {
        let quota = &data["request_count_quota"];
        for (period, minutes) in [("5h", 300), ("weekly", 10080), ("monthly", 43200)] {
            let (Some(used), Some(limit)) = (
                number(&quota[format!("used_{period}")]),
                number(&quota[format!("limit_{period}")]).filter(|limit| *limit > 0.0),
            ) else {
                continue;
            };
            let mut window = amount_window(minutes, used, limit, None, now_ms);
            window.label = Some(request_plans.join(" / "));
            window.unit = Some("requests".into());
            windows.push(window);
        }
        names.extend(request_plans);
    }
    if windows.is_empty() && account.is_none() {
        return None;
    }
    let has_plans = !windows.is_empty();
    let balance = account.map(|account| QuotaBalance {
        unit: "USD".into(),
        remaining: (!has_plans).then_some(account),
        account_remaining: has_plans.then_some(account),
        used: None,
        limit: None,
        unlimited: false,
        expires_at: None,
    });
    Some(
        QuotaSnapshot::new(
            windows,
            None,
            (!names.is_empty()).then(|| names.join(" / ")),
            now_ms,
            QuotaOrigin::Network,
            false,
        )
        .with_balance(balance),
    )
}
