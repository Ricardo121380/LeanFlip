//! 额度：订阅窗口的已用百分比、credits 和中转站余额。
//!
//! 三个来源：Codex 本机会话记录里的 `rate_limits`（不联网）；托管 ChatGPT 账号的
//! `wham/usage` 查询；中转站（sub2api、NewAPI）按卡片里的 Key 查询（见 [`relay`]）。
//! 联网查询只在用户点刷新或识别时发生，结果缓存在 `quota-cache.json`。
//!
//! 数据按 ChatGPT 身份（工作区 + `chatgpt_user_id`）归档。身份的持有者是托管账号和本机登录
//! （原生登录、登录暂存）；Codex 官方卡再对到持有者：绑定托管账号的卡用该账号，引用登录的卡
//! 用那份登录，普通官方卡用本机原生登录。同一个身份只算一个来源，几张卡共用时只查一次。

pub(crate) mod cache;
pub(crate) mod codex_log;
pub(crate) mod codex_remote;
pub mod relay;
#[cfg(test)]
mod test_server;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use leanflip_core::codex::auth::{
    extract_chatgpt_account_id, extract_chatgpt_user_id, extract_codex_auth_api_key,
};
use leanflip_core::codex::login::LoginSummary;
use leanflip_core::codex::oauth::CodexOAuthManager;
use leanflip_core::store::StoreData;
use leanflip_core::Tool;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// 用到这个比例就提醒。
const WARN_FRACTION: f64 = 0.8;

/// 一个限额窗口。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    /// 窗口长度。不能假定 primary 就是 5 小时：有的套餐只有周窗口。
    pub minutes: i64,
    pub used_percent: f64,
    /// 重置时间，Unix 秒。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
    /// 记录之后已经过了重置时间，`used_percent` 不再可信，等下一条记录更新。
    #[serde(default)]
    pub reset: bool,
    /// 中转站的金额窗口：已用和上限（和余额同一单位）。订阅窗口没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_amount: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_amount: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaCredits {
    pub has_credits: bool,
    pub unlimited: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<f64>,
}

