//! 混合登录：ChatGPT OAuth 登录（auth.json、登录暂存、Plus/Team workspace）和 API Key
//! 供应商并存时的保护。全部在临时 HOME 里跑。

mod common;

use ccsl_core::codex::auth::test_codex_id_token;
use ccsl_core::{store, switch, Provider, Tool};
use common::Sandbox;
use serde_json::{json, Value};

fn login(user: &str, account: &str) -> Value {
    json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": test_codex_id_token(user),
            "access_token": format!("access-{account}"),
            "refresh_token": format!("refresh-{account}"),
            "account_id": account,
        },
        "last_refresh": "2026-10-01T00:00:00Z",
    })
}

fn official(id: &str, auth: Value) -> Provider {
    Provider::new(
        id,
        id,
        json!({ "auth": auth, "config": "model = \"gpt-5.5\"\n" }),
    )
    .official()
}

fn relay(id: &str, key: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({
            "auth": { "OPENAI_API_KEY": key },
            "config": format!("model_provider = \"{id}\"\nmodel = \"m\"\n\n[model_providers.{id}]\nname = \"{id}\"\nbase_url = \"https://{id}.example/v1\"\nwire_api = \"responses\"\n"),
        }),
    )
}

fn sandbox(cards: Vec<Provider>, current: &str, live_auth: Option<&Value>) -> Sandbox {
    let sb = Sandbox::new();
    for card in cards {
        sb.add(Tool::Codex, card);
    }
    store::set_current(&sb.env, Tool::Codex, current).unwrap();
    sb.write(&sb.env.codex_config_path(), "model = \"gpt-5.5\"\n");
    if let Some(auth) = live_auth {
        sb.write(&sb.env.codex_auth_path(), &auth.to_string());
    }
    sb
}

fn live_auth(sb: &Sandbox) -> Option<Value> {
    let path = sb.env.codex_auth_path();
    path.is_file().then(|| sb.json(&path))
}

fn go(sb: &Sandbox, id: &str) {
    switch::switch(&sb.env, Tool::Codex, id).unwrap_or_else(|e| panic!("switch to {id}: {e}"));
}

/// 同一个人的 Plus 和 Team 是两个登录：官方卡各存一份（旧版回填的快照），切换时按
/// 「用户 + workspace」换，换下来的那份进暂存，谁都不丢。
#[test]
fn plus_and_team_workspaces_swap_without_losing_either() {
    let plus = login("alice", "acc-plus");
    let team = login("alice", "acc-team");
    let sb = sandbox(
        vec![
            official("plus", plus.clone()),
            official("team", team.clone()),
        ],
        "plus",
        Some(&plus),
    );

    go(&sb, "team");
    assert_eq!(live_auth(&sb), Some(team.clone()));
    let stash = sb.json(&sb.env.data_file("codex-login-stash.json"));
    assert_eq!(stash["logins"]["sub:alice|account:acc-plus"], plus);

    go(&sb, "plus");
    assert_eq!(live_auth(&sb), Some(plus.clone()));
    let stash = sb.json(&sb.env.data_file("codex-login-stash.json"));
    assert_eq!(stash["logins"]["sub:alice|account:acc-team"], team);
    assert!(stash["logins"].get("sub:alice|account:acc-plus").is_none());
}

/// Codex CLI 轮换过 token 后，行里的旧快照不会盖掉 live 里更新的同一个登录。
#[test]
fn a_stale_row_snapshot_never_overwrites_the_live_login_of_the_same_person() {
    let old = login("alice", "acc-plus");
    let mut rotated = old.clone();
    rotated["tokens"]["refresh_token"] = json!("refresh-rotated");
    let sb = sandbox(
        vec![official("plus", old), relay("relay", "sk-relay")],
        "relay",
        Some(&rotated),
    );
    sb.set_preserve(true);
    go(&sb, "plus");
    assert_eq!(
        live_auth(&sb),
        Some(rotated),
        "same identity: auth.json untouched"
    );
}

/// 在官方卡上登出（删了 auth.json）后切走再切回：登出生效，不会被暂存「复活」。
#[test]
fn logging_out_on_the_official_card_sticks() {
    let alice = login("alice", "acc-plus");
    let sb = sandbox(
        vec![official("official", json!({})), relay("relay", "sk-relay")],
        "official",
        Some(&alice),
    );
    go(&sb, "relay");
    go(&sb, "official");
    assert_eq!(live_auth(&sb), Some(alice.clone()), "login comes back");

    std::fs::remove_file(sb.env.codex_auth_path()).unwrap(); // codex logout
    go(&sb, "relay");
    go(&sb, "official");
    assert_eq!(live_auth(&sb), None, "logout must stick");
    let stash = sb.json(&sb.env.data_file("codex-login-stash.json"));
    assert!(stash["logins"].as_object().unwrap().is_empty(), "{stash}");
}

/// 旧版留在 auth.json 里的第三方 Key（能证明是某张第三方卡的）切回官方时删掉；
/// 用户自己 `codex login --api-key` 的 Key 不动。
#[test]
fn only_provable_third_party_key_residue_is_removed() {
    let sb = sandbox(
        vec![official("official", json!({})), relay("relay", "sk-relay")],
        "relay",
        Some(&json!({ "OPENAI_API_KEY": "sk-relay" })),
    );
    go(&sb, "official");
    assert_eq!(live_auth(&sb), None, "stale third-party key removed");

    let own = json!({ "OPENAI_API_KEY": "sk-my-own", "auth_mode": "apikey" });
    let sb = sandbox(
        vec![official("official", json!({})), relay("relay", "sk-relay")],
        "relay",
        Some(&own),
    );
    sb.set_preserve(true);
    go(&sb, "official");
    assert_eq!(
        live_auth(&sb),
        Some(own),
        "the user's own API key login stays"
    );
}

