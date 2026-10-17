use super::*;
use leanflip_core::Env;
use leanflip_service::Service;
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
            // Lock files record pid + timestamp on every acquisition; they are not data.
            if path.is_dir() && path.file_name() == Some("locks".as_ref())
                || path.extension() == Some("lock".as_ref())
            {
                continue;
            }
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
    if let Ok(destination) = std::env::var("LEANFLIP_CAPTURE") {
        let color = |color: Color, fallback: &str| match color {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            _ => fallback.to_string(),
        };
        let cells=(0..height).flat_map(|y|(0..width).map(move|x|(x,y))).map(|(x,y)| {let cell=&buffer[(x,y)];json!({"x":x,"y":y,"text":cell.symbol(),"fg":color(cell.fg,"#ebe7e0"),"bg":color(cell.bg,"#1c1b1a"),"bold":cell.modifier.contains(Modifier::BOLD)})}).collect::<Vec<_>>();
        let name = std::thread::current()
            .name()
            .unwrap_or("screen")
            .replace("::", "-");
        fs::create_dir_all(&destination).unwrap();
        let context = if app.editor.is_some() {
            "editor"
        } else if app.data.is_some() {
            "data"
        } else {
            "workspace"
        };
        fs::write(
            Path::new(&destination).join(format!("{name}-{context}-{width}x{height}.json")),
            serde_json::to_vec(&json!({"width":width,"height":height,"cells":cells})).unwrap(),
        )
        .unwrap();
    }

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
    let service = service(dir.path());
    let settings = provider("alpha", Tool::OpenCode).settings_config;
    let review = service
        .credential_commands(Tool::OpenCode, &settings)
        .unwrap();
    service
        .approve_credential_commands(Tool::OpenCode, &settings, &review.fingerprint)
        .unwrap();
    let mut app = App::new(Some(dir.path()), Language::En, Some(Tool::OpenCode));
    for enabled in [true, false] {
        app.prepare(false);
        let pending = app
            .key(key(KeyCode::Char(if enabled { 'y' } else { 'Y' })))
            .unwrap();
        app.apply(pending);
        assert_eq!(
            service
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
                "LeanFlip"
            }));
            if width >= 110 {
                assert!(screen.contains(lang.text("配置路径", "Configuration paths")));
            }
            if let Ok(output) = std::env::var("LEANFLIP_TUI_SNAPSHOTS") {
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

#[test]
fn data_center_detects_and_migrates_only_after_confirmation() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join(".cc-switch");
    fs::create_dir(&source).unwrap();
    let file = source.join("config.json");
    fs::write(&file, json!({"claude":{"providers":{"relay":{"name":"Migration fixture","settingsConfig":{"env":{"ANTHROPIC_AUTH_TOKEN":"hidden-migration-secret"}}}}}}).to_string()).unwrap();
    let mut app = App::new(Some(dir.path()), Language::En, None);
    app.key(key(KeyCode::Char('b')));
    assert!(render(&mut app, 100, 26).contains("Data center"));
    let before = snapshot(dir.path());
    app.key(key(KeyCode::Enter));
    app.data.as_mut().unwrap().run_pending();
    assert_eq!(snapshot(dir.path()), before);
    assert!(render(&mut app, 100, 26).contains("Migration fixture"));
    assert!(!render(&mut app, 100, 26).contains("hidden-migration-secret"));
    app.key(key(KeyCode::Enter));
    app.key(key(KeyCode::Enter)); // default cancel
    app.data.as_mut().unwrap().run_pending();
    assert_eq!(snapshot(dir.path()), before);
    app.key(key(KeyCode::Enter));
    app.key(key(KeyCode::Char('y')));
    assert!(app.data.as_mut().unwrap().run_pending());
    assert!(render(&mut app, 100, 26).contains("Providers saved 1"));
    assert!(!service(dir.path()).env().claude_settings_path().exists());
    assert_eq!(fs::read(file.clone()).unwrap(), before[&file]);
}

