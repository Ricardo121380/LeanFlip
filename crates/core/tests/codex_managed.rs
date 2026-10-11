//! 托管 ChatGPT 账号（多账号）和混合登录：Plus/Team、托管 vs 原生、refresh token 轮换、
//! 登出、删除账号、官方卡按身份引用登录。全部在临时 HOME 里，网络是假的
//! （`codex::oauth::testing::FakeTransport`，按真实规则每次刷新都轮换、旧 token 作废）。

mod common;

use std::sync::Arc;

use ccsl_core::codex::oauth::live as managed_live;
use ccsl_core::codex::oauth::testing::{FakeTransport, Identity, ManualClock};
use ccsl_core::codex::oauth::{CodexOAuthManager, DeviceFlowPoll, SwitchAwayGuard};
use ccsl_core::provider::AuthBinding;
use ccsl_core::switch::{self, Services};
use ccsl_core::{store, Provider, Tool};
use common::Sandbox;
use serde_json::{json, Value};

const T0: i64 = 1_790_000_000_000;
const HOUR: i64 = 3_600_000;

struct World {
    sb: Sandbox,
    fake: Arc<FakeTransport>,
    clock: Arc<ManualClock>,
    mgr: CodexOAuthManager,
}

fn world() -> World {
    let sb = Sandbox::new();
    sb.write(&sb.env.codex_config_path(), "model = \"gpt-5.5\"\n");
    let fake = Arc::new(FakeTransport::new());
    let clock = Arc::new(ManualClock::new(T0));
    let mgr = CodexOAuthManager::new(&sb.env, Some(fake.clone()))
        .unwrap()
        .with_clock(clock.clone());
    World {
        sb,
        fake,
        clock,
        mgr,
    }
}

impl World {
    fn services(&self) -> Services<'_> {
        Services {
            codex_oauth: Some(&self.mgr),
        }
    }

    fn add_account(&self, identity: &Identity) -> String {
        let tokens = self.fake.issue(identity);
        self.mgr.add_account_from_tokens(&tokens, None).unwrap().id
    }

    fn go(&self, id: &str) -> ccsl_core::operation::OperationReport {
        switch::switch_with(&self.sb.env, self.services(), Tool::Codex, id)
            .unwrap_or_else(|e| panic!("switch to {id}: {e}"))
    }

    fn try_go(
        &self,
        id: &str,
    ) -> Result<ccsl_core::operation::OperationReport, ccsl_core::AppError> {
        switch::switch_with(&self.sb.env, self.services(), Tool::Codex, id)
    }

    fn live(&self) -> Option<Value> {
        let path = self.sb.env.codex_auth_path();
        path.is_file().then(|| self.sb.json(&path))
    }

    fn write_live(&self, auth: &Value) {
        self.sb
            .write(&self.sb.env.codex_auth_path(), &auth.to_string());
    }

    fn live_refresh(&self) -> Option<String> {
        self.live().and_then(|auth| {
            auth.pointer("/tokens/refresh_token")?
                .as_str()
                .map(String::from)
        })
    }

    fn marker(&self) -> Option<Value> {
        let path = managed_live::marker_path(&self.sb.env);
        path.is_file().then(|| self.sb.json(&path))
    }

    fn stash_text(&self) -> String {
        let path = self.sb.env.data_file("codex-login-stash.json");
        if path.is_file() {
            self.sb.read(&path)
        } else {
            String::new()
        }
    }

    fn stash(&self) -> Value {
        serde_json::from_str(&self.stash_text()).unwrap_or(Value::Null)
    }

    /// Codex CLI 自己刷新了 live 里的登录（服务端轮换，旧的作废），`last_refresh` 是现在。
    fn cli_rotate(&self) -> String {
        self.clock.advance(1000);
        let live = self.live().expect("live login");
        let old = live["tokens"]["refresh_token"]
            .as_str()
            .unwrap()
            .to_string();
        let tokens = self.fake.cli_refresh(&old).expect("cli refresh");
        let rotated = managed_live::managed_auth_value(
            live["tokens"]["account_id"].as_str().unwrap(),
            &tokens.access_token,
            tokens.id_token.as_deref(),
            tokens.refresh_token.as_deref().unwrap(),
            &managed_live::format_last_refresh(self.clock_now()),
        );
        self.write_live(&rotated);
        tokens.refresh_token.unwrap()
    }

    fn clock_now(&self) -> i64 {
        use ccsl_core::codex::oauth::transport::Clock;
        self.clock.now_ms()
    }

    fn manager_refresh(&self, account: &str) -> String {
        self.mgr.account(account).unwrap().refresh_token
    }
}

