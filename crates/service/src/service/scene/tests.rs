use super::*;
use serde_json::json;

struct Home {
    _dir: tempfile::TempDir,
    service: Service,
}

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
        id.to_uppercase(),
        json!({"npm": "@ai-sdk/openai-compatible", "name": id,
               "options": {"baseURL": format!("https://{id}"), "apiKey": "sk"}, "models": {"m": {}}}),
    )
}

/// Claude 用 a（还有 b），OpenCode 只启用 x（还有 y）。
fn home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let env = Env::with_home(dir.path());
    std::fs::create_dir_all(&env.opencode_dir).unwrap();
    std::fs::write(
        env.opencode_dir.join("opencode.json"),
        "{\"provider\": {}}\n",
    )
    .unwrap();
    let service = Service::start(env, None).unwrap();
    for id in ["a", "b"] {
        service
            .add_provider(Tool::Claude, claude(id), false)
            .unwrap();
    }
    for id in ["x", "y"] {
        service
            .add_provider(Tool::OpenCode, opencode(id), id == "x")
            .unwrap();
    }
    service.switch_provider(Tool::Claude, "a").unwrap();
    Home { _dir: dir, service }
}

fn scene_b(service: &Service) -> Scene {
    let mut scene = Scene {
        name: "B".into(),
        ..Scene::default()
    };
    scene.tools.insert("claude".into(), SceneTool::current("b"));
    scene
        .tools
        .insert("opencode".into(), SceneTool::enabled(["y"]));
    service.save_scene(scene).unwrap()
}

fn state(service: &Service) -> (Option<String>, Vec<String>) {
    (
        service.list_providers(Tool::Claude).unwrap().current,
        service.list_providers(Tool::OpenCode).unwrap().in_live,
    )
}

fn view(service: &Service, id: &str) -> SceneView {
    service
        .scenes()
        .unwrap()
        .scenes
        .into_iter()
        .find(|view| view.scene.id == id)
        .unwrap()
}

#[test]
fn preview_names_each_step_without_writing() {
    let home = home();
    let b = scene_b(&home.service);
    let preview = home.service.scene_preview("b").unwrap();
    assert_eq!(preview.scene.id, b.id);
    let claude = &preview.tools[0];
    assert_eq!(claude.tool, Tool::Claude);
    assert_eq!(claude.from.as_deref(), Some("A"));
    assert_eq!(
        claude.steps,
        [SceneStep {
            action: Action::Switch("b".into()),
            name: "B".into()
        }]
    );
    assert_eq!(
        serde_json::to_value(&claude.steps[0]).unwrap(),
        json!({"action": {"kind": "switch", "id": "b"}, "name": "B"}),
        "wire format the desktop UI reads"
    );
    let opencode = &preview.tools[1];
    assert_eq!(opencode.kind, ToolKind::Coexist);
    assert_eq!(
        opencode
            .steps
            .iter()
            .map(|step| (&step.action, step.name.as_str()))
            .collect::<Vec<_>>(),
        [
            (&Action::Disable("x".into()), "X"),
            (&Action::Enable("y".into()), "Y")
        ]
    );
    assert_eq!(state(&home.service), (Some("a".into()), vec!["x".into()]));
    assert_eq!(
        home.service.scene_preview("nope").unwrap_err().code(),
        "scene.not_found"
    );
}

#[test]
fn apply_switches_every_tool_and_undo_restores_the_previous_state() {
    let home = home();
    let b = scene_b(&home.service);
    assert_eq!(view(&home.service, &b.id).status, SceneStatus::Idle);

    let report = home.service.apply_scene(&b.id, None).unwrap();
    assert!(!report.failed());
    assert_eq!(
        report
            .results
            .iter()
            .map(|r| (r.tool, r.outcome))
            .collect::<Vec<_>>(),
        [
            (Tool::Claude, SceneOutcome::Applied),
            (Tool::OpenCode, SceneOutcome::Applied)
        ]
    );
    assert!(report.results.iter().all(|r| !r.changed.is_empty()));
    assert_eq!(state(&home.service), (Some("b".into()), vec!["y".into()]));
    let list = home.service.scenes().unwrap();
    assert_eq!(list.last.as_deref(), Some(b.id.as_str()));
    assert_eq!(list.scenes[0].status, SceneStatus::Active);
    assert_eq!(report.undo.last, None);

    // 再应用一次什么都不用做。
    let again = home.service.apply_scene("B", None).unwrap();
    assert!(again
        .results
        .iter()
        .all(|r| r.outcome == SceneOutcome::Unchanged && r.changed.is_empty()));

    let undone = home.service.undo_scene(report.undo).unwrap();
    assert!(!undone.failed());
    assert_eq!(state(&home.service), (Some("a".into()), vec!["x".into()]));
    let list = home.service.scenes().unwrap();
    assert_eq!(list.last, None);
    assert_eq!(list.scenes[0].status, SceneStatus::Idle);

    // 撤销的报告里还有一份快照，可以重做。
    home.service.undo_scene(undone.undo).unwrap();
    assert_eq!(state(&home.service), (Some("b".into()), vec!["y".into()]));
    assert_eq!(
        home.service.scenes().unwrap().last.as_deref(),
        Some(b.id.as_str())
    );
}

