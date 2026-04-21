//! Integration tests for vim-like navigation bindings over `App`. Exercises
//! the full input → action → state chain for `j/k/g/G/Ctrl+D/U` and
//! verifies the `Follow`/`Browse` mode contract (G snaps to Follow; any
//! other move drops to Browse).

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use fixlog_tui::TuiConfig;
use fixlog_tui::app::App;
use fixlog_tui::event::Event;
use fixlog_tui::state::ViewMode;

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

#[test]
fn j_moves_down_and_enters_browse() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    // bootstrap places cursor at the end in Follow mode; jump to row 100
    // so we can observe a downward step that isn't clamped at the bottom.
    app.state.cursor = 100;
    app.state.mode = ViewMode::Follow;

    app.on_event(&press(KeyCode::Char('j'), KeyModifiers::NONE));
    assert_eq!(app.state.cursor, 101);
    assert_eq!(app.state.mode, ViewMode::Browse);
}

#[test]
fn k_moves_up_and_clamps_at_zero() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.state.cursor = 1;
    app.on_event(&press(KeyCode::Char('k'), KeyModifiers::NONE));
    assert_eq!(app.state.cursor, 0);

    // One more k at the top must clamp, not wrap.
    app.on_event(&press(KeyCode::Char('k'), KeyModifiers::NONE));
    assert_eq!(app.state.cursor, 0);
}

#[test]
fn uppercase_g_snaps_to_bottom_and_enters_follow() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.state.cursor = 0;
    app.state.mode = ViewMode::Browse;
    app.on_event(&press(KeyCode::Char('G'), KeyModifiers::SHIFT));
    assert_eq!(app.state.cursor, app.state.visible.len() - 1);
    assert_eq!(app.state.mode, ViewMode::Follow);
}

#[test]
fn lowercase_g_jumps_to_top_and_enters_browse() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    app.on_event(&press(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.state.cursor, 0);
    assert_eq!(app.state.mode, ViewMode::Browse);
}

#[test]
fn ctrl_d_and_ctrl_u_use_last_list_height() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    // Simulate a prior render having observed a 40-row viewport.
    app.state.last_list_height = 40;
    app.state.cursor = 500;

    app.on_event(&press(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert_eq!(app.state.cursor, 520);

    app.on_event(&press(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.state.cursor, 500);
}

#[test]
fn uppercase_f_toggles_mode_and_resets_on_return_to_follow() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    assert_eq!(app.state.mode, ViewMode::Follow);
    app.state.new_since_browse = 0;

    app.on_event(&press(KeyCode::Char('F'), KeyModifiers::SHIFT));
    assert_eq!(app.state.mode, ViewMode::Browse);

    // Simulate some messages arriving while browsing.
    fixlog_tui::app::on_index_grew(&mut app.state, 42);
    assert_eq!(app.state.new_since_browse, 42);
    // Cursor should not move in Browse.
    let cursor_before = app.state.cursor;
    fixlog_tui::app::on_index_grew(&mut app.state, 3);
    assert_eq!(app.state.cursor, cursor_before);
    assert_eq!(app.state.new_since_browse, 45);

    // Switch back to Follow: counter resets, cursor jumps to end.
    app.on_event(&press(KeyCode::Char('F'), KeyModifiers::SHIFT));
    assert_eq!(app.state.mode, ViewMode::Follow);
    assert_eq!(app.state.new_since_browse, 0);
    assert_eq!(app.state.cursor, app.state.visible.len() - 1);
}

#[test]
fn on_index_grew_in_follow_keeps_cursor_at_end() {
    let mut app = App::bootstrap(&cfg()).expect("bootstrap");
    assert_eq!(app.state.mode, ViewMode::Follow);

    // In Follow mode, new arrivals should not inflate new_since_browse.
    fixlog_tui::app::on_index_grew(&mut app.state, 10);
    assert_eq!(app.state.new_since_browse, 0);
    assert_eq!(app.state.cursor, app.state.visible.len() - 1);
}

#[test]
fn navigation_is_noop_on_empty_visible() {
    // Impose a filter that matches nothing and confirm none of the
    // navigation bindings crash or mutate the cursor.
    let mut app = App::bootstrap(&TuiConfig {
        initial_filter: Some("35=ZZZ".to_string()),
        ..cfg()
    })
    .expect("bootstrap");
    assert!(app.state.visible.is_empty());

    for code in [
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('g'),
        KeyCode::Char('G'),
    ] {
        let mods = if matches!(code, KeyCode::Char('G')) {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        app.on_event(&press(code, mods));
    }

    assert_eq!(app.state.cursor, 0);
}