/// Codex 原生浏览器登录写出的 `auth.json`（不是托管账号写的）。
fn native(identity: &Identity, n: u32) -> Value {
    json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": identity.id_token(),
            "access_token": format!("native-at-{}-{n}", identity.user),
            "refresh_token": format!("native-rt-{}-{}-{n}", identity.user, identity.workspace),
            "account_id": identity.workspace,
        },
        "last_refresh": "2026-10-01T00:00:00Z",
    })
}

fn official(id: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({ "auth": {}, "config": "model = \"gpt-5.5\"\n" }),
    )
    .official()
}

fn managed(id: &str, account: &str) -> Provider {
    let mut card = official(id);
    card.meta.auth_binding = Some(AuthBinding {
        auth_provider: "codex_oauth".into(),
        account_id: Some(account.into()),
    });
    card
}

fn relay(id: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({
            "auth": { "OPENAI_API_KEY": format!("sk-{id}") },
            "config": format!("model_provider = \"{id}\"\nmodel = \"m\"\n\n[model_providers.{id}]\nname = \"{id}\"\nbase_url = \"https://{id}.example/v1\"\nwire_api = \"responses\"\n"),
        }),
    )
}

fn alice_plus() -> Identity {
    Identity::new("alice", "ws-plus-alice", "plus")
}
fn alice_team() -> Identity {
    Identity::new("alice", "ws-team", "team")
}
fn bob_plus() -> Identity {
    Identity::new("bob", "ws-plus-bob", "plus")
}
fn bob_team() -> Identity {
    Identity::new("bob", "ws-team", "team")
}
fn carol_team() -> Identity {
    Identity::new("carol", "ws-team", "team")
}

/// 标准场景：alice 在官方卡上原生登录着；bob 的 Plus 和 Team 是两个托管账号。
fn standard() -> (World, String, String) {
    let w = world();
    let bp = w.add_account(&bob_plus());
    let bt = w.add_account(&bob_team());
    w.sb.add(Tool::Codex, official("official"));
    w.sb.add(Tool::Codex, managed("bob-plus", &bp));
    w.sb.add(Tool::Codex, managed("bob-team", &bt));
    w.sb.add(Tool::Codex, relay("relay"));
    store::set_current(&w.sb.env, Tool::Codex, "official").unwrap();
    w.write_live(&native(&alice_plus(), 1));
    (w, bp, bt)
}

// ==================== 账号管理 ====================

#[test]
fn device_flow_adds_plus_and_team_as_two_accounts_and_rejects_duplicates() {
    let w = world();
    let flow = w.mgr.start_device_flow(None).unwrap();
    assert_eq!(
        flow.verification_uri,
        "https://auth.openai.com/codex/device"
    );
    assert_eq!(
        w.mgr.poll_device_flow(&flow.device_code).unwrap(),
        DeviceFlowPoll::Pending
    );
    w.fake.approve(&flow.device_code, &bob_plus());
    let DeviceFlowPoll::Added(plus) = w.mgr.poll_device_flow(&flow.device_code).unwrap() else {
        panic!("not added");
    };
    assert_eq!(plus.plan.as_deref(), Some("plus"));
    assert_eq!(plus.workspace.as_deref(), Some("ws-plus-bob"));
    assert_eq!(plus.email.as_deref(), Some("bob@example.com"));
    assert!(plus.is_default && !plus.reauth_required);

    // 同一个人的 Team 是另一个账号。
    let team = w.add_account(&bob_team());
    assert_ne!(team, plus.id);
    assert_eq!(w.mgr.list_accounts().len(), 2);

    // 同一用户 + 同一 workspace 不能加两次；同 workspace 的另一个用户可以。
    let dup = w
        .mgr
        .add_account_from_tokens(&w.fake.issue(&bob_team()), None);
    assert!(matches!(
        dup.unwrap_err(),
        ccsl_core::AppError::Localized {
            key: "codex.oauth.duplicate_account",
            ..
        }
    ));
    w.add_account(&carol_team());
    assert_eq!(w.mgr.list_accounts().len(), 3);

    // 取消的登录不能再提交。
    let flow = w.mgr.start_device_flow(None).unwrap();
    w.fake.approve(&flow.device_code, &alice_plus());
    assert!(w.mgr.cancel_device_flow(&flow.device_code));
    assert!(w.mgr.poll_device_flow(&flow.device_code).is_err());
    assert_eq!(w.mgr.list_accounts().len(), 3);

    // 库文件 0600，重新打开还在，默认账号不变。
    let path = w.mgr.store_path();
    #[cfg(unix)]
    assert_eq!(common::mode(&path), 0o600);
    let reopened = CodexOAuthManager::new(&w.sb.env, None).unwrap();
    assert_eq!(reopened.list_accounts().len(), 3);
    assert_eq!(
        reopened.default_account_id().as_deref(),
        Some(plus.id.as_str())
    );
}

