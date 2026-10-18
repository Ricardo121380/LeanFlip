//! A local login vault and a pure transition planner for Codex authentication.
use super::auth::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
pub const STASH_FILENAME: &str = "codex-login-stash.json";
pub const STASH_VERSION: u32 = 1;
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LoginStash {
    #[serde(default)]
    pub logins: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
    #[serde(default)]
    pub version: u32,
    #[serde(skip)]
    pub initialized: bool,
}
pub fn identity(auth: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(user) = extract_codex_auth_user_identity(auth) {
        parts.push(user);
    }
    if let Some(workspace) = auth
        .pointer("/tokens/account_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        parts.push(format!("account:{workspace}"));
    }
    if parts.is_empty() {
        if extract_codex_auth_api_key(auth).is_some() {
            "api-key".into()
        } else {
            "unknown".into()
        }
    } else {
        parts.join("|")
    }
}
pub(crate) fn login_id(auth: &Value) -> Option<String> {
    codex_auth_has_credential_login_material(auth).then(|| identity(auth))
}
fn static_key(auth: &Value) -> bool {
    codex_live_auth_is_stale_third_party_residue(auth)
}
impl LoginStash {
    pub fn seeded_from_rows<'a>(rows: impl IntoIterator<Item = &'a Value>) -> Self {
        let mut vault = Self {
            version: STASH_VERSION,
            ..Default::default()
        };
        for row in rows {
            if let Some(id) = login_id(row) {
                vault.logins.entry(id).or_insert_with(|| row.clone());
            }
        }
        vault
    }
    pub fn loaded<'a>(
        mut self,
        rows: impl IntoIterator<Item = &'a Value>,
        live: Option<&Value>,
    ) -> Self {
        if self.version >= STASH_VERSION {
            self.initialized = true;
            return self;
        }
        let last = self
            .last
            .as_ref()
            .and_then(|id| self.logins.get(id))
            .map(identity);
        self.logins = self
            .logins
            .into_values()
            .map(|auth| (identity(&auth), auth))
            .collect();
        let users: BTreeSet<_> = self
            .logins
            .values()
            .chain(live.filter(|v| codex_auth_has_credential_login_material(v)))
            .filter_map(extract_codex_auth_user_identity)
            .collect();
        for row in rows {
            if let Some(id) = login_id(row) {
                let allowed =
                    extract_codex_auth_user_identity(row).is_some_and(|u| users.contains(&u));
                if allowed && live.map(identity).as_deref() != Some(&id) {
                    self.logins.entry(id).or_insert_with(|| row.clone());
                }
            }
        }
        self.last = last;
        self.version = STASH_VERSION;
        self.initialized = false;
        self
    }
    pub fn put(&mut self, auth: &Value) {
        let id = identity(auth);
        self.logins.insert(id.clone(), auth.clone());
        self.last = Some(id);
    }
    fn remove(&mut self, id: &str) -> Option<Value> {
        if self.last.as_deref() == Some(id) {
            self.last = None;
        }
        self.logins.remove(id)
    }
}
#[derive(Debug, Clone, Copy)]
pub enum AuthTarget<'a> {
    ThirdParty {
        preserve: bool,
    },
    Official {
        row_auth: &'a Value,
        login_ref: Option<&'a str>,
    },
    Managed {
        auth: &'a Value,
    },
}
pub struct AuthInput<'a> {
    pub live: Option<&'a Value>,
    pub live_is_managed: bool,
    pub third_party_keys: &'a [String],
    pub leaving_official: Option<&'a Value>,
    pub leaving_official_ref: Option<&'a str>,
    pub official_api_keys: &'a [String],
    pub target: AuthTarget<'a>,
    pub stash: LoginStash,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AuthPlan {
    pub auth: Option<Option<Value>>,
    pub stash: Option<LoginStash>,
    pub login_on_disk: bool,
    pub missing_login: Option<String>,
}
pub fn plan(input: AuthInput<'_>) -> AuthPlan {
    let known_key = input
        .live
        .filter(|a| static_key(a))
        .and_then(extract_codex_auth_api_key);
    let foreign = known_key
        .as_ref()
        .is_some_and(|k| input.third_party_keys.contains(k));
    let from_card = known_key
        .as_ref()
        .is_some_and(|k| input.official_api_keys.contains(k));
    let owned = input.live_is_managed || foreign || from_card;
    let native = input
        .live
        .filter(|auth| !owned && codex_auth_has_login_material(auth));
    let mut vault = input.stash.clone();
    if let Some(leaving) = input.leaving_official {
        if native.is_none() && !input.live_is_managed && !from_card && !static_key(leaving) {
            let id = input
                .leaving_official_ref
                .map(str::to_string)
                .or_else(|| login_id(leaving));
            if let Some(id) = id {
                vault.remove(&id);
            }
        }
    }
    let mut next = None;
    let mut missing = None;
    match input.target {
        AuthTarget::ThirdParty { preserve } => {
            if !preserve {
                if let Some(auth) = native {
                    vault.put(auth);
                }
            }
            if input.live_is_managed || foreign || (!preserve && input.live.is_some()) {
                next = Some(None);
            }
        }
        AuthTarget::Managed { auth } => {
            if let Some(auth) = native {
                vault.put(auth);
            }
            next = Some(Some(auth.clone()));
        }
        AuthTarget::Official {
            row_auth,
            login_ref,
        } => {
            if static_key(row_auth) {
                if let Some(auth) = native.filter(|a| *a != row_auth) {
                    vault.put(auth);
                }
                next = Some(Some(row_auth.clone()));
            } else {
                let requested = login_ref.map(str::to_string).or_else(|| login_id(row_auth));
                let already = native
                    .is_some_and(|auth| requested.as_ref().is_none_or(|id| *id == identity(auth)));
                if !already {
                    let selected = requested.clone().or_else(|| {
                        if native.is_none() {
                            vault.last.clone()
                        } else {
                            None
                        }
                    });
                    let saved = selected.as_deref().and_then(|id| vault.remove(id));
                    if let Some(saved) = saved {
                        if let Some(auth) = native {
                            vault.put(auth);
                        }
                        next = Some(Some(saved));
                    } else {
                        if native.is_none() && owned {
                            next = Some(None);
                        }
                        if login_ref.is_some() {
                            missing = requested;
                        }
                    }
                }
            }
        }
    }
    let resulting = match &next {
        None => native,
        Some(value) => value.as_ref(),
    };
    let login_on_disk = resulting.is_some_and(codex_auth_has_openai_account_material);
    let changed = !input.stash.initialized || input.stash != vault;
    vault.initialized = true;
    AuthPlan {
        auth: next,
        stash: changed.then_some(vault),
        login_on_disk,
        missing_login: missing,
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginSummary {
    pub identity: String,
    pub email: Option<String>,
    pub workspace: Option<String>,
    pub plan: Option<String>,
    pub live: bool,
    /// Matches `creator_user_id` in Codex session records. Kept out of the frontend payload.
    #[serde(skip)]
    pub chatgpt_user_id: Option<String>,
}
pub fn summarize(auth: &Value, live: bool) -> Option<LoginSummary> {
    let claims = auth
        .pointer("/tokens/id_token")
        .and_then(Value::as_str)
        .and_then(jwt_claims)
        .unwrap_or(Value::Null);
    let string = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    Some(LoginSummary {
        identity: login_id(auth)?,
        email: string(claims.get("email")),
        workspace: string(auth.pointer("/tokens/account_id")),
        plan: string(
            claims
                .get("https://api.openai.com/auth")
                .and_then(|v| v.get("chatgpt_plan_type"))
                .or_else(|| claims.get("chatgpt_plan_type")),
        ),
        live,
        chatgpt_user_id: auth
            .pointer("/tokens/id_token")
            .and_then(Value::as_str)
            .and_then(extract_chatgpt_user_id),
    })
}
