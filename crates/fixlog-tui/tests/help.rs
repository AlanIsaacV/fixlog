//! Integration tests for the `:help` overlay wired in Fase 5 docs refresh.
//! Drives `App` through the full input → command → state chain.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use fixlog_tui::TuiConfig;
use fixlog_tui::app::App;
use fixlog_tui::event::Event;
use fixlog_tui::state::{InputMode, Overlay};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/real")).join(name)
}

fn cfg(name: &str) -> TuiConfig {
    TuiConfig {
        path: fixture(name),
        follow: false,
        initial_filter: None,
    }
}

fn press(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        app.on_event(&press(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

#[test]
fn colon_help_opens_help_overlay() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    assert!(app.state.overlay.is_none());

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "help");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.state.input_mode, InputMode::Normal);
    assert!(
        matches!(app.state.overlay, Some(Overlay::Help { scroll: 0 })),
        "expected Help overlay, got {:?}",
        app.state.overlay
    );
}

#[test]
fn colon_h_short_alias_opens_help() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "h");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));

    assert!(matches!(app.state.overlay, Some(Overlay::Help { .. })));
}

#[test]
fn j_k_scrolls_help_overlay() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "help");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));

    // Scroll down three times.
    for _ in 0..3 {
        app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    }
    match app.state.overlay {
        Some(Overlay::Help { scroll }) => assert_eq!(scroll, 3),
        _ => panic!("overlay should still be Help"),
    }

    // Scroll back up one.
    app.on_event(&press(KeyCode::Char('k'), KeyModifiers::NONE));
    match app.state.overlay {
        Some(Overlay::Help { scroll }) => assert_eq!(scroll, 2),
        _ => panic!("overlay should still be Help"),
    }
}

#[test]
fn g_and_capital_g_jump_to_top_and_bottom_of_help() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "help");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));

    // G = jump to bottom (last line).
    app.on_event(&press(KeyCode::Char('G'), KeyModifiers::SHIFT));
    let expected_bottom =
        u16::try_from(fixlog_tui::view::help::content_len().saturating_sub(1)).unwrap();
    match app.state.overlay {
        Some(Overlay::Help { scroll }) => assert_eq!(scroll, expected_bottom),
        _ => panic!("overlay should still be Help after G"),
    }

    // g = jump to top (scroll 0).
    app.on_event(&press(KeyCode::Char('g'), KeyModifiers::NONE));
    match app.state.overlay {
        Some(Overlay::Help { scroll }) => assert_eq!(scroll, 0),
        _ => panic!("overlay should still be Help after g"),
    }
}

#[test]
fn esc_closes_help_overlay() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "help");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.state.overlay.is_some());

    app.on_event(&press(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.state.overlay.is_none());
    assert_eq!(app.state.input_mode, InputMode::Normal);
}

#[test]
fn help_overlay_renders_without_panic() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "help");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
    terminal
        .draw(|frame| fixlog_tui::draw(frame, &mut app))
        .expect("render");

    // Scrape the buffer and assert a keybinding appears somewhere.
    let buf = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            text.push_str(buf[(x, y)].symbol());
        }
        text.push('\n');
    }
    assert!(
        text.contains("MODES") || text.contains("NAVIGATION"),
        "rendered help overlay should show at least one section header; got:\n{text}"
    );
}