#[test]
fn reauth_updates_the_account_in_place_and_refuses_another_user() {
    let w = world();
    let id = w.add_account(&bob_team());
    let tokens = w.fake.issue(&bob_team());
    let info = w.mgr.add_account_from_tokens(&tokens, Some(&id)).unwrap();
    assert_eq!(info.id, id, "the binding id is kept");
    assert_eq!(w.manager_refresh(&id), tokens.refresh_token.unwrap());

    let err = w
        .mgr
        .add_account_from_tokens(&w.fake.issue(&carol_team()), Some(&id))
        .unwrap_err();
    assert!(matches!(
        err,
        ccsl_core::AppError::Localized {
            key: "codex.oauth.reauth_user_mismatch",
            ..
        }
    ));
    let err = w
        .mgr
        .add_account_from_tokens(&w.fake.issue(&bob_plus()), Some(&id))
        .unwrap_err();
    assert!(err.to_string().contains("workspace"), "{err}");
}

#[test]
fn a_corrupt_account_store_is_reported_and_never_overwritten() {
    let sb = Sandbox::new();
    let path = sb.env.data_file("codex_oauth_auth.json");
    sb.write(&path, "{\"accounts\": {\"x\": ");
    assert!(CodexOAuthManager::new(&sb.env, None).is_err());
    assert_eq!(sb.read(&path), "{\"accounts\": {\"x\": ");
}

// ==================== 托管 vs 原生 ====================

#[test]
fn the_native_login_is_stashed_and_restored_around_managed_accounts() {
    let (w, bp, bt) = standard();
    let alice = native(&alice_plus(), 1);

    w.go("bob-plus");
    let live = w.live().unwrap();
    assert_eq!(live["tokens"]["account_id"], "ws-plus-bob");
    assert_eq!(
        live["tokens"]["refresh_token"].as_str().unwrap(),
        w.manager_refresh(&bp)
    );
    assert!(live["last_refresh"].as_str().unwrap().ends_with('Z'));
    let marker = w.marker().unwrap();
    assert_eq!(marker["version"], 3);
    assert_eq!(marker["account_id"], bp.as_str());
    assert_eq!(marker["chatgpt_account_id"], "ws-plus-bob");
    assert_eq!(marker["user_identity"], "sub:bob");
    assert_eq!(
        w.stash()["logins"]["sub:alice|account:ws-plus-alice"],
        alice
    );
    #[cfg(unix)]
    {
        assert_eq!(common::mode(&w.sb.env.codex_auth_path()), 0o600);
        assert_eq!(common::mode(&managed_live::marker_path(&w.sb.env)), 0o600);
    }

    // Plus → Team（同一个人）：bundle 和标记都换成 Team；暂存里仍只有 alice。
    w.go("bob-team");
    assert_eq!(w.live().unwrap()["tokens"]["account_id"], "ws-team");
    assert_eq!(w.marker().unwrap()["account_id"], bt.as_str());
    assert!(
        !w.stash_text().contains("rt-bob"),
        "managed tokens never enter the stash"
    );

    // 回到官方卡：alice 原样回来，标记删掉。
    w.go("official");
    assert_eq!(w.live(), Some(alice));
    assert!(w.marker().is_none());
    assert_eq!(w.stash()["logins"], json!({}));
    assert_eq!(w.sb.current(Tool::Codex).as_deref(), Some("official"));
    assert!(w.sb.temp_files().is_empty());
}

