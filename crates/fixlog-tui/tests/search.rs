//! Integration tests for the `/` search flow. Drives `App` via synthesised
//! key events so the full path (input → action → search::next_match →
//! cursor + status) is exercised.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use fixlog_core::parser::parse_one;
use fixlog_tui::TuiConfig;
use fixlog_tui::app::App;
use fixlog_tui::event::Event;
use fixlog_tui::state::InputMode;

fn cfg() -> TuiConfig {
    TuiConfig {
        path: PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/real/fix44-om.log"
        )),
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

fn msgtype_at_cursor(app: &App) -> Option<Vec<u8>> {
    let ord = *app.state.visible.get(app.state.cursor)? as usize;
    let bytes = app.state.index.message_bytes(&app.state.mmap, ord)?;
    let (msg, _) = parse_one(bytes).ok()?;
    msg.tags
        .iter()
        .find(|(t, _)| *t == 35)
        .map(|(_, v)| v.to_vec())
}

#[test]
fn slash_enters_search_mode() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.on_event(&press(KeyCode::Char('/'), KeyModifiers::NONE));
    assert_eq!(app.state.input_mode, InputMode::Search);
    assert!(app.state.search_buffer.is_empty());
}

#[test]
fn submitting_search_jumps_cursor_to_first_match() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.state.cursor = 0;

    app.on_event(&press(KeyCode::Char('/'), KeyModifiers::NONE));
    type_str(&mut app, "35=D");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.state.input_mode, InputMode::Normal);
    assert!(app.state.search_buffer.is_empty());
    assert_eq!(msgtype_at_cursor(&app).as_deref(), Some(b"D".as_ref()));
    // Remembered for n/N.
    assert_eq!(app.state.search_last_text.as_deref(), Some("35=D"));
}

#[test]
fn n_iterates_forward_through_matches() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.state.cursor = 0;
    app.on_event(&press(KeyCode::Char('/'), KeyModifiers::NONE));
    type_str(&mut app, "35=D");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));
    let first = app.state.cursor;

    app.on_event(&press(KeyCode::Char('n'), KeyModifiers::NONE));
    let second = app.state.cursor;
    assert_ne!(first, second);
    assert_eq!(msgtype_at_cursor(&app).as_deref(), Some(b"D".as_ref()));
}

#[test]
fn capital_n_iterates_backward() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.state.cursor = 0;
    app.on_event(&press(KeyCode::Char('/'), KeyModifiers::NONE));
    type_str(&mut app, "35=D");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));
    let first = app.state.cursor;

    app.on_event(&press(KeyCode::Char('N'), KeyModifiers::SHIFT));
    // Going backward from the first match wraps and lands on the last match.
    let last = app.state.cursor;
    assert_ne!(first, last);
    assert_eq!(msgtype_at_cursor(&app).as_deref(), Some(b"D".as_ref()));
}

#[test]
fn n_without_previous_search_warns() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.on_event(&press(KeyCode::Char('n'), KeyModifiers::NONE));
    assert!(
        app.state
            .status
            .text
            .to_lowercase()
            .contains("no previous search"),
        "got: {}",
        app.state.status.text
    );
}

#[test]
fn esc_during_search_cancels_without_moving_cursor() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    let original = app.state.cursor;

    app.on_event(&press(KeyCode::Char('/'), KeyModifiers::NONE));
    type_str(&mut app, "35=D");
    app.on_event(&press(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(app.state.input_mode, InputMode::Normal);
    assert_eq!(app.state.cursor, original);
    assert!(app.state.search_last_text.is_none());
}

#[test]
fn search_without_match_sets_status_no_match() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.on_event(&press(KeyCode::Char('/'), KeyModifiers::NONE));
    type_str(&mut app, "35=ZZZZ");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        app.state.status.text.to_lowercase().contains("no match"),
        "got: {}",
        app.state.status.text
    );
}

#[test]
fn search_from_end_wraps_and_sets_status() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.state.cursor = app.state.visible.len() - 1;
    app.on_event(&press(KeyCode::Char('/'), KeyModifiers::NONE));
    type_str(&mut app, "35=D");
    app.on_event(&press(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        app.state.status.text.to_lowercase().contains("wrapped"),
        "got: {}",
        app.state.status.text
    );
}
