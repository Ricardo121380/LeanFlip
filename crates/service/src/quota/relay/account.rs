//! 用户账户查询：只用用户访问令牌访问同站账户接口，不向模型接口发送它。
use super::*;

pub(crate) fn address(tool: Tool, provider: &Provider) -> Option<Url> {
    crate::service::connection_info(tool, provider)
        .base_url
        .and_then(|url| Url::parse(&url).ok())
        .or_else(|| {
            crate::probe::address_and_key(tool, &provider.settings_config).map(|(url, _)| url)
        })
}

fn path(adapter: RelayAdapter) -> Option<&'static str> {
    match adapter {
        RelayAdapter::Newapi => Some("/api/user/self"),
        RelayAdapter::Sub2api => Some("/api/v1/user/profile"),
        RelayAdapter::Krill => None,
    }
}

pub(crate) fn key(address: &Url, token: &str, user_id: Option<u64>) -> String {
    source_key(
        &query_base(address),
        &serde_json::to_string(&("account", token, user_id)).expect("string tuple"),
    )
}

pub(crate) fn parse(
    adapter: RelayAdapter,
    value: &Value,
    per_usd: f64,
    now: i64,
) -> Option<QuotaSnapshot> {
    let data = &value["data"];
    let remaining = match adapter {
        RelayAdapter::Newapi if value["success"] == true => number(&data["quota"])? / per_usd,
        RelayAdapter::Sub2api if value["code"] == 0 => number(&data["balance"])?,
        _ => return None,
    };
    Some(
        QuotaSnapshot::new(
            Vec::new(),
            None,
            None,
            now,
            QuotaOrigin::Network,
            // 钱包为零不代表订阅套餐不可用，账户接口未提供限额结论。
            false,
        )
        .with_balance(Some(QuotaBalance {
            unit: "USD".into(),
            remaining: None,
            used: None,
            limit: None,
            unlimited: false,
            expires_at: None,
            account_remaining: Some(remaining),
        })),
    )
}

fn fetch_at(
    client: &Client,
    adapter: RelayAdapter,
    url: Url,
    token: &str,
    user_id: Option<u64>,
    per_usd: f64,
    now: i64,
) -> Result<QuotaSnapshot, Failure> {
    let (status, value) = get_with_user(
        client,
        url,
        token,
        if adapter == RelayAdapter::Newapi {
            user_id
        } else {
            None
        },
    )?;
    if value["success"] == false || matches!(value["code"].as_u64(), Some(401 | 403)) {
        return Err(Failure {
            code: "unauthorized",
            status: Some(status),
        });
    }
    parse(adapter, &value, per_usd, now).ok_or(Failure {
        code: "format",
        status: Some(status),
    })
}

pub(crate) fn fetch(
    client: &Client,
    adapter: RelayAdapter,
    address: &Url,
    token: &str,
    user_id: Option<u64>,
    per_usd: f64,
    now: i64,
) -> Result<QuotaSnapshot, Failure> {
    let path = path(adapter).ok_or(Failure {
        code: "format",
        status: None,
    })?;
    let urls = path_endpoints(address, path);
    let mut result = fetch_at(
        client,
        adapter,
        urls[0].clone(),
        token,
        user_id,
        per_usd,
        now,
    );
    if result.as_ref().is_err_and(try_next_endpoint) && urls.len() > 1 {
        result = fetch_at(
            client,
            adapter,
            urls[1].clone(),
            token,
            user_id,
            per_usd,
            now,
        );
    }
    result
}