#[test]
fn backup_form_masks_secrets_at_every_size_and_cancel_does_not_export() {
    let dir = TempDir::new().unwrap();
    let mut app = App::new(Some(dir.path()), Language::En, None);
    app.key(key(KeyCode::Char('b')));
    app.key(key(KeyCode::Down));
    app.key(key(KeyCode::Down));
    app.key(key(KeyCode::Enter));
    app.key(key(KeyCode::Tab));
    for ch in "private-passphrase-fixture".chars() {
        app.key(key(KeyCode::Char(ch)));
    }
    for (width, height) in [(44, 12), (80, 24), (120, 30)] {
        let screen = render(&mut app, width, height);
        assert!(!screen.contains("private-passphrase-fixture"));
        assert!(screen.contains('*'));
        if std::env::var_os("LEANFLIP_TUI_SNAPSHOTS").is_some() {
            let destination =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tui-preview");
            fs::create_dir_all(&destination).unwrap();
            fs::write(
                destination.join(format!("data-export-{width}x{height}.txt")),
                screen,
            )
            .unwrap();
        }
    }
    app.key(key(KeyCode::Esc));
    assert!(snapshot(dir.path()).is_empty());
}

#[test]
fn connection_editor_preserves_credentials_until_explicit_save_and_supports_paste() {
    let dir = fixture(Tool::Claude);
    let mut app = App::new(Some(dir.path()), Language::En, Some(Tool::Claude));
    let before = snapshot(dir.path());
    app.key(key(KeyCode::Char('e')));
    app.key(key(KeyCode::Tab));
    app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.paste("https://changed.example/v1");
    app.key(key(KeyCode::Tab));
    app.paste("fixture-new-private-key\n");
    for (width, height) in [(44, 12), (52, 16), (80, 24), (120, 28)] {
        let screen = render(&mut app, width, height);
        assert!(!screen.contains("fixture-new-private-key"));
        assert!(!screen.contains("fixture-hidden-credential"));
        assert!(screen.contains('*'));
    }
    app.key(key(KeyCode::Enter)); // review
    assert_eq!(snapshot(dir.path()), before);
    let narrow = render(&mut app, 44, 16);
    assert!(narrow.contains("Cancel") && narrow.contains("Apply Y"));
    app.key(key(KeyCode::Enter)); // defaults to cancel, keeps the editor open
    assert!(app.editor.is_some());
    assert_eq!(snapshot(dir.path()), before);
    app.key(key(KeyCode::Enter));
    app.key(key(KeyCode::Char('y')));
    assert!(app
        .editor
        .as_mut()
        .unwrap()
        .save(Some(dir.path()), Language::En)
        .unwrap()
        .contains("saved"));
    let provider = service(dir.path())
        .list_providers(Tool::Claude)
        .unwrap()
        .providers
        .remove(0);
    assert_eq!(
        provider.settings_config["env"]["ANTHROPIC_AUTH_TOKEN"],
        "fixture-new-private-key"
    );
    assert_eq!(
        provider.settings_config["env"]["ANTHROPIC_BASE_URL"],
        "https://changed.example/v1"
    );
}

#[test]
fn endpoints_are_visible_and_searchable_without_url_secrets_or_api_keys() {
    let dir = fixture(Tool::Claude);
    let service = service(dir.path());
    let mut provider = provider("beta", Tool::Claude);
    provider.settings_config["env"]["ANTHROPIC_BASE_URL"] =
        json!("https://user:url-private-token@special.example/v1?secret=private-query");
    service.update_provider(Tool::Claude, provider).unwrap();
    let mut app = App::new(Some(dir.path()), Language::En, Some(Tool::Claude));
    app.key(key(KeyCode::Char('/')));
    app.paste("special.example");
    app.key(key(KeyCode::Enter));
    assert_eq!(app.selected().unwrap().id, "beta");
    let wide = render(&mut app, 120, 30);
    assert!(wide.contains("special.example"));
    assert!(wide.contains("API Key"));
    for secret in ["fixture-hidden", "url-private-token", "private-query"] {
        assert!(!wide.contains(secret));
    }
    app.key(key(KeyCode::Char('e')));
    app.key(key(KeyCode::Tab));
    let form = render(&mut app, 80, 24);
    assert!(!form.contains("url-private-token"));
}

#[test]
fn invalid_url_keeps_the_masked_key_for_a_corrected_retry_without_writing() {
    let dir = fixture(Tool::Claude);
    let service = service(dir.path());
    let provider = service.list_providers(Tool::Claude).unwrap().providers[0].clone();
    let mut draft = crate::editor::Draft::new(Tool::Claude, provider);
    draft.url = "not-a-url".into();
    draft.key = "private-retry-fixture".into();
    let before = snapshot(dir.path());
    assert!(draft.save(&service, Language::En).is_err());
    assert_eq!(draft.key, "private-retry-fixture");
    assert_eq!(snapshot(dir.path()), before);
    draft.url = "https://corrected.example/v1".into();
    assert!(draft.save(&service, Language::En).is_ok());
    assert!(draft.key.is_empty());
}

