use super::*;
use crate::quota::test_server::{header, serve};
use crate::quota::QuotaLevel;
use crate::{redact, Service};
use leanflip_core::{store, Env};
use serde_json::json;

const NOW: i64 = 1_792_036_800_000;

fn response() -> Value {
    json!({"success": true, "data": {
        "credit_balance_usd": "0.00",
        "subscriptions": [
            {"subscription_id": 1, "status": "active", "subscription_end_at": "2026-11-01T00:00:00Z",
             "plan": {"name": "Codex Daily", "billing_type": "usd_daily", "duration_days": 30},
             "quota": {"used_usd": "60", "daily_limit_usd": "100"},
             "total_used_usd": "80", "total_limit_usd": "200", "total_remaining_usd": "120"},
            {"subscription_id": 2, "status": "active", "subscription_end_at": "2026-11-01T00:00:00Z",
             "plan": {"name": "Claude Weekly", "billing_type": "usd_weekly"},
             "quota": {"used_usd": "20", "daily_limit_usd": "100", "window_reset_at": "2026-10-17T04:00:00Z"}},
            {"subscription_id": 3, "status": "frozen", "plan": {"name": "Frozen", "billing_type": "usd_daily"},
             "quota": {"used_usd": "100", "daily_limit_usd": "100"}},
            {"subscription_id": 4, "status": "active", "subscription_end_at": "2026-10-01T00:00:00Z",
             "plan": {"name": "Expired", "billing_type": "usd_daily"},
             "quota": {"used_usd": "100", "daily_limit_usd": "100"}}
        ]
    }})
}

#[test]
fn reads_active_plans_rollover_and_wallet_without_marking_zero_wallet_exhausted() {
    let snap = parse(&response(), NOW).unwrap();
    assert_eq!(snap.level, QuotaLevel::Ok);
    assert_eq!(snap.plan.as_deref(), Some("Codex Daily / Claude Weekly"));
    assert_eq!(snap.windows.len(), 2);
    let daily = snap
        .windows
        .iter()
        .find(|w| w.label.as_deref() == Some("Codex Daily"))
        .unwrap();
    assert_eq!(
        (daily.used_amount, daily.limit_amount, daily.used_percent),
        (Some(80.0), Some(200.0), 40.0)
    );
    assert_eq!(
        daily.resets_at, None,
        "do not invent a daily reset for rollover quota"
    );
    let weekly = &snap.windows[0];
    assert_eq!(weekly.minutes, 10080);
    assert_eq!(weekly.resets_at, Some(NOW / 1000 + 2 * 86400));
    assert_eq!(snap.balance.as_ref().unwrap().remaining, None);
    assert_eq!(snap.balance.as_ref().unwrap().account_remaining, Some(0.0));
}

#[test]
fn wallet_only_and_request_plans_keep_their_units() {
    let wallet = json!({"data":{"subscriptions":[],"credit_balance_usd":"12.50"}});
    let snap = parse(&wallet, NOW).unwrap();
    assert_eq!(snap.balance.unwrap().remaining, Some(12.5));
    let count = json!({"data":{
        "credit_balance_usd":"0",
        "subscriptions":[
            {"status":"active","plan":{"name":"Requests A","billing_type":"request_count"}},
            {"status":"active","plan":{"name":"Requests B","billing_type":"request_count"}}
        ],
        "request_count_quota":{"used_5h":85,"limit_5h":100,"used_weekly":200,"limit_weekly":1000,"used_monthly":300,"limit_monthly":5000}
    }});
    let snap = parse(&count, NOW).unwrap();
    assert_eq!(snap.level, QuotaLevel::Warn);
    assert_eq!(
        snap.windows.len(),
        3,
        "shared windows are not repeated for each plan"
    );
    assert!(snap
        .windows
        .iter()
        .all(|w| w.unit.as_deref() == Some("requests")));
    assert_eq!(snap.windows[0].used_amount, Some(85.0));
    assert_eq!(snap.windows[0].used_percent, 85.0);
}

#[test]
fn rejects_error_envelopes_and_missing_data_instead_of_inventing_a_balance() {
    for body in [
        json!({"success": false, "data": {"subscriptions": [], "credit_balance_usd": 100}}),
        json!({"data": {"subscriptions": []}}),
        json!({"data": {"subscriptions": [], "credit_balance_usd": "NaN"}}),
        json!({"data": {"credit_balance_usd": 10}}),
        json!({"message": "login required"}),
    ] {
        assert!(parse(&body, NOW).is_none());
    }
    let mut body = response();
    body["data"]["subscriptions"][0]["total_used_usd"] = json!("200");
    assert_eq!(parse(&body, NOW).unwrap().level, QuotaLevel::Out);
}