pub(crate) fn detect(
    client: &Client,
    address: &Url,
    token: &str,
    user_id: Option<u64>,
    per_usd: f64,
    now: i64,
) -> (Vec<DetectStep>, Option<(RelayAdapter, QuotaSnapshot)>) {
    let mut steps = Vec::new();
    for adapter in [RelayAdapter::Newapi, RelayAdapter::Sub2api] {
        for url in path_endpoints(address, path(adapter).expect("account adapter")) {
            let result = fetch_at(client, adapter, url.clone(), token, user_id, per_usd, now);
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
            steps.push(DetectStep {
                adapter,
                url: url.to_string(),
                outcome,
                status,
                code,
            });
            match result {
                Ok(snapshot) => return (steps, Some((adapter, snapshot))),
                Err(f) if try_next_endpoint(&f) => {}
                Err(_) => break,
            }
        }
    }
    (steps, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{quota::test_server, Service};
    use leanflip_core::{store, Env, Provider, Tool};
    use serde_json::json;

    #[test]
    fn account_balances_keep_units_and_reject_error_or_unrelated_payloads() {
        let newapi = json!({"success":true,"data":{"id":7,"quota":60_000_000,"used_quota":1000}});
        let sub2api = json!({"code":0,"data":{"id":7,"balance":12.34}});
        assert_eq!(
            parse(RelayAdapter::Newapi, &newapi, 500_000.0, 1)
                .unwrap()
                .balance
                .unwrap()
                .account_remaining,
            Some(120.0)
        );
        assert_eq!(
            parse(RelayAdapter::Sub2api, &sub2api, 500_000.0, 1)
                .unwrap()
                .balance
                .unwrap()
                .account_remaining,
            Some(12.34)
        );
        assert!(parse(
            RelayAdapter::Newapi,
            &json!({"success":false,"data":{"quota":100}}),
            1.0,
            1
        )
        .is_none());
        assert!(parse(
            RelayAdapter::Sub2api,
            &json!({"code":401,"data":{"balance":100}}),
            1.0,
            1
        )
        .is_none());
        let empty = parse(
            RelayAdapter::Sub2api,
            &json!({"code":0,"data":{"balance":0}}),
            1.0,
            1,
        )
        .unwrap();
        assert_eq!(empty.level, super::super::super::QuotaLevel::Ok);
        assert_eq!(
            super::super::super::brief(&empty, true).as_deref(),
            Some("$0.00")
        );
    }

    #[test]
    fn refresh_uses_account_credentials_only_and_saves_them_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        let (address, seen) = test_server::serve(vec![(
            200,
            "",
            json!({"success":true,"data":{"quota":25_000_000}}).to_string(),
        )]);
        let config = json!({"env":{"ANTHROPIC_BASE_URL":format!("{address}/v1"),"ANTHROPIC_AUTH_TOKEN":"sk-model-only"}});
        let mut card = Provider::new("r", "Relay", config.clone());
        card.meta.extra.insert(
            META_QUOTA.into(),
            json!({"accountToken":"personal-only","accountUserId":7}),
        );
        let service = Service::start(env.clone(), None).unwrap();
        service
            .add_provider(Tool::Claude, card.clone(), false)
            .unwrap();
        let mut account_only = card;
        account_only.id = "no-model-key".into();
        account_only.settings_config["env"]
            .as_object_mut()
            .unwrap()
            .remove("ANTHROPIC_AUTH_TOKEN");
        service
            .add_provider(Tool::Claude, account_only, false)
            .unwrap();
        let view = service.quota_refresh(None).unwrap();
        assert_eq!(
            view.sources[0].users.len(),
            2,
            "account queries do not depend on model credentials"
        );
        assert_eq!(view.sources[0].adapter, Some(RelayAdapter::Newapi));
        assert_eq!(
            view.sources[0]
                .snapshot
                .as_ref()
                .unwrap()
                .balance
                .as_ref()
                .unwrap()
                .account_remaining,
            Some(50.0)
        );
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0][0].starts_with("GET /api/user/self "));
        assert_eq!(
            test_server::header(&requests[0], "authorization").as_deref(),
            Some("Bearer personal-only")
        );
        assert_eq!(
            test_server::header(&requests[0], "new-api-user").as_deref(),
            Some("7")
        );
        let mut visible = service
            .list_providers(Tool::Claude)
            .unwrap()
            .providers
            .into_iter()
            .find(|p| p.id == "r")
            .unwrap();
        let wire = serde_json::to_string(&visible).unwrap();
        assert!(!wire.contains("personal-only"));
        assert_eq!(
            visible.meta.extra[META_QUOTA]["accountToken"],
            crate::redact::PLACEHOLDER
        );
        service
            .update_provider(Tool::Claude, visible.clone())
            .unwrap();
        assert_eq!(
            store::load_readonly(&env)
                .unwrap()
                .provider(Tool::Claude, "r")
                .unwrap()
                .settings_config,
            config
        );
        visible.settings_config["env"]["ANTHROPIC_BASE_URL"] = json!("https://other.example/v1");
        assert!(service
            .update_provider(Tool::Claude, visible.clone())
            .is_err());
        let settings = QuotaSettings::of(&visible).unwrap();
        assert!(service
            .quota_account_detect(Tool::Claude, &visible.settings_config, Some("r"), settings)
            .is_err());
    }

    #[test]
    fn detection_supports_sub2api_and_never_follows_redirects() {
        let client = super::super::super::codex_remote::client().unwrap();
        let (address, seen) = test_server::serve(vec![
            (404, "", "{}".into()),
            (
                200,
                "",
                json!({"code":0,"data":{"balance":8.5}}).to_string(),
            ),
        ]);
        let (_, found) = detect(
            &client,
            &address.parse().unwrap(),
            "user-access",
            Some(9),
            500_000.0,
            1,
        );
        assert_eq!(found.unwrap().0, RelayAdapter::Sub2api);
        let requests = seen.lock().unwrap();
        assert!(requests[1][0].starts_with("GET /api/v1/user/profile "));
        assert_eq!(test_server::header(&requests[1], "new-api-user"), None);
        let (address, seen) = test_server::serve(vec![(
            302,
            "Location: https://other.example/\r\n",
            "{}".into(),
        )]);
        assert!(fetch(
            &client,
            RelayAdapter::Newapi,
            &address.parse().unwrap(),
            "user-access",
            None,
            500_000.0,
            1
        )
        .is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