/// 中转站的余额。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaBalance {
    /// 目前都是 `USD`。
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    /// 总额度；钱包和订阅没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(default)]
    pub unlimited: bool,
    /// Key 的过期时间，Unix 秒。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// 账户余额（NewAPI 允许时随 Key 返回的 `user_quota`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_remaining: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuotaOrigin {
    /// 客户端留在本机的记录，是最后一次请求时的数据。
    Local,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuotaLevel {
    Ok,
    /// 用了 80% 以上。
    Warn,
    /// 用完，或服务端说已经触发限额。
    Out,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSnapshot {
    /// 按窗口长度从短到长。
    pub windows: Vec<QuotaWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<QuotaCredits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<QuotaBalance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// 数据对应的时间，毫秒。本机记录是那次请求的时间，联网查询是查询时间。
    pub fetched_at: i64,
    pub origin: QuotaOrigin,
    pub level: QuotaLevel,
    /// 未重置窗口和有总额的余额里最高的已用比例（0–1），用来挑“最紧张”的一项。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_fraction: Option<f64>,
    #[serde(default)]
    pub limit_reached: bool,
}

impl QuotaSnapshot {
    pub(crate) fn new(
        mut windows: Vec<QuotaWindow>,
        credits: Option<QuotaCredits>,
        plan: Option<String>,
        fetched_at: i64,
        origin: QuotaOrigin,
        limit_reached: bool,
    ) -> Self {
        windows.sort_by_key(|w| w.minutes);
        let mut snapshot = Self {
            windows,
            credits,
            balance: None,
            plan,
            fetched_at,
            origin,
            level: QuotaLevel::Ok,
            used_fraction: None,
            limit_reached,
        };
        snapshot.assess();
        snapshot
    }

    pub(crate) fn with_balance(mut self, balance: Option<QuotaBalance>) -> Self {
        self.balance = balance;
        self.assess();
        self
    }

    fn assess(&mut self) {
        let balance = self.balance.as_ref().filter(|b| !b.unlimited);
        let balance_fraction = balance.and_then(|b| match (b.used, b.limit) {
            (Some(used), Some(limit)) if limit > 0.0 => Some(used / limit),
            _ => None,
        });
        self.used_fraction = self
            .windows
            .iter()
            .filter(|w| !w.reset)
            .map(|w| w.used_percent / 100.0)
            .chain(balance_fraction)
            .map(|f| f.clamp(0.0, 1.0))
            .reduce(f64::max);
        let empty = balance
            .and_then(|b| b.remaining)
            .is_some_and(|left| left <= 0.0);
        self.level = match self.used_fraction {
            _ if self.limit_reached || empty => QuotaLevel::Out,
            Some(f) if f >= 1.0 => QuotaLevel::Out,
            Some(f) if f >= WARN_FRACTION => QuotaLevel::Warn,
            _ => QuotaLevel::Ok,
        };
    }
}

/// `credits` 对象：本机记录和联网查询格式相同，`balance` 是字符串。
pub(crate) fn parse_credits(value: &Value) -> Option<QuotaCredits> {
    let c = value.as_object()?;
    let flag = |a: &str, b: &str| {
        c.get(a)
            .or_else(|| c.get(b))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    Some(QuotaCredits {
        has_credits: flag("has_credits", "hasCredits"),
        unlimited: flag("unlimited", "unlimited"),
        balance: c.get("balance").and_then(|b| match b {
            Value::String(s) => s.trim().parse().ok(),
            other => other.as_f64(),
        }),
    })
}

/// 最近一次联网查询失败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaError {
    /// `reauth`（要重新登录）、`unauthorized`、`rateLimited`、`http`、`timeout`、`network`、
    /// `format`（返回内容认不出）、`token`（取不到登录凭据）。
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    pub at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QuotaKind {
    /// Codex 官方登录的订阅窗口。
    Codex,
    /// 中转站：按 Key 查的余额和限额。
    Relay,
}

/// 用这个来源的一张卡。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaUser {
    pub tool: Tool,
    pub provider_id: String,
    pub provider_name: String,
    /// 是这个工具当前的供应商。
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSource {
    /// `codex:<工作区>:<chatgpt_user_id>`；只知道工作区时用户段是 `*`。
    pub key: String,
    pub kind: QuotaKind,
    /// 账号邮箱，没有时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// 对应的托管账号 id。有它才能联网查询。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// 对应的本机登录（`LoginSummary::identity`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// 中转站的平台类型。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<relay::RelayAdapter>,
    /// 中转站的自动刷新间隔（分钟，共用的几张卡里最短的）；不自动刷新时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_minutes: Option<u32>,
    pub users: Vec<QuotaUser>,
    /// 还没有数据时为空（最近 8 天没在本机用过，也没联网查过）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<QuotaSnapshot>,
    /// 比现有数据更新的一次查询失败。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<QuotaError>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaView {
    pub sources: Vec<QuotaSource>,
    /// 正在用的来源里最紧张的一个，状态栏数字显示它。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busiest: Option<String>,
    /// 读取时间，毫秒。
    pub checked_at: i64,
}

/// ChatGPT 身份。会话记录里两项都有；旧的登录凭据可能只知道工作区。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CodexIdentity {
    pub workspace: String,
    #[serde(default)]
    pub user: Option<String>,
}

impl CodexIdentity {
    pub(crate) fn key(&self) -> String {
        format!(
            "codex:{}:{}",
            self.workspace,
            self.user.as_deref().unwrap_or("*")
        )
    }

    /// 这个身份（可能缺用户）能不能认领一条观测到的身份。
    fn covers(&self, seen: &CodexIdentity) -> bool {
        self.workspace == seen.workspace
            && self
                .user
                .as_ref()
                .is_none_or(|user| seen.user.as_ref() == Some(user))
    }
}

/// 某个身份某一时刻的额度：来自会话记录或联网查询。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Observation {
    pub identity: CodexIdentity,
    pub snapshot: QuotaSnapshot,
}

/// 持有一个身份的托管账号或本机登录。
#[derive(Debug, Clone)]
pub(crate) struct Holder {
    pub identity: CodexIdentity,
    pub label: Option<String>,
    pub account: Option<String>,
    pub login: Option<String>,
    /// 正在 `auth.json` 里生效的原生登录。
    pub live: bool,
}

fn nonblank(text: Option<String>) -> Option<String> {
    text.filter(|s| !s.trim().is_empty())
}

