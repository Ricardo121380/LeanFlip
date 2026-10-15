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
