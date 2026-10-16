//! 场景：记录、计划、一致性判断和场景本身的增删改，全部在临时 HOME 里。

mod common;

use common::Sandbox;
use leanflip_core::scene::{self, Action, Scene, SceneTool};
use leanflip_core::{coexist, store, switch, AppError, Provider, Tool};
use serde_json::json;

fn claude(id: &str) -> Provider {
    Provider::new(
        id,
        id.to_uppercase(),
        json!({"env": {"ANTHROPIC_BASE_URL": format!("https://{id}.example"), "ANTHROPIC_AUTH_TOKEN": format!("sk-{id}")}}),
    )
}

fn opencode(id: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({"npm": "@ai-sdk/openai-compatible", "name": id,
               "options": {"baseURL": format!("https://{id}"), "apiKey": "sk"}, "models": {"m": {}}}),
    )
}

fn hermes(id: &str) -> Provider {
    Provider::new(
        id,
        id,
        json!({"baseUrl": format!("https://{id}/v1"), "api_key": "k", "api": "openai-completions",
               "models": [{"id": "m-1"}]}),
    )
}

/// Claude 两张卡（当前 a），OpenCode 两张卡（只启用 x）加一个用户自己的条目。
fn setup() -> Sandbox {
    let sb = Sandbox::new();
    sb.write(
        &sb.env.opencode_dir.join("opencode.json"),
        r#"{"provider": {"mine": {"npm": "@ai-sdk/openai-compatible", "options": {"baseURL": "https://mine"}, "models": {"m": {}}}}}"#,
    );
    sb.add(Tool::Claude, claude("a"));
    sb.add(Tool::Claude, claude("b"));
    sb.add(Tool::OpenCode, opencode("x"));
    sb.add(Tool::OpenCode, opencode("y"));
    switch::switch(&sb.env, Tool::Claude, "a").unwrap();
    coexist::write_to_live(&sb.env, Tool::OpenCode, "x").unwrap();
    sb
}

fn named(name: &str, tools: &[(Tool, SceneTool)]) -> Scene {
    Scene {
        name: name.into(),
        tools: tools
            .iter()
            .map(|(tool, entry)| (tool.as_str().to_string(), entry.clone()))
            .collect(),
        ..Scene::default()
    }
}

fn plan(sb: &Sandbox, scene: &Scene, tool: Tool) -> scene::ToolPlan {
    let data = store::load(&sb.env).unwrap();
    scene::plan_tool(&sb.env, &data, tool, &scene.tools[tool.as_str()]).unwrap()
}

#[test]
fn capture_records_current_cards_and_only_leanflip_entries() {
    let sb = setup();
    let data = store::load(&sb.env).unwrap();
    assert_eq!(
        scene::capture_tool(&sb.env, &data, Tool::Claude).unwrap(),
        Some(SceneTool::current("a"))
    );
    assert_eq!(
        scene::capture_tool(&sb.env, &data, Tool::OpenCode).unwrap(),
        Some(SceneTool::enabled(["x"]))
    );
    assert_eq!(
        scene::capture_tool(&sb.env, &data, Tool::Codex).unwrap(),
        None
    );
    assert_eq!(scene::capture_tool(&sb.env, &data, Tool::Pi).unwrap(), None);
}

#[test]
fn plan_lists_switches_and_toggles_and_leaves_user_entries_alone() {
    let sb = setup();
    let scene = named(
        "B",
        &[
            (Tool::Claude, SceneTool::current("b")),
            (Tool::OpenCode, SceneTool::enabled(["y"])),
        ],
    );
    assert_eq!(
        plan(&sb, &scene, Tool::Claude).actions,
        [Action::Switch("b".into())]
    );
    assert_eq!(
        plan(&sb, &scene, Tool::OpenCode).actions,
        [Action::Disable("x".into()), Action::Enable("y".into())]
    );
    assert!(!scene::matches(
        &sb.env,
        &store::load(&sb.env).unwrap(),
        &scene
    ));

    switch::switch(&sb.env, Tool::Claude, "b").unwrap();
    coexist::remove_from_live(&sb.env, Tool::OpenCode, "x").unwrap();
    coexist::write_to_live(&sb.env, Tool::OpenCode, "y").unwrap();
    assert!(scene::matches(
        &sb.env,
        &store::load(&sb.env).unwrap(),
        &scene
    ));
    let live = coexist::live_entries(&sb.env, Tool::OpenCode).unwrap();
    assert!(live.contains_key("mine"), "user entry kept");
}

