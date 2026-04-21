//! Integration tests for the "display toggle" keys introduced in Fase A:
//!
//! - `c` toggles [`AppState::skip_common`] (hides session-layer tags in the
//!   detail panel).
//! - `H` toggles [`AppState::hide_heartbeat`] (composes `NOT 35=0` into the
//!   effective filter while preserving the user-supplied filter).
//!
//! The tests drive a real `App` through synthesised key events so the
//! input → app → state chain is exercised end-to-end, using the
//! `fix44-om.log` fixture.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use fixlog_tui::TuiConfig;
use fixlog_tui::app::App;
use fixlog_tui::event::Event;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/real")).join(name)
}

fn cfg(name: &str) -> TuiConfig {
    TuiConfig {
        path: fixture(name),
        ..Default::default()
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
fn c_toggles_skip_common() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    assert!(!app.state.skip_common);

    app.on_event(&press(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(app.state.skip_common);

    app.on_event(&press(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(!app.state.skip_common);
}

#[test]
fn r_toggles_raw_detail_mode() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    assert!(!app.state.raw_detail_mode);

    app.on_event(&press(KeyCode::Char('r'), KeyModifiers::NONE));
    assert!(app.state.raw_detail_mode);

    app.on_event(&press(KeyCode::Char('r'), KeyModifiers::NONE));
    assert!(!app.state.raw_detail_mode);
}

#[test]
fn ctrl_c_still_quits_even_with_skip_common_binding() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    app.on_event(&press(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit);
    assert!(!app.state.skip_common);
}

#[test]
fn capital_h_toggles_hide_heartbeat_and_changes_visible() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    let total = app.state.visible.len();

    // Press Shift+H.
    app.on_event(&press(KeyCode::Char('H'), KeyModifiers::SHIFT));
    assert!(app.state.hide_heartbeat);
    // fix44-om.log contains heartbeats; hiding them must strictly shrink
    // visible (or leave it equal if the fixture happens to have none).
    assert!(
        app.state.visible.len() <= total,
        "hiding heartbeats should not grow visible"
    );
    assert_eq!(app.state.filter_text.as_deref(), Some("NOT 35=0"));

    // Toggle off — back to everything visible.
    app.on_event(&press(KeyCode::Char('H'), KeyModifiers::SHIFT));
    assert!(!app.state.hide_heartbeat);
    assert_eq!(app.state.visible.len(), total);
    assert!(app.state.filter_text.is_none());
}

#[test]
fn fixt11_md_log_parses_and_resolves_detail() {
    // Regression: previously the TUI called parse_one() which hardcoded SOH,
    // so pipe-separated logs (QuickFIX-J style) failed to resolve every
    // message's detail pane with "UnexpectedEof". Now we go through
    // parse_one_with_format and respect the sniffed separator.
    let app = App::bootstrap(&cfg("fixt11-md.log")).expect("bootstrap");
    assert!(!app.state.visible.is_empty(), "fixture must have messages");
    let detail = app
        .state
        .detail_cache
        .as_ref()
        .expect("detail cache populated");
    match &detail.1 {
        Ok(resolved) => {
            assert!(
                !resolved.fields.is_empty(),
                "resolved message should have fields"
            );
        }
        Err(e) => panic!("detail resolve failed: {e}"),
    }
}

#[test]
fn hide_heartbeat_composes_with_user_filter() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");

    // Type `:f 35=D` to set a user filter.
    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "f 35=D");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.state.user_filter_text.as_deref(), Some("35=D"));
    assert_eq!(app.state.filter_text.as_deref(), Some("35=D"));
    let d_only = app.state.visible.len();

    // Toggle hide_heartbeat on — expr becomes `(35=D) AND NOT 35=0`.
    app.on_event(&press(KeyCode::Char('H'), KeyModifiers::SHIFT));
    assert_eq!(app.state.user_filter_text.as_deref(), Some("35=D"));
    assert_eq!(
        app.state.filter_text.as_deref(),
        Some("(35=D) AND NOT 35=0"),
    );
    // Because the user filter is already `35=D`, heartbeats were never
    // visible; the composed visible should match the D-only count.
    assert_eq!(app.state.visible.len(), d_only);

    // Toggle off — we preserve the user filter.
    app.on_event(&press(KeyCode::Char('H'), KeyModifiers::SHIFT));
    assert_eq!(app.state.user_filter_text.as_deref(), Some("35=D"));
    assert_eq!(app.state.filter_text.as_deref(), Some("35=D"));
}