#[test]
fn switching_from_a_managed_account_to_a_third_party_removes_the_login_and_marker() {
    let (w, bp, _) = standard();
    w.go("bob-plus");
    w.go("relay");
    assert_eq!(w.live(), None, "no auth.json next to a third-party route");
    assert!(w.marker().is_none());
    assert!(!w.stash_text().contains("rt-bob"));
    let config = w.sb.read(&w.sb.env.codex_config_path());
    assert!(config.contains("requires_openai_auth = false"), "{config}");

    // 托管账号的 token 在管理器里，切回来重新写出。
    w.go("bob-plus");
    assert_eq!(w.live_refresh().unwrap(), w.manager_refresh(&bp));
    assert_eq!(w.marker().unwrap()["account_id"], bp.as_str());
    // alice 一直在暂存里。
    w.go("official");
    assert_eq!(w.live(), Some(native(&alice_plus(), 1)));
}

/// 同一个 Team workspace 里的另一个用户：标记按用户身份认，不会把 carol 的原生登录当成
/// bob 的托管登录删掉。
#[test]
fn another_user_in_the_same_team_workspace_is_never_treated_as_managed() {
    let (w, _, bt) = standard();
    w.go("bob-team");
    let marker: managed_live::Marker = serde_json::from_value(w.marker().unwrap()).unwrap();
    let carol = native(&carol_team(), 1);
    assert!(!managed_live::auth_matches_marker(
        &carol,
        Some(&marker),
        &bt
    ));

    // carol 在 Codex 里自己登录（覆盖了 bob 的托管登录），然后切到第三方：carol 进暂存。
    w.write_live(&carol);
    w.go("relay");
    assert_eq!(w.stash()["logins"]["sub:carol|account:ws-team"], carol);
    assert!(w.marker().is_none(), "the stale marker is removed");
    // 管理器里的 bob 没被 carol 的 token 污染。
    assert!(w.manager_refresh(&bt).starts_with("rt-bob-ws-team"));
}

/// 托管账号切走后，同一个人用 Codex 原生登录同一个 workspace：没有标记，按原生登录处理。
#[test]
fn a_native_login_of_the_same_account_after_switching_away_stays_native() {
    let (w, _, _) = standard();
    w.go("bob-team");
    w.go("official");
    let bob_native = native(&bob_team(), 7);
    w.write_live(&bob_native);
    w.go("relay");
    assert_eq!(w.stash()["logins"]["sub:bob|account:ws-team"], bob_native);
    w.go("official");
    assert_eq!(w.live(), Some(bob_native));
}

// ==================== 轮换 ====================

#[test]
fn a_cli_rotation_is_adopted_before_switching_away_and_used_next_time() {
    let (w, bp, _) = standard();
    w.go("bob-plus");
    let r2 = w.cli_rotate();
    w.go("official");
    assert_eq!(
        w.manager_refresh(&bp),
        r2,
        "the CLI's newer generation was adopted"
    );
    assert_eq!(w.live(), Some(native(&alice_plus(), 1)));

    // 两小时后 access token 过期：切回时用 r2 刷新（不是作废了的 r1），live 是新一代。
    w.clock.advance(2 * HOUR);
    w.go("bob-plus");
    assert_eq!(w.fake.refresh_calls().last(), Some(&r2));
    let r3 = w.live_refresh().unwrap();
    assert_ne!(r3, r2);
    assert_eq!(w.manager_refresh(&bp), r3);
    assert!(w.fake.is_valid(&r3) && !w.fake.is_valid(&r2));
}