#[test]
fn a_hand_edited_client_file_needs_the_switch_again() {
    let sb = setup();
    let scene = named("A", &[(Tool::Claude, SceneTool::current("a"))]);
    assert!(plan(&sb, &scene, Tool::Claude).in_sync());
    sb.write(
        &sb.env.claude_settings_path(),
        r#"{"env": {"ANTHROPIC_BASE_URL": "https://elsewhere"}}"#,
    );
    assert_eq!(
        plan(&sb, &scene, Tool::Claude).actions,
        [Action::Switch("a".into())]
    );
}

#[test]
fn deleted_cards_are_reported_missing_and_skipped() {
    let sb = setup();
    let scene = named(
        "A",
        &[
            (Tool::Claude, SceneTool::current("gone")),
            (Tool::OpenCode, SceneTool::enabled(["x", "gone"])),
        ],
    );
    let claude = plan(&sb, &scene, Tool::Claude);
    assert!(claude.actions.is_empty());
    assert_eq!(claude.missing, ["gone"]);
    let opencode = plan(&sb, &scene, Tool::OpenCode);
    assert!(opencode.actions.is_empty());
    assert_eq!(opencode.missing, ["gone"]);
    assert!(!scene::matches(
        &sb.env,
        &store::load(&sb.env).unwrap(),
        &scene
    ));
}

#[test]
fn hermes_scenes_track_the_default_provider() {
    let sb = Sandbox::new();
    sb.write(
        &sb.env.hermes_config_path(),
        "model:\n  default: old\n  provider: openrouter\n",
    );
    sb.add(Tool::Hermes, hermes("r1"));
    sb.add(Tool::Hermes, hermes("r2"));
    coexist::write_to_live(&sb.env, Tool::Hermes, "r1").unwrap();
    coexist::write_to_live(&sb.env, Tool::Hermes, "r2").unwrap();
    let data = store::load(&sb.env).unwrap();
    let captured = scene::capture_tool(&sb.env, &data, Tool::Hermes)
        .unwrap()
        .unwrap();
    assert_eq!(captured.current, None, "openrouter is not a LeanFlip card");
    assert_eq!(captured.enabled, Some(vec!["r1".into(), "r2".into()]));

    let mut entry = SceneTool::enabled(["r1", "r2"]);
    entry.current = Some("r2".into());
    let scene = named("H", &[(Tool::Hermes, entry)]);
    assert_eq!(
        plan(&sb, &scene, Tool::Hermes).actions,
        [Action::Activate("r2".into())]
    );
    switch::switch(&sb.env, Tool::Hermes, "r2").unwrap();
    assert!(plan(&sb, &scene, Tool::Hermes).in_sync());
    assert_eq!(
        coexist::active_entry(&sb.env, Tool::Hermes)
            .unwrap()
            .as_deref(),
        Some("r2")
    );
}

