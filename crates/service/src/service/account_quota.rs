use super::*;
use crate::quota::relay::{self, QuotaDetect, QuotaSettings};

fn base(tool: Tool, provider: &Provider) -> Option<reqwest::Url> {
    relay::account::address(tool, provider).map(|url| relay::query_base(&url))
}

impl Service {
    pub(super) fn check_account_origin(
        &self,
        tool: Tool,
        incoming: &Provider,
        stored: Option<&Provider>,
    ) -> ApiResult<()> {
        let hidden = QuotaSettings::of(incoming)
            .and_then(|q| q.account_token)
            .as_deref()
            == Some(redact::PLACEHOLDER);
        if hidden && base(tool, incoming) != stored.and_then(|p| base(tool, p)) {
            return Err(ApiError::message("站点地址已改变，请重新填写用户访问令牌 / Site changed; re-enter the user access token".into()));
        }
        Ok(())
    }

    pub fn quota_account_detect(
        &self,
        tool: Tool,
        settings_config: &serde_json::Value,
        provider_id: Option<&str>,
        mut settings: QuotaSettings,
    ) -> ApiResult<QuotaDetect> {
        settings.validate().map_err(ApiError::message)?;
        let mut draft = Provider::new(provider_id.unwrap_or("draft"), "", settings_config.clone());
        draft.meta.extra.insert(
            relay::META_QUOTA.into(),
            serde_json::to_value(&settings)
                .map_err(|_| ApiError::message("Invalid quota settings".into()))?,
        );
        if settings.account_token.as_deref() == Some(redact::PLACEHOLDER) {
            let data = store::load_readonly(&self.env()).map_err(|e| self.err(e))?;
            let stored = provider_id.and_then(|id| data.provider(tool, id));
            self.check_account_origin(tool, &draft, stored)?;
            redact::restore(tool, &mut draft, stored);
            settings = QuotaSettings::of(&draft).unwrap_or_default();
        }
        let address = base(tool, &draft);
        let token = settings
            .account_token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty() && *t != redact::PLACEHOLDER);
        let (Some(address), Some(token)) = (address, token) else {
            return Ok(QuotaDetect {
                steps: vec![],
                adapter: None,
                snapshot: None,
                key: None,
                error: Some("noAccountToken".into()),
            });
        };
        let client =
            quota::codex_remote::client().map_err(|_| ApiError::message("network".into()))?;
        let (steps, found) = relay::account::detect(
            &client,
            &address,
            token,
            settings.account_user_id,
            settings.per_usd(),
            quota::now_ms(),
        );
        let key = relay::account::key(&address, token, settings.account_user_id);
        if let Some((adapter, snapshot)) = &found {
            let entry = quota::cache::RelayEntry {
                key: key.clone(),
                snapshot: snapshot.clone(),
                adapter: Some(*adapter),
            };
            if let Err(err) = quota::cache::save(&self.env(), &[], &[entry], quota::now_ms()) {
                log::warn!("quota: cache not saved: {err}");
            }
            self.quota_outcome(key.clone(), Ok(()));
        }
        Ok(QuotaDetect {
            steps,
            adapter: found.as_ref().map(|(a, _)| *a),
            snapshot: found.map(|(_, s)| s),
            key: Some(key),
            error: None,
        })
    }
}