/// 直连 OpenAI API 的官方卡（行里存 API Key）：整份写它，换下来的 ChatGPT 登录进暂存。
///
/// Phase 1 锁住的边界（和 cc-switch 原版一致）：再切回「跟随当前登录」的官方卡时
/// API Key 留着，ChatGPT 登录困在暂存里。Phase 2 修掉：官方卡写进去的 API Key 不算原生
/// 登录，切回 OAuth 官方卡时换回暂存里的登录；API Key 不进暂存（它本来就在卡里）。
#[test]
fn an_api_key_official_card_stashes_the_chatgpt_login() {
    let alice = login("alice", "acc-plus");
    let api = json!({ "OPENAI_API_KEY": "sk-openai" });
    let sb = sandbox(
        vec![official("chatgpt", json!({})), official("api", api.clone())],
        "chatgpt",
        Some(&alice),
    );
    go(&sb, "api");
    assert_eq!(live_auth(&sb), Some(api.clone()));
    let stash = sb.json(&sb.env.data_file("codex-login-stash.json"));
    assert_eq!(stash["logins"]["sub:alice|account:acc-plus"], alice);

    go(&sb, "chatgpt");
    assert_eq!(
        live_auth(&sb),
        Some(alice.clone()),
        "the ChatGPT login is back"
    );
    let stash = sb.json(&sb.env.data_file("codex-login-stash.json"));
    assert_eq!(stash["logins"], json!({}), "the API key is not stashed");

    // 来回再走一次，结果一样；从 API Key 卡切到第三方时 Key 不进暂存、登录也不丢。
    go(&sb, "api");
    assert_eq!(live_auth(&sb), Some(api));
    sb.add(Tool::Codex, relay("relay", "sk-relay"));
    go(&sb, "relay");
    assert_eq!(live_auth(&sb), None);
    let stash = sb.json(&sb.env.data_file("codex-login-stash.json"));
    assert_eq!(stash["logins"]["sub:alice|account:acc-plus"], alice);
    assert!(stash["logins"].get("api-key").is_none(), "{stash}");
    go(&sb, "chatgpt");
    assert_eq!(live_auth(&sb), Some(alice));
}

/// 暂存文件坏了、这次又要往里存登录：停下，什么都不写，指针不动。
#[test]
fn an_unreadable_stash_refuses_instead_of_overwriting() {
    let alice = login("alice", "acc-plus");
    let sb = sandbox(
        vec![official("official", json!({})), relay("relay", "sk-relay")],
        "official",
        Some(&alice),
    );
    let stash = sb.env.data_file("codex-login-stash.json");
    sb.write(&stash, "{\"logins\": {\"sub:bob\": ");
    let config_before = sb.read(&sb.env.codex_config_path());

    let err = switch::switch(&sb.env, Tool::Codex, "relay").unwrap_err();
    assert!(err.to_string().contains("登录暂存"), "{err}");
    assert_eq!(live_auth(&sb), Some(alice));
    assert_eq!(sb.read(&stash), "{\"logins\": {\"sub:bob\": ");
    assert_eq!(sb.read(&sb.env.codex_config_path()), config_before);
    assert_eq!(sb.current(Tool::Codex).as_deref(), Some("official"));

    // 不需要动暂存的切换（保留登录打开）照常进行。
    sb.set_preserve(true);
    go(&sb, "relay");
    assert_eq!(sb.read(&stash), "{\"logins\": {\"sub:bob\": ");
}

/// 登录存在系统钥匙串（keyring/auto）时 auth.json 看不到登录：按保留开关推断；
/// ephemeral 当成没登录。
#[test]
fn credential_store_modes_decide_requires_openai_auth() {
    for (mode, preserve, expected) in [
        ("keyring", false, false),
        ("keyring", true, true),
        ("auto", true, true),
        ("ephemeral", true, false),
    ] {
        let sb = sandbox(
            vec![official("official", json!({})), relay("relay", "sk-relay")],
            "official",
            None,
        );
        sb.set_preserve(preserve);
        sb.write(
            &sb.env.codex_config_path(),
            &format!("cli_auth_credentials_store = \"{mode}\"\n"),
        );
        go(&sb, "relay");
        let doc: toml::Table = toml::from_str(&sb.read(&sb.env.codex_config_path())).unwrap();
        assert_eq!(
            doc["model_providers"]["custom"]["requires_openai_auth"].as_bool(),
            Some(expected),
            "{mode} preserve={preserve}"
        );
        assert_eq!(doc["cli_auth_credentials_store"].as_str(), Some(mode));
    }
}

/// 绑定了托管账号的卡，但没有账号管理器（`switch` 而不是 `switch_with`）：明确拒绝，
/// 不写任何东西。
#[test]
fn managed_account_cards_need_the_account_manager() {
    let alice = login("alice", "acc-plus");
    let mut managed = official("managed", json!({}));
    managed.meta.auth_binding = Some(ccsl_core::provider::AuthBinding {
        auth_provider: "codex_oauth".to_string(),
        account_id: Some("acc-x".to_string()),
    });
    let sb = sandbox(
        vec![official("official", json!({})), managed],
        "official",
        Some(&alice),
    );
    let err = switch::switch(&sb.env, Tool::Codex, "managed").unwrap_err();
    assert!(matches!(
        err,
        ccsl_core::AppError::Localized {
            key: "codex.oauth.manager_unavailable",
            ..
        }
    ));
    assert_eq!(live_auth(&sb), Some(alice));
    assert_eq!(sb.current(Tool::Codex).as_deref(), Some("official"));
}