#[test]
fn validation_rejects_malformed_scenes() {
    let invalid = |scene: Scene| {
        assert!(
            matches!(
                scene::validate(&scene),
                Err(AppError::Localized {
                    key: "scene.invalid",
                    ..
                })
            ),
            "{scene:?}"
        )
    };
    invalid(named(" ", &[(Tool::Claude, SceneTool::current("a"))]));
    invalid(named(
        &"x".repeat(61),
        &[(Tool::Claude, SceneTool::current("a"))],
    ));
    invalid(named("A", &[]));
    invalid(named("A", &[(Tool::Claude, SceneTool::enabled(["a"]))]));
    invalid(named("A", &[(Tool::OpenCode, SceneTool::current("a"))]));
    invalid(named(
        "A",
        &[(Tool::OpenCode, SceneTool::enabled(["a", "a"]))],
    ));
    invalid(named("A", &[(Tool::Claude, SceneTool::current(""))]));
    let mut entry = SceneTool::enabled(["r1"]);
    entry.current = Some("r2".into());
    invalid(named("A", &[(Tool::Hermes, entry)]));
    let mut entry = SceneTool::enabled(["x"]);
    entry.current = Some("x".into());
    invalid(named("A", &[(Tool::OpenCode, entry)]));
    // 只有新版本才认识的工具不算数。
    let mut only_unknown = named("A", &[]);
    only_unknown
        .tools
        .insert("future-tool".into(), SceneTool::current("a"));
    invalid(only_unknown);
}

#[test]
fn save_find_and_remove_keep_names_unique_and_unknown_tools_intact() {
    let sb = setup();
    let mut work = named("Work", &[(Tool::Claude, SceneTool::current("a"))]);
    let future: SceneTool = serde_json::from_value(json!({"current": "z", "extra": 1})).unwrap();
    work.tools.insert("future-tool".into(), future);
    let work = scene::save(&sb.env, work).unwrap();
    assert!(work.id.starts_with("scene-"));
    let home = scene::save(
        &sb.env,
        named(" Home ", &[(Tool::Claude, SceneTool::current("b"))]),
    )
    .unwrap();
    assert_eq!(home.name, "Home");
    assert!(matches!(
        scene::save(
            &sb.env,
            named("work", &[(Tool::Claude, SceneTool::current("b"))])
        ),
        Err(AppError::Localized {
            key: "scene.duplicate_name",
            ..
        })
    ));

    let data = store::load(&sb.env).unwrap();
    assert_eq!(scene::find(&data, "WORK").unwrap().id, work.id);
    assert_eq!(scene::find(&data, &home.id).unwrap().name, "Home");
    assert_eq!(data.scenes[0].tools["future-tool"].extra["extra"], 1);

    let mut renamed = data.scenes[0].clone();
    renamed.name = "Office".into();
    scene::save(&sb.env, renamed).unwrap();
    let mut stranger = named("Ghost", &[(Tool::Claude, SceneTool::current("a"))]);
    stranger.id = "scene-nope".into();
    assert!(matches!(
        scene::save(&sb.env, stranger),
        Err(AppError::Localized {
            key: "scene.not_found",
            ..
        })
    ));

    scene::set_last(&sb.env, Some(&work.id)).unwrap();
    assert_eq!(
        store::load(&sb.env).unwrap().last_scene,
        Some(work.id.clone())
    );
    scene::set_last(&sb.env, Some("scene-unknown")).unwrap();
    assert_eq!(store::load(&sb.env).unwrap().last_scene, None);
    scene::set_last(&sb.env, Some(&work.id)).unwrap();
    scene::remove(&sb.env, &work.id).unwrap();
    let data = store::load(&sb.env).unwrap();
    assert_eq!(data.last_scene, None);
    assert_eq!(data.scenes.len(), 1);
}

#[test]
fn stores_without_scenes_keep_their_shape() {
    let sb = setup();
    let text = sb.read(&sb.env.store_path());
    assert!(!text.contains("scenes"), "{text}");
    assert!(!text.contains("lastScene"), "{text}");
}