#[test]
fn an_ambiguous_generation_refuses_to_switch_away() {
    for last_refresh in [Some(T0), None] {
        let (w, bp, _) = standard();
        w.go("bob-plus");
        // live 换成了另一代，但时间戳和管理器的一样（或者没有）：分不出新旧。
        let mut live = w.live().unwrap();
        let other = w
            .fake
            .cli_refresh(live["tokens"]["refresh_token"].as_str().unwrap())
            .unwrap();
        live["tokens"]["refresh_token"] = json!(other.refresh_token.clone().unwrap());
        match last_refresh {
            Some(ms) => live["last_refresh"] = json!(managed_live::format_last_refresh(ms)),
            None => {
                live.as_object_mut().unwrap().remove("last_refresh");
            }
        }
        w.write_live(&live);
        let before_manager = w.manager_refresh(&bp);

        let err = w.try_go("official").unwrap_err();
        assert!(matches!(
            err,
            ccsl_core::AppError::Localized {
                key: "codex.oauth.ambiguous_generation",
                ..
            }
        ));
        assert_eq!(w.live(), Some(live), "nothing was deleted");
        assert!(w.marker().is_some());
        assert_eq!(w.manager_refresh(&bp), before_manager);
        assert_eq!(w.sb.current(Tool::Codex).as_deref(), Some("bob-plus"));
    }
}

#[test]
fn a_provably_older_live_generation_may_be_replaced() {
    let (w, bp, _) = standard();
    w.go("bob-plus");
    let old_live = w.live().unwrap();
    // 管理器刷新（live 同步成新一代），然后用户从备份里恢复了旧的 auth.json。
    w.clock.advance(2 * HOUR);
    w.mgr.token_bundle(&bp).unwrap();
    let newest = w.manager_refresh(&bp);
    assert_eq!(
        w.live_refresh().as_deref(),
        Some(newest.as_str()),
        "live was synced"
    );
    w.write_live(&old_live);

    w.go("official");
    assert_eq!(
        w.manager_refresh(&bp),
        newest,
        "the manager is never rolled back"
    );
    assert_eq!(w.live(), Some(native(&alice_plus(), 1)));
}

/// 服务端拒绝了管理器的 refresh token（CLI 已轮换，live 又没有可比的时间戳）：盘上那一代
/// 是唯一的出路，采纳后重试一次。
#[test]
fn a_rejected_manager_token_recovers_from_the_live_generation() {
    let (w, bp, _) = standard();
    w.go("bob-plus");
    let r1 = w.manager_refresh(&bp);
    let mut live = w.live().unwrap();
    let r2 = w.fake.cli_refresh(&r1).unwrap().refresh_token.unwrap();
    live["tokens"]["refresh_token"] = json!(r2);
    live.as_object_mut().unwrap().remove("last_refresh");
    w.write_live(&live);

    w.clock.advance(2 * HOUR);
    let bundle = w.mgr.token_bundle(&bp).unwrap();
    assert_eq!(w.fake.refresh_calls(), vec![r1, r2.clone()]);
    let r3 = w.manager_refresh(&bp);
    assert_eq!(bundle.refresh_token, r3);
    assert_eq!(
        w.live_refresh().as_deref(),
        Some(r3.as_str()),
        "live synced to r3"
    );
}

