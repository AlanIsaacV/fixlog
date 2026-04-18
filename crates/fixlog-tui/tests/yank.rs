//! Integration tests for the yank (`yy` / `yY`) flow. We test the state
//! transitions — pending_prefix set on first `y`, cleared on completion —
//! but do not assert on `arboard` success since headless CI environments
//! often lack a clipboard. The code path either copies successfully or
//! writes a "clipboard unavailable" status; both outcomes are acceptable.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use fixlog_tui::TuiConfig;
use fixlog_tui::app::App;
use fixlog_tui::event::Event;

fn cfg() -> TuiConfig {
    TuiConfig {
        path: PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/real/fix44-om.log"
        )),
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

#[test]
fn first_y_sets_pending_prefix() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    assert!(app.state.pending_prefix.is_none());
    app.on_event(&press(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_eq!(app.state.pending_prefix, Some('y'));
}

#[test]
fn second_y_clears_prefix_and_sets_status() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.on_event(&press(KeyCode::Char('y'), KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.state.pending_prefix.is_none());
    // Status must be set either to "yanked raw" on success or
    // "clipboard unavailable" on headless CI.
    let t = app.state.status.text.to_lowercase();
    assert!(
        t.contains("yank") || t.contains("clipboard"),
        "unexpected status: {}",
        app.state.status.text
    );
}

#[test]
fn y_shift_y_copies_pretty() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.on_event(&press(KeyCode::Char('y'), KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Char('Y'), KeyModifiers::SHIFT));
    assert!(app.state.pending_prefix.is_none());
    let t = app.state.status.text.to_lowercase();
    assert!(t.contains("pretty") || t.contains("clipboard"), "got: {t}");
}

#[test]
fn unrelated_key_after_y_clears_prefix_and_processes_normally() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.state.cursor = 100;
    app.on_event(&press(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_eq!(app.state.pending_prefix, Some('y'));

    // Pressing `j` must clear the prefix and also perform the normal
    // cursor-down move.
    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    assert!(app.state.pending_prefix.is_none());
    assert_eq!(app.state.cursor, 101);
}

#[test]
fn capital_y_alone_warns_without_copying() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.on_event(&press(KeyCode::Char('Y'), KeyModifiers::SHIFT));
    // Pending prefix must not be set from a bare `Y`.
    assert!(app.state.pending_prefix.is_none());
    assert!(
        app.state.status.text.to_lowercase().contains("yy")
            || app.state.status.text.to_lowercase().contains("yank"),
        "got: {}",
        app.state.status.text
    );
}
