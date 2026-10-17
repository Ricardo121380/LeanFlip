use std::fs;

use leanflip_core::{store, Env, Provider, Tool};
use serde_json::{json, Value};

use super::super::test_server::{self, serve};
use super::super::{QuotaKind, QuotaLevel};
use super::*;
use crate::Service;

/// 2026-10-15T04:00:00Z
const NOW: i64 = 1_792_036_800_000;

/// sub2api 按 Key 限额：总额度加 5h / 7d 金额窗口（字段取自 gateway_handler.go 的 usageQuotaLimited）。
fn sub2api_limited() -> Value {
    json!({
        "mode": "quota_limited",
        "isValid": true,
        "status": "active",
        "quota": {"limit": 50.0, "used": 18.6, "remaining": 31.4, "unit": "USD"},
        "remaining": 31.4,
        "unit": "USD",
        "rate_limits": [
            {"window": "5h", "limit": 5.0, "used": 4.2, "remaining": 0.8,
             "window_start": "2026-10-15T01:00:00Z", "reset_at": "2026-10-15T06:00:00Z"},
            {"window": "7d", "limit": 30.0, "used": 9.0, "remaining": 21.0,
             "window_start": "2026-10-10T00:00:00Z"}
        ],
        "expires_at": "2026-12-31T00:00:00Z",
        "usage": {"today": {"cost": 1.2, "actual_cost": 1.1}},
        "model_stats": [{"model": "x"}]
    })
}

/// NewAPI `/api/usage/token/`（controller/token.go 的 GetTokenUsage）。
fn newapi_usage(available: i64, unlimited: bool) -> Value {
    json!({
        "code": true,
        "message": "ok",
        "data": {
            "object": "token_usage",
            "name": "work",
            "total_granted": available + 13_300_000,
            "total_used": 13_300_000,
            "total_available": available,
            "unlimited_quota": unlimited,
            "model_limits": {},
            "model_limits_enabled": false,
            "expires_at": 0,
            "user_quota": {"total_available": 60_000_000, "total_used": 1_000}
        }
    })
}

#[test]
fn sub2api_key_quota_and_amount_windows() {
    let snap = parse(
        RelayAdapter::Sub2api,
        &sub2api_limited(),
        DEFAULT_QUOTA_PER_USD,
        NOW,
    )
    .unwrap();
    let balance = snap.balance.as_ref().unwrap();
    assert_eq!(
        (
            balance.remaining,
            balance.limit,
            balance.used,
            balance.unit.as_str()
        ),
        (Some(31.4), Some(50.0), Some(18.6), "USD")
    );
    assert_eq!(balance.expires_at, Some(1_798_675_200));
    let windows: Vec<_> = snap
        .windows
        .iter()
        .map(|w| {
            (
                w.minutes,
                (w.used_percent * 10.0).round() / 10.0,
                w.used_amount,
                w.limit_amount,
            )
        })
        .collect();
    assert_eq!(
        windows,
        [
            (300, 84.0, Some(4.2), Some(5.0)),
            (10080, 30.0, Some(9.0), Some(30.0))
        ]
    );
    assert_eq!(snap.windows[0].resets_at, Some(NOW / 1000 + 2 * 3600));
    // 5h 窗口用了 84%：整体要提醒。
    assert_eq!(snap.level, QuotaLevel::Warn);
}