#[test]
fn connection_editor_rejects_paths_changed_after_review() {
    let dir = fixture(Tool::Claude);
    let mut app = App::new(Some(dir.path()), Language::En, Some(Tool::Claude));
    app.key(key(KeyCode::Char('e')));
    app.paste(" updated");
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    app.key(key(KeyCode::Char('y')));
    fs::write(
        dir.path().join(".leanflip/app-settings.json"),
        serde_json::to_vec(&json!({
            "dirOverrides": {"claudeDir": dir.path().join("new-client")}
        }))
        .unwrap(),
    )
    .unwrap();
    let before = snapshot(dir.path());
    assert!(app
        .editor
        .as_mut()
        .unwrap()
        .save(Some(dir.path()), Language::En)
        .is_none());
    assert_eq!(snapshot(dir.path()), before);
    assert!(render(&mut app, 120, 28).contains("Configuration paths changed"));
}

#[test]
fn creation_wizard_masks_key_previews_and_defaults_to_no_activation() {
    let dir = TempDir::new().unwrap();
    let mut app = App::new(Some(dir.path()), Language::En, Some(Tool::Claude));
    app.key(key(KeyCode::Char('n')));
    app.paste("Work");
    app.key(key(KeyCode::Tab));
    app.paste("https://example.com");
    app.key(key(KeyCode::Tab));
    app.paste("fixture-new-card-key");
    for (width, height) in [(44, 12), (80, 24), (120, 28)] {
        assert!(!render(&mut app, width, height).contains("fixture-new-card-key"));
    }
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(snapshot(dir.path()).is_empty());
    let screen = render(&mut app, 120, 28);
    assert!(screen.contains("without activation"));
    assert!(screen.contains("API Key: Entered (hidden)"));
    app.key(key(KeyCode::Enter)); // cancel is default
    assert!(snapshot(dir.path()).is_empty());
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    app.key(key(KeyCode::Char('y')));
    app.editor
        .as_mut()
        .unwrap()
        .save(Some(dir.path()), Language::En)
        .unwrap();
    let saved = service(dir.path()).list_providers(Tool::Claude).unwrap();
    assert_eq!(saved.providers[0].id, "work");
    assert_eq!(
        saved.providers[0].settings_config["env"]["ANTHROPIC_AUTH_TOKEN"],
        "fixture-new-card-key"
    );
    assert!(saved.current.is_none());
    assert!(!dir.path().join(".claude/settings.json").exists());
}

#[test]
fn wizard_templates_can_save_and_activate_each_supported_tool_without_overwriting_duplicate_ids() {
    for tool in Tool::ALL {
        let dir = TempDir::new().unwrap();
        let service = service(dir.path());
        let mut draft = crate::editor::Draft::creation(tool);
        draft.name = "Fixture".into();
        draft.url = "https://example.com/v1".into();
        draft.key = "fixture-wizard-key".into();
        draft.model = "fixture-model".into();
        draft.activate = true;
        if tool == Tool::OpenCode {
            assert!(draft.save(&service, Language::En).is_err());
            assert!(!service.env().opencode_dir.join("opencode.json").exists());
            let settings = crate::creation::provider(&draft)
                .unwrap_or_else(|e| panic!("{}", e.message))
                .settings_config;
            let review = service.credential_commands(tool, &settings).unwrap();
            service
                .approve_credential_commands(tool, &settings, &review.fingerprint)
                .unwrap();
        }
        draft
            .save(&service, Language::En)
            .unwrap_or_else(|e| panic!("{tool}: {}", e.message));
        let stored = leanflip_core::store::load_readonly(&service.env()).unwrap();
        assert!(stored.provider(tool, "custom").is_some());
        assert!(
            service
                .env()
                .live_paths(tool)
                .iter()
                .any(|path| path.exists()),
            "{tool}"
        );
        let before = snapshot(dir.path());
        draft.name = "Must not overwrite".into();
        assert!(draft.save(&service, Language::En).is_err());
        assert_eq!(snapshot(dir.path()), before);
    }
}
