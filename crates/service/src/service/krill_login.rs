//! 官网登录后的 JWT 只留在后端；编辑器用短期随机引用，保存卡片时才落盘。
use std::time::{Duration, Instant};

use serde::Serialize;
use zeroize::Zeroizing;

use super::*;

const PREFIX: &str = "__leanflip_krill_login_";
const LIFETIME: Duration = Duration::from_secs(30 * 60);

pub(super) struct PendingLogin {
    jwt: Zeroizing<String>,
    created: Instant,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KrillLogin {
    /// 临时引用，不是 JWT；只在当前 LeanFlip 进程内可用。
    pub credential: String,
    pub detected: quota::relay::QuotaDetect,
}

impl Service {
    pub fn quota_krill_login_complete(&self, tool: Tool, jwt: String) -> ApiResult<KrillLogin> {
        let jwt = Zeroizing::new(jwt);
        let detected = self.quota_krill_detect(tool, None, Some(&jwt))?;
        if detected.adapter != Some(quota::relay::RelayAdapter::Krill) {
            return Err(ApiError::message(if self.english() {
                "Krill sign-in could not be verified. Please sign in again.".into()
            } else {
                "Krill 登录状态验证失败，请重新登录。".into()
            }));
        }
        Ok(KrillLogin {
            credential: self.stage_krill_login(jwt)?,
            detected,
        })
    }

    fn stage_krill_login(&self, jwt: Zeroizing<String>) -> ApiResult<String> {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| ApiError::message("Could not create login reference".into()))?;
        let id = format!(
            "{PREFIX}{}",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let mut pending = self.krill_logins.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|_, login| login.created.elapsed() < LIFETIME);
        pending.insert(
            id.clone(),
            PendingLogin {
                jwt,
                created: Instant::now(),
            },
        );
        Ok(id)
    }

    pub(super) fn krill_credential(&self, value: &str) -> ApiResult<Zeroizing<String>> {
        if !value.starts_with(PREFIX) {
            return Ok(Zeroizing::new(value.to_string()));
        }
        let mut pending = self.krill_logins.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|_, login| login.created.elapsed() < LIFETIME);
        pending
            .get(value)
            .map(|login| login.jwt.clone())
            .ok_or_else(|| {
                ApiError::message(if self.english() {
                    "Krill sign-in expired before saving. Please sign in again.".into()
                } else {
                    "Krill 登录尚未保存且已过期，请重新登录后保存卡片。".into()
                })
            })
    }

    pub(super) fn resolve_krill_login(&self, provider: &mut Provider) -> ApiResult<()> {
        if let Some(value) = provider
            .meta
            .extra
            .get_mut(quota::relay::META_QUOTA)
            .and_then(|q| q.get_mut("krillJwt"))
        {
            if let Some(jwt) = value.as_str().filter(|jwt| jwt.starts_with(PREFIX)) {
                *value = serde_json::Value::String(self.krill_credential(jwt)?.to_string());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn login_references_resolve_only_in_the_originating_process_and_save_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::with_home(dir.path());
        let service = Service::start(env.clone(), None).unwrap();
        let handle = service
            .stage_krill_login(Zeroizing::new("test-private-login".into()))
            .unwrap();
        assert!(!handle.contains("test-private-login"));
        let mut provider = Provider::new(
            "krill",
            "Krill",
            json!({"env":{"ANTHROPIC_AUTH_TOKEN":"sk-model"}}),
        );
        provider.meta.extra.insert(
            quota::relay::META_QUOTA.into(),
            json!({"adapter":"krill", "krillJwt":handle}),
        );
        service
            .add_provider(Tool::Claude, provider.clone(), false)
            .unwrap();
        let stored = store::load_readonly(&env).unwrap();
        assert_eq!(
            stored.provider(Tool::Claude, "krill").unwrap().meta.extra[quota::relay::META_QUOTA]
                ["krillJwt"],
            "test-private-login"
        );
        let wire = serde_json::to_string(&service.list_providers(Tool::Claude).unwrap()).unwrap();
        assert!(!wire.contains("test-private-login") && !wire.contains(&handle));
        assert!(wire.contains(redact::PLACEHOLDER));
        service
            .link_provider(Tool::Claude, provider.clone(), "new-krill-sync".into())
            .unwrap();
        assert_eq!(
            store::load_readonly(&env)
                .unwrap()
                .provider(Tool::Claude, "krill")
                .unwrap()
                .meta
                .extra[quota::relay::META_QUOTA]["krillJwt"],
            "test-private-login"
        );
        service.update_provider(Tool::Claude, provider).unwrap();
        let restarted = Service::start(env, None).unwrap();
        assert!(restarted.krill_credential(&handle).is_err());
        service
            .krill_logins
            .lock()
            .unwrap()
            .get_mut(&handle)
            .unwrap()
            .created = Instant::now() - LIFETIME;
        assert!(service.krill_credential(&handle).is_err());
    }
}