#[test]
fn sub2api_subscription_and_wallet_modes() {
    let subscription = json!({
        "mode": "unrestricted",
        "isValid": true,
        "planName": "OpenAI-Max",
        "unit": "USD",
        "remaining": 104.9,
        "subscription": {
            "daily_usage_usd": 5.1, "daily_limit_usd": 110.0,
            "weekly_usage_usd": 49.94, "weekly_limit_usd": 800.0,
            "monthly_usage_usd": 224.84, "monthly_limit_usd": null,
            "weekly_window_start": "2026-10-13T00:00:00Z",
            "expires_at": "2026-11-01T00:00:00Z"
        }
    });
    let snap = parse(
        RelayAdapter::Sub2api,
        &subscription,
        DEFAULT_QUOTA_PER_USD,
        NOW,
    )
    .unwrap();
    assert_eq!(snap.plan.as_deref(), Some("OpenAI-Max"));
    // 没有上限的月窗口不算。
    assert_eq!(
        snap.windows.iter().map(|w| w.minutes).collect::<Vec<_>>(),
        [1440, 10080]
    );
    assert_eq!(snap.windows[1].resets_at, Some(1_792_454_400));
    assert_eq!(snap.balance.as_ref().unwrap().remaining, Some(104.9));
    assert_eq!(
        snap.balance.as_ref().unwrap().expires_at,
        Some(1_793_491_200)
    );

    let wallet = json!({"mode": "unrestricted", "isValid": true, "planName": "钱包余额",
                        "remaining": 0.0, "unit": "USD", "balance": 0.0});
    let snap = parse(RelayAdapter::Sub2api, &wallet, DEFAULT_QUOTA_PER_USD, NOW).unwrap();
    assert!(snap.windows.is_empty());
    assert_eq!(snap.level, QuotaLevel::Out);
}

#[test]
fn sub2api_exhausted_or_expired_keys_are_out() {
    let mut body = sub2api_limited();
    body["status"] = json!("quota_exhausted");
    let snap = parse(RelayAdapter::Sub2api, &body, DEFAULT_QUOTA_PER_USD, NOW).unwrap();
    assert!(snap.limit_reached);
    assert_eq!(snap.level, QuotaLevel::Out);
}

#[test]
fn newapi_converts_quota_to_dollars_and_reads_the_account_balance() {
    let snap = parse(
        RelayAdapter::Newapi,
        &newapi_usage(15_700_000, false),
        DEFAULT_QUOTA_PER_USD,
        NOW,
    )
    .unwrap();
    let b = snap.balance.as_ref().unwrap();
    assert_eq!(
        (b.remaining, b.used, b.limit),
        (Some(31.4), Some(26.6), Some(58.0))
    );
    assert_eq!(b.account_remaining, Some(120.0));
    assert_eq!(b.expires_at, None);
    assert!((snap.used_fraction.unwrap() - 26.6 / 58.0).abs() < 1e-9);
    assert_eq!(snap.level, QuotaLevel::Ok);

    // 站点改了换算比例时按卡片上的设置算。
    let snap = parse(
        RelayAdapter::Newapi,
        &newapi_usage(15_700_000, false),
        1_000_000.0,
        NOW,
    )
    .unwrap();
    assert_eq!(snap.balance.unwrap().remaining, Some(15.7));

    let unlimited = parse(
        RelayAdapter::Newapi,
        &newapi_usage(0, true),
        DEFAULT_QUOTA_PER_USD,
        NOW,
    )
    .unwrap();
    let b = unlimited.balance.as_ref().unwrap();
    assert!(b.unlimited && b.limit.is_none());
    assert_eq!(unlimited.level, QuotaLevel::Ok);

    let mut expired = newapi_usage(100, false);
    expired["data"]["expires_at"] = json!(NOW / 1000 - 60);
    let snap = parse(RelayAdapter::Newapi, &expired, DEFAULT_QUOTA_PER_USD, NOW).unwrap();
    assert_eq!(snap.level, QuotaLevel::Out);
}

#[test]
fn unlimited_key_quota_is_not_an_account_balance() {
    let mut body = newapi_usage(-3_332_825_871, true);
    body["data"].as_object_mut().unwrap().remove("user_quota");
    let missing = parse(RelayAdapter::Newapi, &body, DEFAULT_QUOTA_PER_USD, NOW).unwrap();
    assert_eq!(missing.balance.as_ref().unwrap().remaining, None);
    assert_eq!(missing.balance.as_ref().unwrap().account_remaining, None);
    assert_eq!(missing.level, QuotaLevel::Ok);
    assert_eq!(
        super::super::brief(&missing, false).as_deref(),
        Some("余额未提供")
    );
    for (available, level) in [(60_000_000, QuotaLevel::Ok), (0, QuotaLevel::Out)] {
        body["data"]["user_quota"] = json!({"total_available": available});
        let snap = parse(RelayAdapter::Newapi, &body, DEFAULT_QUOTA_PER_USD, NOW).unwrap();
        assert_eq!(
            snap.balance.as_ref().unwrap().account_remaining,
            Some(available as f64 / DEFAULT_QUOTA_PER_USD)
        );
        assert_eq!(snap.level, level);
        assert!(super::super::brief(&snap, false)
            .unwrap()
            .contains(if available == 0 { "$0.00" } else { "$120.00" }));
    }
}

