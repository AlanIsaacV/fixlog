//! Integration tests for focus-driven scrolling.
//!
//! Navigation keys (`j/k/g/G/Ctrl+D/U`) and the left/right arrows act on
//! whichever panel has focus. `Tab` / `Shift+Tab` switch focus. Raw mode
//! forces the effective focus to Detail because the list isn't rendered.
//!
//! - `Left` / `Right` drive `list_h_offset` when focus is `List`,
//!   `detail_h_offset` when focus is `Detail`.
//! - `0` resets all three offsets (list h, detail h, detail v).
//! - `Tab` toggles focus; `Shift+Tab` (BackTab) also toggles.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use fixlog_tui::TuiConfig;
use fixlog_tui::app::App;
use fixlog_tui::event::Event;
use fixlog_tui::state::Focus;

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

#[test]
fn tab_toggles_focus() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    assert_eq!(app.state.focus, Focus::List);

    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.state.focus, Focus::Detail);

    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.state.focus, Focus::List);
}

#[test]
fn shift_tab_toggles_focus() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    // BackTab is what crossterm emits for Shift+Tab.
    app.on_event(&press(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.state.focus, Focus::Detail);
}

#[test]
fn right_arrow_scrolls_list_when_focus_is_list() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    assert_eq!(app.state.focus, Focus::List);

    app.on_event(&press(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.state.list_h_offset, 8);
    assert_eq!(app.state.detail_h_offset, 0, "detail h must stay untouched");
}

#[test]
fn right_arrow_scrolls_detail_when_focus_is_detail() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.state.focus, Focus::Detail);

    app.on_event(&press(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.state.detail_h_offset, 8);
    assert_eq!(app.state.list_h_offset, 0, "list h must stay untouched");
}

#[test]
fn j_scrolls_detail_vertically_when_focus_is_detail() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    let original_cursor = app.state.cursor;
    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));

    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.state.detail_v_offset, 1);
    assert_eq!(
        app.state.cursor, original_cursor,
        "cursor must stay on current message when detail is focused"
    );

    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.state.detail_v_offset, 2);
}

#[test]
fn k_saturates_at_zero_in_detail_focus() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));

    app.on_event(&press(KeyCode::Char('k'), KeyModifiers::NONE));
    assert_eq!(app.state.detail_v_offset, 0);

    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Char('k'), KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Char('k'), KeyModifiers::NONE));
    assert_eq!(app.state.detail_v_offset, 0);
}

#[test]
fn j_still_moves_cursor_when_focus_is_list() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    // Move cursor off the end so `j` has somewhere to go.
    app.on_event(&press(KeyCode::Char('g'), KeyModifiers::NONE));
    let cursor_before = app.state.cursor;

    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.state.cursor, cursor_before + 1);
    assert_eq!(
        app.state.detail_v_offset, 0,
        "detail v must not move when list is focused"
    );
}

#[test]
fn cursor_move_resets_detail_v_offset() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    // Scroll detail down within the current message.
    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.state.detail_v_offset, 3);

    // Switch back to list, move cursor — the next refresh_detail_cache
    // must reset v_offset.
    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Char('g'), KeyModifiers::NONE));
    // refresh_detail_cache runs when the detail view renders; simulate
    // that directly by calling it.
    app.state.refresh_detail_cache();
    assert_eq!(app.state.detail_v_offset, 0);
}

#[test]
fn zero_key_resets_all_three_offsets() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    // Build up list_h, detail_h, and detail_v offsets via Tab + arrows + j.
    app.on_event(&press(KeyCode::Right, KeyModifiers::NONE)); // list_h += 8
    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Right, KeyModifiers::NONE)); // detail_h += 8
    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE)); // detail_v += 1
    assert_eq!(app.state.list_h_offset, 8);
    assert_eq!(app.state.detail_h_offset, 8);
    assert_eq!(app.state.detail_v_offset, 1);

    app.on_event(&press(KeyCode::Char('0'), KeyModifiers::NONE));
    assert_eq!(app.state.list_h_offset, 0);
    assert_eq!(app.state.detail_h_offset, 0);
    assert_eq!(app.state.detail_v_offset, 0);
}

#[test]
fn raw_mode_forces_detail_focus_for_navigation() {
    // Even with state.focus == Focus::List, raw mode must route j/k to
    // detail_v_offset because the list isn't rendered.
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    assert_eq!(app.state.focus, Focus::List);
    app.on_event(&press(KeyCode::Char('r'), KeyModifiers::NONE));
    assert!(app.state.raw_detail_mode);

    let cursor_before = app.state.cursor;
    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.state.detail_v_offset, 1);
    assert_eq!(app.state.cursor, cursor_before);
}

#[test]
fn raw_mode_hides_list_panel() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    app.on_event(&press(KeyCode::Char('r'), KeyModifiers::NONE));
    assert!(app.state.raw_detail_mode);

    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| fixlog_tui::draw(f, &mut app)).unwrap();
    let rendered = terminal.backend().to_string();

    assert!(
        rendered.contains("8=FIX.4.4|"),
        "raw should be rendered: {rendered}"
    );
    for header in ["time", "message", "client order id"] {
        assert!(
            !rendered.contains(header),
            "list header `{header}` should be hidden in raw mode: {rendered}"
        );
    }
}

#[test]
fn toggling_raw_off_restores_list_panel() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    app.on_event(&press(KeyCode::Char('r'), KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Char('r'), KeyModifiers::NONE));
    assert!(!app.state.raw_detail_mode);

    let backend = TestBackend::new(140, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| fixlog_tui::draw(f, &mut app)).unwrap();
    let rendered = terminal.backend().to_string();

    assert!(
        rendered.contains("time") && rendered.contains("message"),
        "list panel should be visible again after toggling raw off: {rendered}"
    );
}