#[test]
fn keeping_the_hermes_default_never_removes_the_active_entry() {
    let sb = Sandbox::new();
    sb.write(
        &sb.env.hermes_config_path(),
        "model:\n  default: old\n  provider: openrouter\n",
    );
    sb.add(Tool::Hermes, hermes("r1"));
    sb.add(Tool::Hermes, hermes("r2"));
    coexist::write_to_live(&sb.env, Tool::Hermes, "r2").unwrap();
    switch::switch(&sb.env, Tool::Hermes, "r1").unwrap();
    let keep = named("Keep", &[(Tool::Hermes, SceneTool::enabled(["r2"]))]);
    assert!(
        plan(&sb, &keep, Tool::Hermes).in_sync(),
        "r1 is the default, so it stays"
    );
    let mut pick = SceneTool::enabled(["r2"]);
    pick.current = Some("r2".into());
    let pick = named("Pick", &[(Tool::Hermes, pick)]);
    assert_eq!(
        plan(&sb, &pick, Tool::Hermes).actions,
        [Action::Disable("r1".into()), Action::Activate("r2".into())]
    );
}

#[test]
fn hermes_undo_snapshots_restore_a_default_that_is_not_a_leanflip_card() {
    let sb = Sandbox::new();
    sb.write(
        &sb.env.hermes_config_path(),
        "model:\n  default: old\n  provider: openrouter\n",
    );
    sb.add(Tool::Hermes, hermes("r1"));
    let data = store::load(&sb.env).unwrap();
    let undo = scene::undo_entry(&sb.env, &data, Tool::Hermes)
        .unwrap()
        .unwrap();
    assert_eq!(undo.current, None);
    assert_eq!(
        undo.extra[scene::HERMES_MODEL],
        json!({"default": "old", "provider": "openrouter"})
    );
    switch::switch(&sb.env, Tool::Hermes, "r1").unwrap();
    let snapshot = Scene {
        name: "undo".into(),
        tools: [("hermes".to_string(), undo)].into(),
        ..Scene::default()
    };
    let actions = plan(&sb, &snapshot, Tool::Hermes).actions;
    assert_eq!(actions[0], Action::Disable("r1".into()));
    assert!(matches!(actions[1], Action::RestoreDefault(_)));
    coexist::remove_from_live(&sb.env, Tool::Hermes, "r1").unwrap();
    let Action::RestoreDefault(model) = &actions[1] else {
        unreachable!()
    };
    coexist::restore_hermes_model(&sb.env, model).unwrap();
    assert_eq!(
        coexist::active_entry(&sb.env, Tool::Hermes)
            .unwrap()
            .as_deref(),
        Some("openrouter")
    );
    assert!(plan(&sb, &snapshot, Tool::Hermes).in_sync());
    // 普通的「存为场景」不带 model 段，编辑器里的默认选项仍然有效。
    let data = store::load(&sb.env).unwrap();
    assert!(scene::capture_tool(&sb.env, &data, Tool::Hermes)
        .unwrap()
        .unwrap()
        .extra
        .is_empty());
}

#[test]
fn scenes_with_only_unknown_tools_never_match() {
    let sb = setup();
    let mut future = named("Future", &[]);
    future
        .tools
        .insert("future-tool".into(), SceneTool::current("a"));
    assert!(!scene::matches(
        &sb.env,
        &store::load(&sb.env).unwrap(),
        &future
    ));
}

mod rules {
    use super::*;
    use leanflip_core::scene_rules::{self, Rule, RuleContext, SceneRule};
    use std::net::IpAddr;

    fn rule(value: serde_json::Value) -> SceneRule {
        serde_json::from_value(value).unwrap()
    }

    fn at(weekday: u8, time: &str, addresses: &[&str], hosts: &[&str], rule: &SceneRule) -> bool {
        let addresses: Vec<IpAddr> = addresses.iter().map(|a| a.parse().unwrap()).collect();
        let resolves = |host: &str| hosts.contains(&host);
        scene_rules::matches(
            rule,
            &RuleContext {
                weekday,
                minute: scene_rules::parse_time(time).unwrap(),
                addresses: &addresses,
                resolves: &resolves,
            },
        )
    }

