use super::*;
use ccsl_core::Env;
use ccsl_service::Service;
use ratatui::{backend::TestBackend, Terminal};
use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

fn service(home: &Path) -> Service {
    Service::open(Env::with_home(home), None).unwrap()
}

fn provider(id: &str, tool: Tool) -> Provider {
    serde_json::from_value(json!({
        "id": id, "name": "Shared name 工作配置",
        "settingsConfig": if tool.is_coexist() {
            json!({"npm":"@ai-sdk/openai-compatible", "options":{"baseURL":"https://example.com", "apiKey":"fixture-hidden-credential"}, "models":{"model":{}}})
        } else {
            json!({"env":{"ANTHROPIC_BASE_URL":"https://example.com", "ANTHROPIC_AUTH_TOKEN":"fixture-hidden-credential"}})
        },
        "meta": {"private":"fixture-hidden-metadata"}
    })).unwrap()
}

fn fixture(tool: Tool) -> TempDir {
    let dir = TempDir::new().unwrap();
    let service = service(dir.path());
    for id in ["alpha", "beta"] {
        service
            .add_provider(tool, provider(id, tool), false)
            .unwrap();
    }
    if !tool.is_coexist() {
        service.switch_provider(tool, "alpha").unwrap();
    }
    dir
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(path: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        if !path.exists() {
            return;
        }
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, result);
            } else {
                result.insert(path.clone(), fs::read(path).unwrap());
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, &mut result);
    result
}

fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            let mut line = String::new();
            let mut x = 0;
            while x < width {
                let symbol = buffer[(x, y)].symbol();
                line.push_str(symbol);
                x += unicode_width::UnicodeWidthStr::width(symbol).max(1) as u16;
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn browsing_empty_home_and_searching_never_write_configuration() {
    let dir = TempDir::new().unwrap();
    let home = dir.path().join("not-created");
    let mut app = App::new(Some(&home), Language::En, Some(Tool::Codex));
    assert!(render(&mut app, 80, 24).contains("No providers yet"));
    for code in [
        KeyCode::Char('/'),
        KeyCode::Char('x'),
        KeyCode::Enter,
        KeyCode::Char('r'),
        KeyCode::Esc,
        KeyCode::Char('?'),
        KeyCode::Esc,
        KeyCode::Char('p'),
        KeyCode::Enter,
    ] {
        app.key(key(code));
    }
    assert!(!home.exists());
}

#[test]
fn confirmation_defaults_to_cancel_and_applies_the_exact_provider_id() {
    let dir = fixture(Tool::Claude);
    let mut app = App::new(Some(dir.path()), Language::Zh, Some(Tool::Claude));
    app.key(key(KeyCode::Down));
    let before = snapshot(dir.path());
    app.prepare(false);
    assert!(app.dialog.as_ref().unwrap().body.contains("ID: beta"));
    assert!(app.key(key(KeyCode::Enter)).is_none());
    assert!(app.dialog.is_none());
    assert_eq!(snapshot(dir.path()), before);
    app.prepare(false);
    app.key(key(KeyCode::Tab));
    let pending = app.key(key(KeyCode::Enter)).unwrap();
    app.apply(pending);
    assert_eq!(
        service(dir.path())
            .list_providers(Tool::Claude)
            .unwrap()
            .current
            .as_deref(),
        Some("beta")
    );
    assert!(app.notice.contains("供应商已切换"));
}

#[test]
fn changed_confirmation_is_rejected_without_overwriting_external_edits() {
    let dir = fixture(Tool::Claude);
    let mut app = App::new(Some(dir.path()), Language::En, Some(Tool::Claude));
    app.move_selection(1);
    app.prepare(false);
    let pending = app.key(key(KeyCode::Char('y'))).unwrap();
    let mut changed = provider("beta", Tool::Claude);
    changed.name = "Changed elsewhere".into();
    service(dir.path())
        .update_provider(Tool::Claude, changed)
        .unwrap();
    let before = snapshot(dir.path());
    app.apply(pending);
    assert_eq!(snapshot(dir.path()), before);
    assert!(app.dialog.unwrap().body.contains("conflict"));
}

#[test]
fn coexist_toggles_and_unreadable_state_keep_the_native_safety_contract() {
    let dir = fixture(Tool::OpenCode);
    let mut app = App::new(Some(dir.path()), Language::En, Some(Tool::OpenCode));
    for enabled in [true, false] {
        app.prepare(false);
        let pending = app
            .key(key(KeyCode::Char(if enabled { 'y' } else { 'Y' })))
            .unwrap();
        app.apply(pending);
        assert_eq!(
            service(dir.path())
                .list_providers(Tool::OpenCode)
                .unwrap()
                .in_live
                .contains(&"alpha".into()),
            enabled
        );
    }
    let path = app.list.as_ref().unwrap().paths[0].clone();
    fs::write(path, "{broken fixture-hidden-credential").unwrap();
    let before = snapshot(dir.path());
    app.refresh();
    app.prepare(false);
    assert!(app.dialog.is_none());
    assert!(app.notice.contains("unreadable"));
    assert!(render(&mut app, 120, 28).contains("Unknown"));
    assert!(!render(&mut app, 120, 28).contains("fixture-hidden"));
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn filters_do_not_turn_an_empty_selection_into_an_action() {
    let dir = fixture(Tool::Claude);
    let mut app = App::new(Some(dir.path()), Language::En, Some(Tool::Claude));
    app.key(key(KeyCode::Char('/')));
    for ch in "beta".chars() {
        app.key(key(KeyCode::Char(ch)));
    }
    app.key(key(KeyCode::Enter));
    assert_eq!(app.selected().unwrap().id, "beta");
    app.prepare(false);
    assert!(app.dialog.as_ref().unwrap().body.contains("ID: beta"));
    app.key(key(KeyCode::Esc));
    app.key(key(KeyCode::Char('/')));
    app.key(key(KeyCode::Char('x')));
    app.key(key(KeyCode::Enter));
    let before = snapshot(dir.path());
    app.prepare(false);
    assert!(app.dialog.is_none());
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn responsive_views_and_scrollable_details_do_not_expose_configuration() {
    let dir = fixture(Tool::Claude);
    for lang in [Language::Zh, Language::En] {
        let mut app = App::new(Some(dir.path()), lang, Some(Tool::Claude));
        for (width, height) in [(120, 28), (80, 24), (52, 16), (30, 8)] {
            let screen = render(&mut app, width, height);
            assert!(!screen.contains("fixture-hidden"));
            assert!(screen.contains(if width < 44 {
                lang.text("窗口太小", "too small")
            } else {
                "CC Switch Lite"
            }));
            if width >= 110 {
                assert!(screen.contains(lang.text("配置路径", "Configuration paths")));
            }
            if let Ok(output) = std::env::var("CCSL_TUI_SNAPSHOTS") {
                fs::create_dir_all(&output).unwrap();
                fs::write(
                    Path::new(&output)
                        .join(format!("{}-{width}x{height}.txt", lang.text("zh", "en"))),
                    screen,
                )
                .unwrap();
            }
        }
    }
    let mut app = App::new(Some(dir.path()), Language::En, Some(Tool::Claude));
    app.key(key(KeyCode::Char('p')));
    let dialog = app.dialog.as_mut().unwrap();
    dialog
        .body
        .push_str(&format!("\n{}\nLAST-LINE", "long-id".repeat(80)));
    render(&mut app, 52, 16);
    assert!(app.dialog.as_ref().unwrap().max_scroll > 0);
    app.key(key(KeyCode::End));
    assert!(render(&mut app, 52, 16).contains("LAST-LINE"));
}