#[test]
fn refresh_detects_unconfigured_relays_and_reuses_detection_after_restart() {
    let (_dir, env) = sandbox();
    let (base, seen) = serve(vec![
        (404, "", "{}".into()),
        (200, "", newapi_usage(15_700_000, false).to_string()),
        (200, "", newapi_usage(10_000_000, false).to_string()),
    ]);
    for id in ["auto", "shared", "off", "official"] {
        let mut card = claude_relay(id, &base, "sk-auto");
        if id == "off" {
            card.meta
                .extra
                .insert(META_QUOTA.into(), json!({"adapter":"off"}));
        }
        if id == "official" {
            card.category = Some("official".into());
        }
        store::add_provider(&env, Tool::Claude, card).unwrap();
    }
    let before = fs::read(env.data_file("providers.json")).unwrap();
    let service = Service::start(env.clone(), None).unwrap();
    let pending = service.quota_overview().unwrap();
    assert_eq!(pending.sources.len(), 1);
    assert_eq!(pending.sources[0].users.len(), 2);
    assert!(pending.sources[0].snapshot.is_none());
    assert!(seen.lock().unwrap().is_empty(), "overview remains offline");
    let refreshed = service.quota_refresh(None).unwrap();
    assert_eq!(refreshed.sources[0].adapter, Some(RelayAdapter::Newapi));
    assert_eq!(
        refreshed.sources[0]
            .snapshot
            .as_ref()
            .unwrap()
            .balance
            .as_ref()
            .unwrap()
            .remaining,
        Some(31.4)
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(fs::read(env.data_file("providers.json")).unwrap(), before);
    let restarted = Service::start(env, None).unwrap();
    let refreshed = restarted.quota_refresh(None).unwrap();
    assert_eq!(
        refreshed.sources[0]
            .snapshot
            .as_ref()
            .unwrap()
            .balance
            .as_ref()
            .unwrap()
            .remaining,
        Some(20.0)
    );
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 3, "cached adapter skips detection");
    assert!(requests[2][0].starts_with("GET /api/usage/token/ "));
}

#[test]
fn other_json_is_not_mistaken_for_either_platform() {
    let samples = [
        json!({"success": false, "message": "无效的令牌"}),
        json!({"error": {"message": "not found", "type": "invalid_request_error"}}),
        json!({"object": "list", "data": [{"id": "gpt-5"}]}),
        json!({"mode": "quota_limited"}),
        json!({"code": false, "data": {"object": "token_usage", "total_available": 1}}),
    ];
    for sample in &samples {
        for adapter in RelayAdapter::ALL {
            assert!(
                parse(adapter, sample, DEFAULT_QUOTA_PER_USD, NOW).is_none(),
                "{adapter:?} {sample}"
            );
        }
    }
}

