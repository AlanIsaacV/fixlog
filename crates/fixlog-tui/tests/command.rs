//! Integration tests for the command bar flow: typing `:filter <expr>` and
//! submitting it should update `visible` and reset cursor/viewport.
//!
//! Drives `App` via synthesised key events so the full `input.rs` →
//! `app.rs` → `command.rs` → `state.rs` chain is exercised the way a user
//! would in the real TUI.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use fixlog_tui::TuiConfig;
use fixlog_tui::app::App;
use fixlog_tui::event::Event;
use fixlog_tui::state::InputMode;

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
fn colon_enters_command_mode() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    assert_eq!(app.state.input_mode, InputMode::Normal);

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    assert_eq!(app.state.input_mode, InputMode::Command);
    assert!(app.state.command_buffer.is_empty());
}

#[test]
fn typing_and_submitting_filter_reduces_visible() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    let total = app.state.visible.len();
    assert_eq!(total, 5419);

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "filter 35=D");
    assert_eq!(app.state.command_buffer, "filter 35=D");

    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.state.input_mode, InputMode::Normal);
    assert!(app.state.command_buffer.is_empty());
    assert!(
        app.state.visible.len() < total,
        "filter should reduce visible count (was {total}, now {})",
        app.state.visible.len()
    );
    assert!(!app.state.visible.is_empty(), "35=D must match something");
    assert!(app.state.command_history.last().map(String::as_str) == Some("filter 35=D"));
}

#[test]
fn submitting_q_quits() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    assert!(!app.should_quit);

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "q");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));

    assert!(app.should_quit);
}

#[test]
fn esc_cancels_command_mode_without_executing() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    let visible_before = app.state.visible.len();

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "filter 35=D");
    app.on_event(&press(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(app.state.input_mode, InputMode::Normal);
    assert!(app.state.command_buffer.is_empty());
    assert_eq!(
        app.state.visible.len(),
        visible_before,
        "filter should not be applied after Esc"
    );
}

#[test]
fn backspace_trims_command_buffer() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "quz");
    app.on_event(&press(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(app.state.command_buffer, "qu");
}

#[test]
fn history_up_recalls_last_command() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    // First command: apply a filter.
    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "filter 35=D");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));

    // Open command bar again and hit Up.
    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.state.command_buffer, "filter 35=D");
}

#[test]
fn live_preview_updates_visible_while_typing() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    let total = app.state.visible.len();

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    // After typing `filter ` (space at end) the live helper clears filter (expr empty).
    type_str(&mut app, "filter ");
    assert_eq!(app.state.visible.len(), total);

    // As we add an expression, visible should drop.
    type_str(&mut app, "35=D");
    let filtered = app.state.visible.len();
    assert!(
        filtered < total && filtered > 0,
        "expected partial filter, got {filtered}/{total}"
    );
}

#[test]
fn esc_during_live_preview_restores_previous_filter() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");

    // Apply a filter first.
    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "filter 35=D");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));
    let committed = app.state.visible.len();

    // Open command bar, start typing a different filter, then Esc.
    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "filter 35=0");
    let previewed = app.state.visible.len();
    assert_ne!(previewed, committed, "preview should have changed visible");

    app.on_event(&press(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(
        app.state.visible.len(),
        committed,
        "Esc should roll back to the committed filter"
    );
}

fn backspace(app: &mut App) {
    app.on_event(&press(KeyCode::Backspace, KeyModifiers::NONE));
}

#[test]
fn backspace_during_live_preview_preserves_last_valid_preview() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    let total = app.state.visible.len();

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "filter 35=D");
    let filtered = app.state.visible.len();
    assert!(filtered < total);

    // Backspace through the value+op: buffer walks `filter 35=`, `filter 35`,
    // `filter 3` — all invalid queries. The preview must freeze at the last
    // valid state (35=D).
    for _ in 0..3 {
        backspace(&mut app);
    }
    assert_eq!(
        app.state.visible.len(),
        filtered,
        "partial expression should not overwrite the previous preview"
    );

    // One more backspace → buffer becomes `filter ` (empty expr) and the
    // preview clears to "no filter".
    backspace(&mut app);
    assert_eq!(app.state.visible.len(), total);
}

#[test]
fn invalid_filter_shows_error_and_keeps_visible_intact() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    let visible_before = app.state.visible.len();

    app.on_event(&press(KeyCode::Char(':'), KeyModifiers::NONE));
    type_str(&mut app, "filter 35=");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.state.visible.len(), visible_before);
    assert!(
        app.state.status.text.to_lowercase().contains("invalid"),
        "expected error status, got: {}",
        app.state.status.text
    );
}