/// 管理器刷新的同时 Codex CLI 也刷新了：最后的 bundle 是同一代（access、refresh、live
/// 三者一致），不会是「旧 access + 新 refresh」的混合体。
#[test]
fn a_cli_refresh_racing_the_manager_never_produces_a_mixed_bundle() {
    let (w, bp, _) = standard();
    w.go("bob-plus");
    w.clock.advance(2 * HOUR);
    let fake = w.fake.clone();
    let auth_path = w.sb.env.codex_auth_path();
    let now = w.clock_now();
    let fired = std::sync::atomic::AtomicBool::new(false);
    fake.clone()
        .set_on_refresh(Some(Box::new(move |used: &str| {
            // 只在第一次刷新时插一脚：CLI 用同一个 refresh token 先刷新并写盘。
            if fired.swap(true, std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let Ok(text) = std::fs::read_to_string(&auth_path) else {
                return;
            };
            let mut live: Value = serde_json::from_str(&text).unwrap();
            if live["tokens"]["refresh_token"] != used {
                return;
            }
            let tokens = fake.cli_refresh(used).unwrap();
            live["tokens"]["refresh_token"] = json!(tokens.refresh_token.unwrap());
            live["tokens"]["access_token"] = json!(tokens.access_token);
            live["last_refresh"] = json!(managed_live::format_last_refresh(now + 5));
            std::fs::write(&auth_path, live.to_string()).unwrap();
        })));
    w.go("bob-plus");
    w.fake.set_on_refresh(None);
    let live = w.live().unwrap();
    let refresh = live["tokens"]["refresh_token"].as_str().unwrap();
    assert_eq!(refresh, w.manager_refresh(&bp));
    assert!(
        w.fake.is_valid(refresh),
        "the written refresh token is the live one"
    );
    let access = live["tokens"]["access_token"].as_str().unwrap();
    let n = |token: &str| token.rsplit('-').next().unwrap().parse::<u64>().unwrap();
    assert_eq!(
        n(access),
        n(refresh),
        "access and refresh are one generation"
    );
}

#[test]
fn the_switch_away_guard_catches_a_rotation_or_logout_after_prepare() {
    let (w, bp, _) = standard();
    w.go("bob-plus");
    let guard = w.mgr.prepare_switch_away(&bp).unwrap();
    assert!(matches!(&guard, SwitchAwayGuard::ExistingAccount(Some(_))));
    guard.ensure_unchanged(&w.sb.env, &bp).unwrap();
    w.cli_rotate();
    assert!(guard.ensure_unchanged(&w.sb.env, &bp).is_err());
    std::fs::remove_file(w.sb.env.codex_auth_path()).unwrap();
    assert!(guard.ensure_unchanged(&w.sb.env, &bp).is_err());
}

// ==================== 登出、删除账号 ====================

#[test]
fn logging_out_in_codex_while_managed_then_switching_away() {
    let (w, _, _) = standard();
    w.go("bob-plus");
    std::fs::remove_file(w.sb.env.codex_auth_path()).unwrap(); // codex logout
    w.go("official");
    assert_eq!(
        w.live(),
        Some(native(&alice_plus(), 1)),
        "alice comes back from the stash"
    );
    assert!(w.marker().is_none(), "the stale marker is cleaned up");
    w.go("bob-plus");
    assert!(w.marker().is_some());
}

#[test]
fn removing_an_account_clears_its_live_login_and_unbinds_safely() {
    let (w, bp, _) = standard();
    w.go("bob-plus");
    w.mgr.remove_account(&bp).unwrap();
    assert_eq!(w.live(), None);
    assert!(w.marker().is_none());
    assert!(w.mgr.account(&bp).is_none());

    // 指针还在那张卡上；切走不需要这个账号。
    w.go("official");
    assert_eq!(w.live(), Some(native(&alice_plus(), 1)));
    // 切回去：账号没了，拒绝，什么都不改。
    let err = w.try_go("bob-plus").unwrap_err();
    assert!(matches!(
        err,
        ccsl_core::AppError::Localized {
            key: "codex.oauth.account_not_found",
            ..
        }
    ));
    assert_eq!(w.live(), Some(native(&alice_plus(), 1)));
    assert_eq!(w.sb.current(Tool::Codex).as_deref(), Some("official"));
}

#[test]
fn removing_an_account_leaves_a_native_login_alone() {
    let (w, bp, _) = standard();
    w.go("bob-plus");
    w.go("official");
    w.mgr.remove_account(&bp).unwrap();
    assert_eq!(w.live(), Some(native(&alice_plus(), 1)));
}

#[test]
fn clear_all_logs_every_account_out() {
    let (w, _, _) = standard();
    w.go("bob-team");
    w.mgr.clear_all().unwrap();
    assert!(w.mgr.list_accounts().is_empty());
    assert_eq!(w.live(), None);
    assert!(!w.mgr.store_path().exists());
}

// ==================== 官方卡按身份引用登录 ====================

#[test]
fn official_cards_reference_stashed_logins_by_identity() {
    let w = world();
    w.sb.add(Tool::Codex, official("plus"));
    w.sb.add(Tool::Codex, official("team"));
    w.sb.add(Tool::Codex, relay("relay"));
    store::set_current(&w.sb.env, Tool::Codex, "team").unwrap();
    let team = native(&alice_team(), 1);
    w.write_live(&team);

    let logins = switch::codex::available_logins(&w.sb.env).unwrap();
    assert_eq!(logins.len(), 1);
    assert_eq!(logins[0].identity, "sub:alice|account:ws-team");
    assert_eq!(logins[0].plan.as_deref(), Some("team"));
    assert!(logins[0].live);
    switch::codex::set_login_ref(
        &w.sb.env,
        w.services(),
        "team",
        Some("sub:alice|account:ws-team"),
    )
    .unwrap();
    // 不认识的身份、第三方卡都拒绝。
    assert!(
        switch::codex::set_login_ref(&w.sb.env, w.services(), "plus", Some("sub:x|account:y"))
            .is_err()
    );
    assert!(switch::codex::set_login_ref(&w.sb.env, w.services(), "relay", None).is_err());

    // 第三方：team 进暂存。
    w.go("relay");
    // plus 卡引用 alice 的 Plus：暂存里还没有 → 报告缺失，auth.json 不动。
    let mut plus_card = store::load(&w.sb.env)
        .unwrap()
        .provider(Tool::Codex, "plus")
        .unwrap()
        .clone();
    plus_card.set_codex_login_ref(Some("sub:alice|account:ws-plus-alice"));
    store::replace_provider(&w.sb.env, Tool::Codex, plus_card).unwrap();
    let report = w.go("plus");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.starts_with("codex_login_ref_missing")),
        "{:?}",
        report.warnings
    );
    assert_eq!(w.live(), None);

    // 用户在 Codex 里登录 Plus；切到 team 卡：Plus 进暂存，Team 换回来。
    let plus = native(&alice_plus(), 1);
    w.write_live(&plus);
    w.go("team");
    assert_eq!(w.live(), Some(team.clone()));
    w.go("plus");
    assert_eq!(w.live(), Some(plus.clone()));
    w.go("team");
    assert_eq!(w.live(), Some(team.clone()));
    let identities: Vec<String> = switch::codex::available_logins(&w.sb.env)
        .unwrap()
        .into_iter()
        .map(|l| l.identity)
        .collect();
    assert_eq!(identities.len(), 2, "{identities:?}");

    // 引用托管账号之外的登录不会把托管 token 带进卡里：卡里只有身份。
    let row = store::load(&w.sb.env)
        .unwrap()
        .provider(Tool::Codex, "plus")
        .unwrap()
        .clone();
    assert_eq!(row.settings_config["auth"], json!({}));
    assert_eq!(
        row.codex_login_ref(),
        Some("sub:alice|account:ws-plus-alice")
    );
}