#[test]
fn settings_round_trip_and_validate() {
    let mut card = Provider::new("relay", "Relay", json!({}));
    assert!(QuotaSettings::of(&card).is_none());
    card.meta.extra.insert(
        META_QUOTA.into(),
        json!({"adapter": "newapi", "refreshMinutes": 15, "quotaPerUsd": 1000000}),
    );
    let settings = QuotaSettings::of(&card).unwrap();
    assert_eq!(settings.adapter(), Some(RelayAdapter::Newapi));
    assert_eq!(settings.per_usd(), 1_000_000.0);
    assert!(settings.validate().is_ok());
    let off = QuotaSettings {
        adapter: Some("off".into()),
        ..Default::default()
    };
    assert_eq!(off.adapter(), None);
    assert!(off.validate().is_ok());
    assert!(QuotaSettings {
        adapter: Some("js".into()),
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(QuotaSettings {
        quota_per_usd: Some(0.0),
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(QuotaSettings {
        refresh_minutes: 5000,
        ..Default::default()
    }
    .validate()
    .is_err());
}

#[test]
fn source_keys_use_the_origin_and_hide_the_key() {
    let a = origin(&Url::parse("https://relay.example.com/v1/chat?x=1").unwrap());
    let b = origin(&Url::parse("https://relay.example.com").unwrap());
    assert_eq!(a.as_str(), "https://relay.example.com/");
    let key = source_key(&a, "sk-secret-123");
    assert_eq!(key, source_key(&b, "sk-secret-123"));
    assert!(key.starts_with("relay:https://relay.example.com:"));
    assert!(!key.contains("sk-secret"));
    assert_ne!(key, source_key(&a, "sk-other"));
}

#[test]
fn detection_skips_html_and_stops_at_the_first_match() {
    let (origin_url, seen) = serve(vec![
        // sub2api 的路径在 NewAPI 站点上回了一张网页。
        (200, "", "<!doctype html><title>NewAPI</title>".into()),
        (200, "", newapi_usage(15_700_000, false).to_string()),
    ]);
    let client = super::super::codex_remote::client().unwrap();
    let base = Url::parse(&format!("{origin_url}/v1")).unwrap();
    let (steps, found) = detect(
        &client,
        &origin(&base),
        "sk-relay",
        DEFAULT_QUOTA_PER_USD,
        NOW,
    );
    let summary: Vec<_> = steps
        .iter()
        .map(|s| (s.adapter, s.outcome, s.status))
        .collect();
    assert_eq!(
        summary,
        [
            (RelayAdapter::Sub2api, StepOutcome::Miss, Some(200)),
            (RelayAdapter::Newapi, StepOutcome::Hit, Some(200)),
        ]
    );
    assert_eq!(found.unwrap().0, RelayAdapter::Newapi);
    let requests = seen.lock().unwrap().clone();
    assert!(requests[0][0].starts_with("GET /v1/usage "));
    assert!(requests[1][0].starts_with("GET /api/usage/token/ "));
    assert_eq!(
        test_server::header(&requests[1], "authorization").as_deref(),
        Some("Bearer sk-relay")
    );
    assert!(steps.iter().all(|s| !s.url.contains("sk-relay")));

    let (origin_url, seen) = serve(vec![(200, "", sub2api_limited().to_string())]);
    let (steps, found) = detect(
        &client,
        &Url::parse(&origin_url).unwrap(),
        "sk-relay",
        DEFAULT_QUOTA_PER_USD,
        NOW,
    );
    assert_eq!(found.unwrap().0, RelayAdapter::Sub2api);
    assert_eq!(steps[1].outcome, StepOutcome::Skipped);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn detection_reports_rejections_and_unreachable_sites() {
    let (origin_url, _) = serve(vec![(401, "", "{}".into()), (404, "", "{}".into())]);
    let client = super::super::codex_remote::client().unwrap();
    let (steps, found) = detect(
        &client,
        &Url::parse(&origin_url).unwrap(),
        "sk-bad",
        DEFAULT_QUOTA_PER_USD,
        NOW,
    );
    assert!(found.is_none());
    let summary: Vec<_> = steps
        .iter()
        .map(|s| (s.outcome, s.status, s.code.as_deref()))
        .collect();
    assert_eq!(
        summary,
        [
            (StepOutcome::Miss, Some(401), Some("unauthorized")),
            (StepOutcome::Miss, Some(404), Some("http")),
        ]
    );

    let unused = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let closed = Url::parse(&format!("http://{}", unused.local_addr().unwrap())).unwrap();
    drop(unused);
    let (steps, _) = detect(&client, &closed, "sk", DEFAULT_QUOTA_PER_USD, NOW);
    assert!(steps.iter().all(|s| s.outcome == StepOutcome::Error));
}

#[test]
fn prefixed_endpoints_are_tried_only_after_a_root_path_miss() {
    let client = super::super::codex_remote::client().unwrap();
    for (adapter, body, path) in [
        (RelayAdapter::Sub2api, sub2api_limited(), "/relay/v1/usage"),
        (
            RelayAdapter::Newapi,
            newapi_usage(15_700_000, false),
            "/relay/api/usage/token/",
        ),
    ] {
        let (base, seen) = serve(vec![(404, "", "{}".into()), (200, "", body.to_string())]);
        let address = Url::parse(&format!("{base}/relay/v1/?ignored=secret#fragment")).unwrap();
        let snap = fetch(
            &client,
            adapter,
            &address,
            "sk-path",
            DEFAULT_QUOTA_PER_USD,
            NOW,
        )
        .unwrap();
        assert_eq!(snap.balance.unwrap().remaining, Some(31.4));
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1][0].starts_with(&format!("GET {path} ")));
        assert_eq!(
            test_server::header(&requests[1], "authorization").as_deref(),
            Some("Bearer sk-path")
        );
    }

    let (base, seen) = serve(vec![(200, "", sub2api_limited().to_string())]);
    let address = Url::parse(&format!("{base}/codex/v1")).unwrap();
    fetch(
        &client,
        RelayAdapter::Sub2api,
        &address,
        "sk",
        DEFAULT_QUOTA_PER_USD,
        NOW,
    )
    .unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "root-hosted quota remains the default"
    );
}

#[test]
fn prefixed_detection_records_the_real_url_and_refresh_uses_the_same_source() {
    let (_dir, env) = sandbox();
    let (base, seen) = serve(vec![
        (200, "", "<html>not an endpoint</html>".into()),
        (200, "", sub2api_limited().to_string()),
        (404, "", "{}".into()),
        (200, "", sub2api_limited().to_string()),
    ]);
    let card = claude_relay("prefix", &format!("{base}/relay/v1"), "sk-prefix");
    store::add_provider(&env, Tool::Claude, card.clone()).unwrap();
    let service = Service::start(env, None).unwrap();
    let detected = service
        .quota_detect(Tool::Claude, &card.settings_config, None)
        .unwrap();
    assert_eq!(detected.adapter, Some(RelayAdapter::Sub2api));
    assert_eq!(detected.steps[1].url, format!("{base}/relay/v1/usage"));
    assert_eq!(detected.steps[1].outcome, StepOutcome::Hit);
    service
        .quota_configure(
            Tool::Claude,
            "prefix",
            Some(QuotaSettings {
                adapter: Some("sub2api".into()),
                ..Default::default()
            }),
        )
        .unwrap();
    let view = service.quota_overview().unwrap();
    assert_eq!(Some(&view.sources[0].key), detected.key.as_ref());
    assert!(view.sources[0].snapshot.is_some());
    let view = service.quota_refresh(None).unwrap();
    assert!(view.sources[0].error.is_none());
    assert_eq!(seen.lock().unwrap().len(), 4);
}

#[test]
fn url_candidates_never_change_authority_or_leak_url_credentials() {
    let address = Url::parse("https://login:secret@gw.example/relay/v1/?token=secret#x").unwrap();
    let urls = endpoints(RelayAdapter::Sub2api, &address);
    assert_eq!(
        urls.iter().map(Url::as_str).collect::<Vec<_>>(),
        [
            "https://gw.example/v1/usage",
            "https://gw.example/relay/v1/usage",
        ]
    );
    assert_ne!(
        source_key(&query_base(&address), "sk"),
        source_key(&origin(&address), "sk")
    );
    assert_eq!(
        query_base(&Url::parse("https://gw.example/relay/v1beta").unwrap()),
        query_base(&address),
    );
}

#[test]
fn rate_limits_and_redirects_do_not_trigger_fallback_or_follow_redirects() {
    let client = super::super::codex_remote::client().unwrap();
    for status in [301, 302, 307, 308, 429, 500] {
        let (base, seen) = serve(vec![(
            status,
            "Location: https://elsewhere.invalid/\r\n",
            "{}".into(),
        )]);
        let address = Url::parse(&format!("{base}/relay/v1")).unwrap();
        let error = fetch(
            &client,
            RelayAdapter::Newapi,
            &address,
            "sk",
            DEFAULT_QUOTA_PER_USD,
            NOW,
        )
        .unwrap_err();
        assert_eq!(error.status, Some(status));
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}

fn sandbox() -> (tempfile::TempDir, Env) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    (dir, Env::with_home(&home))
}

fn claude_relay(id: &str, base: &str, key: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({"env": {"ANTHROPIC_BASE_URL": base, "ANTHROPIC_AUTH_TOKEN": key}}),
    )
}

#[test]
fn service_detects_configures_shares_one_source_and_refreshes() {
    let (_dir, env) = sandbox();
    let (origin_url, seen) = serve(vec![
        (200, "", "<html></html>".into()),
        (200, "", newapi_usage(15_700_000, false).to_string()),
        (200, "", newapi_usage(10_000_000, false).to_string()),
        (500, "", "{}".into()),
    ]);
    store::add_provider(
        &env,
        Tool::Claude,
        claude_relay("corp", &origin_url, "sk-corp"),
    )
    .unwrap();
    store::set_current(&env, Tool::Claude, "corp").unwrap();
    // OpenCode 也接了同一个站点、同一个 Key：只算一个来源。
    store::add_provider(
        &env,
        Tool::OpenCode,
        Provider::new(
            "corp-oc",
            "corp-oc",
            json!({"npm": "@ai-sdk/openai-compatible", "options": {"baseURL": format!("{origin_url}/v1"), "apiKey": "sk-corp"},
                   "models": {"m": {"name": "m"}}}),
        ),
    )
    .unwrap();
    let mut service = Service::start(env, None).unwrap();
    service.set_quota_url("http://127.0.0.1:9/unused");

    let config =
        json!({"env": {"ANTHROPIC_BASE_URL": origin_url, "ANTHROPIC_AUTH_TOKEN": "sk-corp"}});
    let detected = service.quota_detect(Tool::Claude, &config, None).unwrap();
    assert_eq!(detected.adapter, Some(RelayAdapter::Newapi));
    assert_eq!(
        detected
            .snapshot
            .as_ref()
            .unwrap()
            .balance
            .as_ref()
            .unwrap()
            .remaining,
        Some(31.4)
    );
    assert!(!serde_json::to_string(&detected)
        .unwrap()
        .contains("sk-corp"));

    // 识别不改卡片：自动模式直接使用识别缓存。
    assert_eq!(
        service.quota_overview().unwrap().sources[0].adapter,
        Some(RelayAdapter::Newapi)
    );
    let settings = QuotaSettings {
        adapter: Some("newapi".into()),
        refresh_minutes: 15,
        quota_per_usd: None,
        krill_jwt: None,
        account_token: None,
        account_user_id: None,
        query_base_url: None,
    };
    service
        .quota_configure(Tool::Claude, "corp", Some(settings.clone()))
        .unwrap();
    service
        .quota_configure(Tool::OpenCode, "corp-oc", Some(settings))
        .unwrap();
    assert!(service
        .quota_configure(Tool::Claude, "missing", None)
        .is_err());
    assert!(service
        .quota_configure(
            Tool::Claude,
            "corp",
            Some(QuotaSettings {
                adapter: Some("js".into()),
                ..Default::default()
            })
        )
        .is_err());

    let view = service.quota_overview().unwrap();
    assert_eq!(view.sources.len(), 1, "{view:?}");
    let source = &view.sources[0];
    assert_eq!(source.kind, QuotaKind::Relay);
    assert_eq!(source.adapter, Some(RelayAdapter::Newapi));
    assert_eq!(source.label.as_deref(), Some("127.0.0.1"));
    assert_eq!(
        source
            .users
            .iter()
            .map(|u| (u.tool, u.active))
            .collect::<Vec<_>>(),
        [(Tool::Claude, true), (Tool::OpenCode, false)]
    );
    // 识别时查到的结果已经在缓存里。
    assert_eq!(
        source
            .snapshot
            .as_ref()
            .unwrap()
            .balance
            .as_ref()
            .unwrap()
            .remaining,
        Some(31.4)
    );
    assert!(!serde_json::to_string(&view).unwrap().contains("sk-corp"));

    // 刷新：两张卡共用一个来源，只发一次请求。
    let refreshed = service.quota_refresh(None).unwrap();
    assert_eq!(
        refreshed.sources[0]
            .snapshot
            .as_ref()
            .unwrap()
            .balance
            .as_ref()
            .unwrap()
            .remaining,
        Some(20.0)
    );
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert_eq!(refreshed.busiest.as_deref(), Some(source.key.as_str()));

    // 失败时保留旧数据并带上错误。
    service.forget_quota_attempts();
    let failed = service
        .quota_refresh(Some(vec![source.key.clone()]))
        .unwrap();
    let source = &failed.sources[0];
    assert_eq!(
        source
            .snapshot
            .as_ref()
            .unwrap()
            .balance
            .as_ref()
            .unwrap()
            .remaining,
        Some(20.0)
    );
    let error = source.error.as_ref().unwrap();
    assert_eq!((error.code.as_str(), error.status), ("http", Some(500)));

    // 清除设置恢复自动模式，显式关闭才隐藏来源。
    service.quota_configure(Tool::Claude, "corp", None).unwrap();
    service
        .quota_configure(Tool::OpenCode, "corp-oc", None)
        .unwrap();
    assert_eq!(service.quota_overview().unwrap().sources.len(), 1);
    for (tool, id) in [(Tool::Claude, "corp"), (Tool::OpenCode, "corp-oc")] {
        service
            .quota_configure(
                tool,
                id,
                Some(QuotaSettings {
                    adapter: Some("off".into()),
                    ..Default::default()
                }),
            )
            .unwrap();
    }
    assert!(service.quota_overview().unwrap().sources.is_empty());
}

#[test]
fn detection_without_an_address_or_key_says_so() {
    let (_dir, env) = sandbox();
    let service = Service::start(env, None).unwrap();
    let detected = service
        .quota_detect(
            Tool::Claude,
            &json!({"env": {"ANTHROPIC_BASE_URL": "https://relay.example.com"}}),
            None,
        )
        .unwrap();
    assert_eq!(detected.error.as_deref(), Some("noEndpoint"));
    assert!(detected.steps.is_empty());
}

#[test]
fn refresh_shows_fresh_results_even_when_the_cache_cannot_be_written() {
    let (_dir, env) = sandbox();
    let (origin_url, _seen) = serve(vec![(200, "", newapi_usage(15_700_000, false).to_string())]);
    let mut card = claude_relay("corp", &origin_url, "sk-corp");
    card.meta
        .extra
        .insert(META_QUOTA.into(), json!({"adapter": "newapi"}));
    store::add_provider(&env, Tool::Claude, card).unwrap();
    store::set_current(&env, Tool::Claude, "corp").unwrap();
    // 缓存文件的位置被一个目录占着，写不进去。
    fs::create_dir_all(env.data_file("quota-cache.json").join("blocked")).unwrap();
    let service = Service::start(env, None).unwrap();

    let view = service.quota_refresh(None).unwrap();
    let source = view
        .sources
        .iter()
        .find(|s| s.kind == QuotaKind::Relay)
        .unwrap();
    let balance = source.snapshot.as_ref().unwrap().balance.as_ref().unwrap();
    assert_eq!(balance.remaining, Some(31.4));
    assert!(source.error.is_none());
}

#[test]
fn auto_refresh_runs_only_when_a_card_interval_has_passed() {
    let (_dir, env) = sandbox();
    let (origin_url, seen) = serve(vec![
        (200, "", newapi_usage(15_700_000, false).to_string()),
        (500, "", "{}".into()),
    ]);
    let mut card = claude_relay("corp", &origin_url, "sk-corp");
    card.meta.extra.insert(
        META_QUOTA.into(),
        json!({"adapter": "newapi", "refreshMinutes": 15}),
    );
    let mut manual = claude_relay("manual", &format!("{origin_url}/other"), "sk-manual");
    manual.meta.extra.insert(
        META_QUOTA.into(),
        json!({"adapter": "newapi", "refreshMinutes": 0}),
    );
    store::add_provider(&env, Tool::Claude, card).unwrap();
    store::add_provider(&env, Tool::Claude, manual).unwrap();
    let service = Service::start(env.clone(), None).unwrap();

    // 从没查过：到点，只查设了间隔的那张卡。
    assert!(service.quota_refresh_due().unwrap());
    assert_eq!(seen.lock().unwrap().len(), 1);
    // 刚查过：不到点。
    assert!(!service.quota_refresh_due().unwrap());
    assert_eq!(seen.lock().unwrap().len(), 1);

    // 上次结果已经过了 16 分钟：到点；这次失败。
    let key = service
        .quota_overview()
        .unwrap()
        .sources
        .into_iter()
        .find(|s| s.users.iter().any(|u| u.provider_id == "corp"))
        .unwrap()
        .key;
    let mut old = parse(
        RelayAdapter::Newapi,
        &newapi_usage(15_700_000, false),
        DEFAULT_QUOTA_PER_USD,
        super::super::now_ms(),
    )
    .unwrap();
    old.fetched_at -= 16 * 60_000;
    super::super::cache::save(
        &env,
        &[],
        &[super::super::cache::RelayEntry {
            key,
            snapshot: old,
            adapter: None,
        }],
        super::super::now_ms(),
    )
    .unwrap();
    service.forget_quota_attempts();
    assert!(service.quota_refresh_due().unwrap());
    assert_eq!(seen.lock().unwrap().len(), 2);
    // 失败后也要等满一个间隔，不会每分钟重试。
    assert!(!service.quota_refresh_due().unwrap());
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn explicit_query_base_is_used_for_detection_refresh_and_isolated_cache() {
    let (host, seen) = serve(vec![
        (200, "", sub2api_limited().to_string()),
        (200, "", sub2api_limited().to_string()),
    ]);
    let (_dir, env) = sandbox();
    let service = Service::open(env.clone(), None).unwrap();
    let config = json!({"env":{"ANTHROPIC_BASE_URL":format!("{host}/models/v1"),"ANTHROPIC_AUTH_TOKEN":"fixture-model-key"}});
    let base = format!("{host}/billing/v1/"); // An explicit /v1 is part of the prefix, not stripped.
    let found = service
        .quota_detect_with_base(Tool::Claude, &config, None, Some(base.clone()))
        .unwrap();
    assert_eq!(found.adapter, Some(RelayAdapter::Sub2api));
    let mut card = Provider::new("explicit", "Explicit", config);
    card.meta.extra.insert(
        META_QUOTA.into(),
        json!({"adapter":"sub2api","refreshMinutes":0,"queryBaseUrl":base}),
    );
    service
        .add_provider(Tool::Claude, card.clone(), false)
        .unwrap();
    let cards = relay_cards(&[(Tool::Claude, &card)], &|_, _| false);
    assert_eq!(Some(cards[0].key.clone()), found.key);
    let default_key = source_key(
        &query_base(&Url::parse(&format!("{host}/models/v1")).unwrap()),
        "fixture-model-key",
    );
    assert_ne!(cards[0].key, default_key);
    let view = service.quota_refresh(None).unwrap();
    assert!(view.sources[0].error.is_none());
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| request[0].starts_with("GET /billing/v1/v1/usage ")));
}

#[test]
fn explicit_query_base_rejects_other_origins_and_credential_bearing_urls() {
    let model = Url::parse("https://example.com/models/v1").unwrap();
    for base in [
        "https://other.example/billing",
        "http://example.com/billing",
        "https://user:secret@example.com/",
        "https://example.com/?token=secret",
        "https://example.com/#secret",
        "file:///tmp/quota",
    ] {
        let settings = QuotaSettings {
            query_base_url: Some(base.into()),
            ..Default::default()
        };
        assert!(settings.query_address(&model).is_err(), "{base}");
    }
    let settings = QuotaSettings {
        query_base_url: Some("https://example.com/billing/".into()),
        ..Default::default()
    };
    let explicit = settings.query_address(&model).unwrap();
    assert_eq!(
        endpoints(RelayAdapter::Newapi, &explicit)[0].as_str(),
        "https://example.com/billing/api/usage/token/"
    );
    assert_ne!(
        explicit.cache_key("fixture"),
        QueryAddress::Automatic(Url::parse("https://example.com/billing/").unwrap())
            .cache_key("fixture")
    );
}
