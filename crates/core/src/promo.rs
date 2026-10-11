//! 认出推广链接（返利 / 邀请 / 追踪 / 短链），规则和 `scripts/build-presets.mjs` 一致。
//! 导入 cc-switch 的卡时用：这类 `websiteUrl` 不带过来。

const TRACKING_PARAMS: &[&str] = &[
    "aff",
    "track_id",
    "ref",
    "invite",
    "invite_code",
    "invitecode",
    "ic",
    "code",
    "source",
    "from",
    "ch",
    "ytag",
    "ac",
    "rc",
];
const REFERRAL_PREFIXES: &[&str] = &["i", "invite", "r", "ref", "go", "aff", "s"];
const SHORTENER_HOSTS: &[&str] = &[
    "s.qiniu.com",
    "bit.ly",
    "t.cn",
    "dwz.cn",
    "tinyurl.com",
    "url.cn",
];

pub fn is_promo_link(url: &str) -> bool {
    let Some((_, rest)) = url.trim().split_once("://") else {
        return false;
    };
    let rest = rest.split('#').next().unwrap_or("");
    let (before_query, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, path) = match before_query.find('/') {
        Some(i) => (&before_query[..i], &before_query[i..]),
        None => (before_query, ""),
    };
    let host = host.to_ascii_lowercase();
    if SHORTENER_HOSTS.contains(&host.as_str()) {
        return true;
    }
    for pair in query.split('&') {
        let key = pair.split('=').next().unwrap_or("").to_ascii_lowercase();
        if TRACKING_PARAMS.contains(&key.as_str()) || key.starts_with("utm_") {
            return true;
        }
    }
    referral_path(path)
}

fn referral_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.contains("ccswitch") || lower.contains("cc-switch") {
        return true;
    }
    let segments: Vec<&str> = path.trim_end_matches('/').split('/').skip(1).collect();
    match segments.as_slice() {
        [prefix, code] if REFERRAL_PREFIXES.contains(&prefix.to_ascii_lowercase().as_str()) => {
            !code.is_empty()
        }
        ["register", code] | ["agent", "register", code] => {
            code.len() >= 6
                && code
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        }
        [code] => short_code(code),
        _ => false,
    }
}

/// 单段随机短码：5–10 位字母数字，同时有数字和大写字母（`/VjM74M`）。
fn short_code(code: &str) -> bool {
    (5..=10).contains(&code.len())
        && code.chars().all(|c| c.is_ascii_alphanumeric())
        && code.chars().any(|c| c.is_ascii_digit())
        && code.chars().any(|c| c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::is_promo_link;

    #[test]
    fn recognises_referral_tracking_and_short_links() {
        for url in [
            "https://platform.kimi.com?track_id=x&aff=cc-switch",
            "https://example.com/?utm_source=a",
            "https://aicoding.inc/i/CCSWITCH",
            "https://soleapi.com/r/abc",
            "https://zetaapi.ai/go/u117",
            "https://code0.ai/agent/register/B2XHxGjGmRvqgznY",
            "https://apinebula.ai/VjM74M",
            "https://s.qiniu.com/nMvAvy",
            "https://ppio.com/activity/ccswitch",
        ] {
            assert!(is_promo_link(url), "{url}");
        }
        for url in [
            "https://platform.deepseek.com",
            "https://openrouter.ai/keys",
            "https://aicodewith.ai/login?tab=register",
            "https://developer.ant-ling.com/zh-CN/docs/",
            "https://www.relaxycode.com/register",
            "not a url",
        ] {
            assert!(!is_promo_link(url), "{url}");
        }
    }
}