#[test]
fn quota_requests_use_bearer_jwt_and_report_expired_login_without_response_secrets() {
    let (base, seen) = serve(vec![
        (200, "", response().to_string()),
        (401, "", r#"{"message":"private-server-detail"}"#.into()),
        (302, "Location: https://other.invalid/\r\n", "{}".into()),
    ]);
    let url = Url::parse(&format!("{base}/api/subscription")).unwrap();
    let client = crate::quota::codex_remote::client().unwrap();
    let request = || {
        fetch_at(
            &client,
            RelayAdapter::Krill,
            url.clone(),
            "test-login-jwt",
            DEFAULT_QUOTA_PER_USD,
            NOW,
        )
    };
    assert!(request().is_ok());
    let failure = request().unwrap_err();
    assert_eq!((failure.code, failure.status), ("krillAuth", Some(401)));
    assert!(!format!("{failure:?}").contains("private-server-detail"));
    assert_eq!(request().unwrap_err().status, Some(302));
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0][0].starts_with("GET /api/subscription "));
    assert_eq!(
        header(&requests[0], "authorization").as_deref(),
        Some("Bearer test-login-jwt")
    );
    assert_eq!(
        endpoints(
            RelayAdapter::Krill,
            &Url::parse("https://unrelated.invalid/codex/v1").unwrap()
        )[0]
        .as_str(),
        KRILL_URL
    );
    assert!(
        !RelayAdapter::ALL.contains(&RelayAdapter::Krill),
        "automatic detection must never send model API keys to Krill"
    );
}

#[test]
fn settings_preserve_hidden_jwt_without_changing_model_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    let service = Service::start(env.clone(), None).unwrap();
    let config = json!({"env":{"ANTHROPIC_BASE_URL":"https://api.krill-code.net/codex/v1","ANTHROPIC_AUTH_TOKEN":"model-key"}});
    let card = Provider::new("krill", "Krill", config.clone());
    store::add_provider(&env, Tool::Claude, card).unwrap();
    let settings = QuotaSettings {
        adapter: Some("krill".into()),
        krill_jwt: Some("test-login-jwt".into()),
        refresh_minutes: 15,
        ..Default::default()
    };
    service
        .quota_configure(Tool::Claude, "krill", Some(settings))
        .unwrap();
    let view = service.list_providers(Tool::Claude).unwrap();
    assert!(!serde_json::to_string(&view)
        .unwrap()
        .contains("test-login-jwt"));
    let mut incoming = view.providers[0].clone();
    assert_eq!(
        incoming.meta.extra[META_QUOTA]["krillJwt"],
        redact::PLACEHOLDER
    );
    incoming.name = "Renamed".into();
    service
        .update_provider(Tool::Claude, incoming.clone())
        .unwrap();
    let settings = QuotaSettings::of(&incoming).unwrap();
    service
        .quota_configure(Tool::Claude, "krill", Some(settings))
        .unwrap();
    let data = store::load_readonly(&env).unwrap();
    let stored = data.provider(Tool::Claude, "krill").unwrap();
    assert_eq!(stored.settings_config, config);
    assert_eq!(
        QuotaSettings::of(stored).unwrap().krill_jwt.as_deref(),
        Some("test-login-jwt")
    );
    let cards = relay_cards(&[(Tool::Claude, stored)], &|_, _| true);
    assert_eq!(cards[0].secret, "test-login-jwt");
    assert_eq!(cards[0].address.as_str(), KRILL_URL);
    assert!(!cards[0].key.contains("test-login-jwt"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(env.store_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let preview = service
        .quota_krill_detect(Tool::Claude, Some("krill"), None)
        .unwrap();
    assert_eq!(preview.error.as_deref(), Some("noKrillJwt"));
    let preview = service
        .quota_krill_detect(Tool::Claude, Some("missing"), Some(redact::PLACEHOLDER))
        .unwrap();
    assert_eq!(preview.error.as_deref(), Some("noKrillJwt"));
    incoming.id = "copied".into();
    assert!(service.add_provider(Tool::Claude, incoming, false).is_err());
    assert!(service
        .quota_configure(
            Tool::Claude,
            "krill",
            Some(QuotaSettings {
                adapter: Some("krill".into()),
                ..Default::default()
            })
        )
        .is_err());
    service
        .quota_configure(Tool::Claude, "krill", None)
        .unwrap();
    assert!(!std::fs::read_to_string(env.store_path())
        .unwrap()
        .contains("test-login-jwt"));
}

#[test]
fn same_krill_login_is_queried_once_across_model_endpoints() {
    let mut a = Provider::new("a", "A", json!({}));
    a.meta.extra.insert(
        META_QUOTA.into(),
        json!({"adapter":"krill","krillJwt":"test-login-jwt","refreshMinutes":15}),
    );
    let mut b = a.clone();
    b.id = "b".into();
    b.settings_config = json!({"env":{"ANTHROPIC_BASE_URL":"https://different.invalid/v1","ANTHROPIC_AUTH_TOKEN":"other-model-key"}});
    let cards = relay_cards(&[(Tool::Claude, &a), (Tool::Codex, &b)], &|_, _| false);
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0].key, cards[1].key);
    let sources = relay_sources(&cards, &HashMap::new(), &HashMap::new());
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].users.len(), 2);
    assert!(!serde_json::to_string(&sources)
        .unwrap()
        .contains("test-login-jwt"));
}