#[test]
fn logging_out_on_a_referencing_card_sticks() {
    let w = world();
    let mut team_card = official("team");
    team_card.set_codex_login_ref(Some("sub:alice|account:ws-team"));
    w.sb.add(Tool::Codex, team_card);
    w.sb.add(Tool::Codex, relay("relay"));
    store::set_current(&w.sb.env, Tool::Codex, "team").unwrap();
    w.write_live(&native(&alice_team(), 1));
    w.go("relay");
    w.go("team");
    assert!(w.live().is_some());
    std::fs::remove_file(w.sb.env.codex_auth_path()).unwrap(); // codex logout
    w.go("relay");
    let report = w.go("team");
    assert_eq!(w.live(), None, "stays logged out");
    assert!(report
        .warnings
        .iter()
        .any(|w| w.starts_with("codex_login_ref_missing")));
}

#[test]
fn a_managed_card_and_a_referencing_card_mix_without_crossing() {
    let w = world();
    let bt = w.add_account(&bob_team());
    let mut alice_card = official("alice-team");
    alice_card.set_codex_login_ref(Some("sub:alice|account:ws-team"));
    w.sb.add(Tool::Codex, alice_card);
    w.sb.add(Tool::Codex, managed("bob-team", &bt));
    store::set_current(&w.sb.env, Tool::Codex, "alice-team").unwrap();
    let alice = native(&alice_team(), 1);
    w.write_live(&alice);
    for _ in 0..3 {
        w.go("bob-team");
        assert_eq!(
            w.live().unwrap()["tokens"]["id_token"],
            json!(bob_team().id_token())
        );
        assert!(w.marker().is_some());
        w.go("alice-team");
        assert_eq!(w.live(), Some(alice.clone()));
        assert!(w.marker().is_none());
    }
    assert!(!w.stash_text().contains("rt-bob"));
}