/// 托管账号的身份。旧记录可能没存工作区（这样的账号要重新登录），这时从 id_token 里读。
pub(crate) fn account_identity(oauth: &CodexOAuthManager, account: &str) -> Option<CodexIdentity> {
    let data = oauth.try_account(account).ok().flatten()?;
    let id_token = data.id_token.as_deref();
    Some(CodexIdentity {
        workspace: nonblank(data.chatgpt_account_id)
            .or_else(|| id_token.and_then(extract_chatgpt_account_id))?,
        user: id_token.and_then(extract_chatgpt_user_id),
    })
}

/// 所有托管账号和本机登录。
pub(crate) fn holders(oauth: &CodexOAuthManager, logins: &[LoginSummary]) -> Vec<Holder> {
    let accounts = oauth.try_list_accounts().unwrap_or_else(|err| {
        log::warn!("quota: managed accounts unreadable: {err}");
        Vec::new()
    });
    let managed = accounts.into_iter().filter_map(|info| {
        Some(Holder {
            identity: account_identity(oauth, &info.id)?,
            label: info.email,
            account: Some(info.id),
            login: None,
            live: false,
        })
    });
    let local = logins.iter().filter_map(|s| {
        Some(Holder {
            identity: CodexIdentity {
                workspace: nonblank(s.workspace.clone())?,
                user: s.chatgpt_user_id.clone(),
            },
            label: s.email.clone(),
            account: None,
            login: Some(s.identity.clone()),
            live: s.live,
        })
    });
    managed.chain(local).collect()
}

/// 一张 Codex 官方卡和它对应的身份。
#[derive(Debug, Clone)]
pub(crate) struct CodexCard {
    pub provider_id: String,
    pub provider_name: String,
    pub active: bool,
    pub identity: CodexIdentity,
}

/// 找出 Codex 官方卡各自用哪个身份。认不出的卡不算：存了 API Key 的官方卡、绑定的账号
/// 已删除、引用的登录已不在暂存里、没有任何原生登录。
pub(crate) fn codex_cards(
    data: &StoreData,
    holders: &[Holder],
    default_account: Option<&str>,
) -> Vec<CodexCard> {
    let current = data.current(Tool::Codex).map(|p| p.id.as_str());
    let logins: Vec<&Holder> = holders.iter().filter(|h| h.login.is_some()).collect();
    // 普通官方卡切回来时用本机原生登录：正在生效的那份，否则暂存里唯一的那份。
    let native = logins
        .iter()
        .find(|h| h.live)
        .or_else(|| (logins.len() == 1).then(|| &logins[0]))
        .copied();
    data.providers(Tool::Codex)
        .iter()
        .filter(|p| p.is_official_category())
        .filter(|p| extract_codex_auth_api_key(&p.settings_config["auth"]).is_none())
        .filter_map(|p| {
            let holder = if let Some(bound) = p.managed_codex_account() {
                // 绑定时没写账号 id：用默认托管账号。
                let id = if bound.is_empty() {
                    default_account?
                } else {
                    bound
                };
                holders.iter().find(|h| h.account.as_deref() == Some(id))?
            } else if let Some(reference) = p.codex_login_ref() {
                logins
                    .iter()
                    .find(|h| h.login.as_deref() == Some(reference))
                    .copied()?
            } else {
                native?
            };
            Some(CodexCard {
                provider_id: p.id.clone(),
                provider_name: p.name.clone(),
                active: current == Some(p.id.as_str()),
                identity: holder.identity.clone(),
            })
        })
        .collect()
}