    #[test]
    fn time_rules_cover_days_and_overnight_ranges() {
        let work = rule(
            json!({"kind": "time", "days": [1, 2, 3, 4, 5], "start": "09:00", "end": "18:00"}),
        );
        assert!(matches!(work, SceneRule::Known(Rule::Time { .. })));
        assert!(at(1, "09:00", &[], &[], &work));
        assert!(at(5, "17:59", &[], &[], &work));
        assert!(!at(5, "18:00", &[], &[], &work), "end is exclusive");
        assert!(!at(6, "10:00", &[], &[], &work));

        let night = rule(json!({"kind": "time", "days": [5], "start": "22:00", "end": "02:00"}));
        assert!(at(5, "23:30", &[], &[], &night));
        assert!(at(6, "01:59", &[], &[], &night), "continues into Saturday");
        assert!(!at(6, "22:30", &[], &[], &night));
        assert!(
            !at(5, "01:00", &[], &[], &night),
            "Thursday night is not listed"
        );
        let sunday = rule(json!({"kind": "time", "days": [7], "start": "23:00", "end": "01:00"}));
        assert!(at(1, "00:30", &[], &[], &sunday), "Sunday wraps to Monday");
    }

    #[test]
    fn network_rules_match_subnets_and_resolvable_hosts() {
        let office = rule(json!({"kind": "subnet", "cidr": "10.20.0.0/16"}));
        assert!(at(1, "12:00", &["192.168.1.5", "10.20.3.4"], &[], &office));
        assert!(!at(1, "12:00", &["10.21.0.1"], &[], &office));
        assert!(!at(1, "12:00", &["fd00::1"], &[], &office));
        let v6 = rule(json!({"kind": "subnet", "cidr": "fd00::/8"}));
        assert!(at(1, "12:00", &["fd12::1"], &[], &v6));
        let any = rule(json!({"kind": "subnet", "cidr": "0.0.0.0/0"}));
        assert!(at(1, "12:00", &["8.8.8.8"], &[], &any));

        let host = rule(json!({"kind": "host", "host": "intranet.corp.example"}));
        assert!(at(1, "12:00", &[], &["intranet.corp.example"], &host));
        assert!(!at(1, "12:00", &[], &[], &host));
    }

    #[test]
    fn malformed_rules_are_rejected_and_unknown_kinds_are_kept() {
        for bad in [
            json!({"kind": "time", "days": [], "start": "09:00", "end": "18:00"}),
            json!({"kind": "time", "days": [8], "start": "09:00", "end": "18:00"}),
            json!({"kind": "time", "days": [1], "start": "9:00", "end": "18:00"}),
            json!({"kind": "time", "days": [1], "start": "09:00", "end": "09:00"}),
            json!({"kind": "time", "days": [1], "start": "24:00", "end": "09:00"}),
            json!({"kind": "time", "days": [1]}),
            json!({"kind": "subnet", "cidr": "10.0.0.0"}),
            json!({"kind": "subnet", "cidr": "10.0.0.0/33"}),
            json!({"kind": "host", "host": "https://intranet"}),
            json!({"kind": "host", "host": "a..b"}),
            json!({"no": "kind"}),
        ] {
            assert!(scene_rules::validate(&rule(bad.clone())).is_err(), "{bad}");
        }
        let future = rule(json!({"kind": "bluetooth", "device": "desk"}));
        assert!(scene_rules::validate(&future).is_ok());
        assert!(!at(1, "12:00", &[], &[], &future));

        let sb = setup();
        let mut scene = named("Office", &[(Tool::Claude, SceneTool::current("a"))]);
        scene.rules = vec![
            rule(json!({"kind": "subnet", "cidr": "10.0.0.0/8"})),
            future.clone(),
        ];
        let saved = scene::save(&sb.env, scene).unwrap();
        let reloaded = store::load(&sb.env).unwrap();
        assert_eq!(
            reloaded.scenes[0].rules, saved.rules,
            "unknown rules survive a save"
        );
        let mut broken = saved.clone();
        broken
            .rules
            .push(rule(json!({"kind": "subnet", "cidr": "nope"})));
        assert!(matches!(
            scene::save(&sb.env, broken),
            Err(AppError::Localized {
                key: "scene.invalid",
                ..
            })
        ));
    }
}
