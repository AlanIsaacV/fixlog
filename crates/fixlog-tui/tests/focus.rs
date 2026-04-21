//! Integration tests for Fase B panel focus + detail_cursor + f/x
//! anti-filter.
//!
//! Drives a real `App` against `fix44-om.log` through synthesised key
//! events: Tab moves focus to the detail panel; `j` / `k` navigate the
//! detail's field cursor instead of the list cursor; `f` adds a
//! `tag=value` predicate to the effective filter; `x` adds the negated
//! counterpart. Toggling `c` reclamps the cursor.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

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
fn tab_switches_focus_between_list_and_detail() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    assert_eq!(app.state.focus, Focus::List);

    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.state.focus, Focus::Detail);

    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.state.focus, Focus::List);
}

#[test]
fn j_k_move_list_cursor_when_focus_is_list() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    // Move cursor near the top so `j` has somewhere to go.
    app.on_event(&press(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.state.cursor, 0);
    assert_eq!(app.state.detail_cursor, 0);

    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.state.cursor, 1, "j should step the list cursor");
    // detail_cursor resets on ordinal change.
    assert_eq!(app.state.detail_cursor, 0);
}

#[test]
fn j_k_move_detail_cursor_when_focus_is_detail() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    app.on_event(&press(KeyCode::Char('g'), KeyModifiers::NONE));
    let list_cursor_before = app.state.cursor;

    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.state.focus, Focus::Detail);
    assert_eq!(app.state.detail_cursor, 0);

    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    // The list cursor must stay put — the detail cursor is what moved.
    assert_eq!(app.state.cursor, list_cursor_before);
    assert_eq!(app.state.detail_cursor, 1);

    app.on_event(&press(KeyCode::Char('k'), KeyModifiers::NONE));
    assert_eq!(app.state.detail_cursor, 0);
}

#[test]
fn f_on_detail_field_filters_by_that_value() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    // Land on a NewOrderSingle (35=D) so the fields are stable + predictable.
    app.state.user_filter_text = Some("35=D".into());
    fixlog_tui::state::recompute_effective_filter(&mut app.state);
    assert!(!app.state.visible.is_empty(), "fixture must have 35=D");
    app.state.cursor = 0;
    app.state.refresh_detail_cache();
    let visible_before = app.state.visible.len();

    // Focus detail and walk the cursor down to the first non-common field.
    // With skip_common = false (default), common tags (8/9/10/34/35/49/52/56)
    // are present. We look for a Side (54) row, which will always be there
    // for 35=D; step forward until the cursor lands on tag 54.
    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    let mut guard = 0;
    loop {
        guard += 1;
        assert!(guard < 64, "failed to locate tag 54 within 64 steps");
        if let Some((tag, _)) = app.state.detail_cursor_field()
            && tag == 54
        {
            break;
        }
        app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    app.on_event(&press(KeyCode::Char('f'), KeyModifiers::NONE));
    // Filter now carries a predicate referring to tag 54.
    let applied = app
        .state
        .user_filter_text
        .clone()
        .expect("user_filter_text set");
    assert!(
        applied.contains("54="),
        "user filter should include tag 54 predicate, got: {applied}"
    );
    assert!(
        app.state.visible.len() <= visible_before,
        "applying a stricter filter should not grow visible"
    );
}

#[test]
fn x_on_detail_field_adds_negated_predicate() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    // Pick the Logon MsgType=A at ordinal 0 for a deterministic starting
    // point, then push focus to detail.
    app.state.cursor = 0;
    app.state.refresh_detail_cache();
    let visible_before = app.state.visible.len();

    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    // Cursor 0 is the first detail field — tag 8 (BeginString) on the
    // raw Logon message. `x` should emit `NOT (8=FIX.4.4)`.
    assert!(app.state.detail_cursor_field().is_some());
    app.on_event(&press(KeyCode::Char('x'), KeyModifiers::NONE));

    let applied = app
        .state
        .user_filter_text
        .clone()
        .expect("user_filter_text set");
    assert!(
        applied.contains("NOT"),
        "x should produce a negated predicate, got: {applied}"
    );
    // Excluding BeginString = FIX.4.4 should shrink visible to zero
    // (this fixture is all FIX 4.4) or at least not grow.
    assert!(app.state.visible.len() <= visible_before);
}

#[test]
fn toggle_skip_common_clamps_detail_cursor() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    app.state.cursor = 0;
    app.state.refresh_detail_cache();
    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));

    // Step to the very last detail field under the default (skip_common=false).
    app.on_event(&press(KeyCode::Char('G'), KeyModifiers::SHIFT));
    // `G` sets the cursor to `len-1`.
    let len_full = app.state.detail_fields_len();
    assert!(len_full > 0);
    assert_eq!(app.state.detail_cursor, len_full - 1);

    // Now turn skip_common on — the field list shrinks, cursor must clamp.
    app.on_event(&press(KeyCode::Char('c'), KeyModifiers::NONE));
    let len_filtered = app.state.detail_fields_len();
    assert!(
        len_filtered <= len_full,
        "skip_common should not grow the list"
    );
    assert!(app.state.detail_cursor < len_filtered.max(1));
}

#[test]
fn f_from_list_focus_warns_and_does_not_filter() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    assert_eq!(app.state.focus, Focus::List);
    let visible_before = app.state.visible.len();
    let filter_before = app.state.user_filter_text.clone();

    app.on_event(&press(KeyCode::Char('f'), KeyModifiers::NONE));

    assert_eq!(app.state.visible.len(), visible_before);
    assert_eq!(app.state.user_filter_text, filter_before);
}

#[test]
fn hide_heartbeat_composes_with_f_x_filter() {
    let mut app = App::bootstrap(&cfg("fix44-om.log")).expect("bootstrap");
    // Hide heartbeats first.
    app.on_event(&press(KeyCode::Char('H'), KeyModifiers::SHIFT));
    assert!(app.state.hide_heartbeat);

    app.state.cursor = 0;
    app.state.refresh_detail_cache();
    app.on_event(&press(KeyCode::Tab, KeyModifiers::NONE));
    app.on_event(&press(KeyCode::Char('f'), KeyModifiers::NONE));

    // Effective filter must still carry the heartbeat exclusion alongside
    // whatever f produced.
    let eff = app
        .state
        .filter_text
        .clone()
        .expect("filter_text set after f");
    assert!(
        eff.contains("NOT 35=0"),
        "hide_heartbeat predicate must survive f/x, got: {eff}"
    );
}