/// 按身份合并持有者和卡片，配上最新的数据和比它更新的错误。
pub(crate) fn build_view(
    holders: Vec<Holder>,
    cards: Vec<CodexCard>,
    seen: Vec<Observation>,
    errors: &HashMap<String, QuotaError>,
    now_ms: i64,
) -> QuotaView {
    let mut sources: Vec<(CodexIdentity, QuotaSource)> = Vec::new();
    let source = |identity: &CodexIdentity, label: Option<String>| QuotaSource {
        key: identity.key(),
        kind: QuotaKind::Codex,
        label,
        account_id: None,
        login: None,
        adapter: None,
        refresh_minutes: None,
        users: Vec::new(),
        snapshot: None,
        error: None,
    };
    for holder in holders {
        let index = match sources.iter().position(|(id, _)| *id == holder.identity) {
            Some(index) => index,
            None => {
                sources.push((holder.identity.clone(), source(&holder.identity, None)));
                sources.len() - 1
            }
        };
        let entry = &mut sources[index].1;
        entry.label = entry.label.take().or(holder.label);
        entry.account_id = entry.account_id.take().or(holder.account);
        entry.login = entry.login.take().or(holder.login);
    }
    for card in cards {
        let index = match sources.iter().position(|(id, _)| *id == card.identity) {
            Some(index) => index,
            None => {
                sources.push((card.identity.clone(), source(&card.identity, None)));
                sources.len() - 1
            }
        };
        sources[index].1.users.push(QuotaUser {
            tool: Tool::Codex,
            provider_id: card.provider_id,
            provider_name: card.provider_name,
            active: card.active,
        });
    }
    let sources: Vec<QuotaSource> = sources
        .into_iter()
        .map(|(identity, mut source)| {
            source.snapshot = seen
                .iter()
                .filter(|o| identity.covers(&o.identity))
                .max_by_key(|o| o.snapshot.fetched_at)
                .map(|o| o.snapshot.clone());
            let newest = source.snapshot.as_ref().map_or(i64::MIN, |s| s.fetched_at);
            source.error = errors.get(&source.key).filter(|e| e.at > newest).cloned();
            source
        })
        .collect();
    let busiest = busiest(&sources);
    QuotaView {
        sources,
        busiest,
        checked_at: now_ms,
    }
}

/// 正在用的来源里最紧张的一个：先比等级，再比已用比例。只有余额、算不出比例的（钱包）
/// 排在最后，但只有它时也选它。
pub(crate) fn busiest(sources: &[QuotaSource]) -> Option<String> {
    sources
        .iter()
        .filter(|s| s.users.iter().any(|u| u.active))
        .filter_map(|s| {
            let snap = s.snapshot.as_ref()?;
            let fraction = snap.used_fraction.unwrap_or(match snap.level {
                QuotaLevel::Out => 1.0,
                _ => 0.0,
            });
            Some((s, fraction, snap.level))
        })
        .max_by(|a, b| a.2.cmp(&b.2).then(a.1.total_cmp(&b.1)))
        .map(|(s, _, _)| s.key.clone())
}

fn window_short(minutes: i64, english: bool) -> String {
    match minutes {
        10080 => (if english { "wk" } else { "周" }).to_string(),
        1440 => (if english { "day" } else { "日" }).to_string(),
        m if m % 1440 == 0 => format!("{}{}", m / 1440, if english { "d" } else { "天" }),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

fn money(amount: f64, unit: &str) -> String {
    if unit.eq_ignore_ascii_case("USD") {
        format!("${amount:.2}")
    } else {
        format!("{amount:.2} {unit}")
    }
}

/// 托盘和状态栏用的一小段文字。快满的窗口优先，其次是余额，再其次是最满的窗口；
/// 用量过 80% 或用完时前面加 ⚠︎。没有可显示的数字时为空。
pub fn brief(snapshot: &QuotaSnapshot, english: bool) -> Option<String> {
    let worst = snapshot
        .windows
        .iter()
        .filter(|w| !w.reset)
        .max_by(|a, b| a.used_percent.total_cmp(&b.used_percent));
    let window = |w: &QuotaWindow| {
        format!(
            "{} {}%",
            window_short(w.minutes, english),
            w.used_percent.round() as i64
        )
    };
    let text = match (&snapshot.balance, worst) {
        (_, Some(w)) if w.used_percent >= WARN_FRACTION * 100.0 => window(w),
        (Some(b), _) if b.unlimited => (if english { "unlimited" } else { "不限" }).to_string(),
        (Some(b), _) if b.remaining.is_some() => money(b.remaining.unwrap_or_default(), &b.unit),
        (_, Some(w)) => window(w),
        _ => {
            let balance = snapshot.credits.as_ref()?.balance?;
            format!("credits {balance:.0}")
        }
    };
    Some(match snapshot.level {
        QuotaLevel::Ok => text,
        _ => format!("\u{26A0}\u{FE0E} {text}"),
    })
}