#[test]
fn a_manual_switch_marks_the_last_scene_drifted() {
    let home = home();
    let b = scene_b(&home.service);
    home.service.apply_scene(&b.id, None).unwrap();
    home.service.switch_provider(Tool::Claude, "a").unwrap();
    let drifted = view(&home.service, &b.id);
    assert_eq!(drifted.status, SceneStatus::Drifted);
    assert_eq!(drifted.pending, [Tool::Claude]);

    let mut other = Scene {
        name: "Only Claude A".into(),
        ..Scene::default()
    };
    other.tools.insert("claude".into(), SceneTool::current("a"));
    let other = home.service.save_scene(other).unwrap();
    assert_eq!(view(&home.service, &other.id).status, SceneStatus::Active);

    home.service.delete_scene(&b.id).unwrap();
    assert_eq!(home.service.scenes().unwrap().last, None);
}

#[test]
fn one_failing_tool_does_not_stop_the_others() {
    let home = home();
    let b = scene_b(&home.service);
    let env = home.service.env();
    std::fs::write(env.opencode_dir.join("opencode.json"), "{ not json").unwrap();
    let report = home.service.apply_scene(&b.id, None).unwrap();
    assert!(report.failed());
    let opencode = report
        .results
        .iter()
        .find(|r| r.tool == Tool::OpenCode)
        .unwrap();
    assert_eq!(opencode.outcome, SceneOutcome::Failed);
    assert!(opencode.error.is_some());
    assert_eq!(
        home.service.list_providers(Tool::Claude).unwrap().current,
        Some("b".into())
    );
    // 读不了的工具没有快照，撤销只回滚读得到的。
    assert!(report.undo.tools.contains_key("claude"));
    assert!(!report.undo.tools.contains_key("opencode"));
    assert_eq!(view(&home.service, &b.id).status, SceneStatus::Drifted);
}

#[test]
fn chosen_tools_and_missing_cards_limit_what_is_applied() {
    let home = home();
    let b = scene_b(&home.service);
    let report = home
        .service
        .apply_scene(&b.id, Some(vec![Tool::Claude]))
        .unwrap();
    assert_eq!(report.results.len(), 1);
    assert_eq!(state(&home.service), (Some("b".into()), vec!["x".into()]));

    home.service.delete_provider(Tool::OpenCode, "y").unwrap();
    let report = home.service.apply_scene(&b.id, None).unwrap();
    let opencode = report
        .results
        .iter()
        .find(|r| r.tool == Tool::OpenCode)
        .unwrap();
    assert_eq!(opencode.outcome, SceneOutcome::Applied);
    assert_eq!(opencode.missing, ["y"]);
    assert!(home
        .service
        .list_providers(Tool::OpenCode)
        .unwrap()
        .in_live
        .is_empty());
    assert_eq!(
        view(&home.service, &b.id).missing,
        [SceneMissing {
            tool: Tool::OpenCode,
            id: "y".into()
        }]
    );
}

#[test]
fn capture_skips_tools_without_cards_and_rejects_bad_undo_snapshots() {
    let home = home();
    let captured = home.service.scene_capture(None).unwrap();
    assert_eq!(
        captured.tools.keys().collect::<Vec<_>>(),
        ["claude", "opencode"]
    );
    assert_eq!(captured.tools["opencode"], SceneTool::enabled(["x"]));
    let only = home.service.scene_capture(Some(vec![Tool::Codex])).unwrap();
    assert!(only.tools.is_empty());

    let mut bad = SceneUndo::default();
    bad.tools.insert("claude".into(), SceneTool::enabled(["a"]));
    assert_eq!(
        home.service.undo_scene(bad).unwrap_err().code(),
        "scene.invalid"
    );
    assert!(home
        .service
        .undo_scene(SceneUndo::default())
        .unwrap()
        .results
        .is_empty());
}
