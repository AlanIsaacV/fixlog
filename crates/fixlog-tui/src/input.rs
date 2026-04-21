//! Map raw terminal events to high-level actions.
//!
//! The mapping depends on the current [`InputMode`]:
//! - `Normal` maps vim-like bindings (navigation, `:` to enter command mode,
//!   `q`/`Ctrl+C` to quit).
//! - `Command` routes keystrokes into the command buffer (character input,
//!   Backspace, Enter, Esc, history navigation).
//!
//! Tests live here so the key→action table is exercised without needing a
//! live `AppState`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::event::Event;
use crate::state::InputMode;

/// High-level command emitted by the input layer. The event loop maps each
/// action to mutations on `App`/`AppState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Resize(u16, u16),
    EnterCommand,
    CommandChar(char),
    CommandBackspace,
    CommandSubmit,
    CommandCancel,
    CommandHistoryPrev,
    CommandHistoryNext,
    /// `j` / `↓` — move cursor down one row.
    CursorDown,
    /// `k` / `↑` — move cursor up one row.
    CursorUp,
    /// `g` — jump to the first row. Activates `Browse` mode.
    CursorTop,
    /// `G` — jump to the last row. Activates `Follow` mode so new messages
    /// stay on screen.
    CursorBottom,
    /// `Ctrl+D` / `PageDown` — half page down. Browse.
    CursorHalfPageDown,
    /// `Ctrl+U` / `PageUp` — half page up. Browse.
    CursorHalfPageUp,
    /// `F` — toggle between `Follow` (auto-scroll) and `Browse` (free cursor).
    /// Switching back to `Follow` resets the "⬇ N new" counter and snaps the
    /// cursor to the end of `visible`.
    ToggleMode,
    /// `/` — enter search mode.
    EnterSearch,
    SearchChar(char),
    SearchBackspace,
    SearchSubmit,
    SearchCancel,
    /// `n` — next match in the last search direction.
    SearchNext,
    /// `N` — previous match.
    SearchPrev,
    /// `y` press — resolved by the app layer as the first half of a `yy` /
    /// `yY` sequence.
    YankPrefix,
    /// `Y` press — only meaningful after a prior `y`; otherwise the app
    /// layer drops it.
    YankPretty,
    /// `O` — open order lifecycle overlay for the ordinal under the cursor.
    OpenOrderTimeline,
    /// `d` press — first half of a `dd` / `dD` sequence.
    DiffPrefix,
    /// `d` after `d` prefix — set diff slot A to the current cursor.
    DiffSlotA,
    /// `D` after `d` prefix — set diff slot B and open the diff overlay.
    DiffSlotB,
    /// `m` press — first half of a `m<letter>` sequence.
    MarkPrefix,
    /// `'` press — first half of a `'<letter>` sequence.
    JumpPrefix,
    /// A mark-completion character (ASCII digit) emitted only when a
    /// `m` or `'` prefix is pending. Marks are restricted to digits
    /// (`0..=9`) so an accidental follow-up keystroke never triggers a
    /// letter-bound shortcut instead of setting/jumping the mark.
    Letter(char),
    /// `?` in normal mode — open the help overlay directly (mirrors
    /// `:help` in command mode).
    OpenHelp,
    /// `Esc` in normal mode — closes an open overlay if one is up.
    OverlayClose,
    /// `Enter` in normal mode — "commit" in the current overlay (e.g.
    /// apply session filter). Ignored when no overlay is open.
    OverlayApply,
    /// `c` in normal mode — toggle hiding of session-layer common tags
    /// (8, 9, 10, 34, 35, 49, 52, 56) in the detail panel.
    ToggleSkipCommon,
    /// `H` in normal mode — toggle hiding of Heartbeat messages (35=0) by
    /// composing `AND NOT 35=0` on top of the user's filter.
    ToggleHideHeartbeat,
    /// `r` in normal mode — toggle the detail panel between the resolved
    /// tag table and the raw FIX bytes (SOH → `|`).
    ToggleRawDetail,
    /// `→` / `Right` — scroll the focused panel right one step.
    ScrollRight,
    /// `←` / `Left` — scroll the focused panel left one step.
    ScrollLeft,
    /// `0` (zero) — reset all scroll offsets (list h, detail h, detail v).
    ScrollHome,
    /// `Tab` — move focus to the next panel (List → Detail → List).
    FocusNext,
    /// `Shift+Tab` / `BackTab` — move focus to the previous panel. With
    /// only two panels today this is the same toggle as `FocusNext`, but
    /// the separate variant keeps room for a third panel later.
    FocusPrev,
    /// `f` / `x` in Normal mode while `focus == Detail`: AND a
    /// `tag=value` (or `NOT tag=value`) predicate into the effective
    /// filter, taking `(tag, value)` from the row at `detail_cursor`.
    /// Ignored when focus is on the list.
    FilterFromDetail {
        negated: bool,
    },
    /// `o` in Normal mode — cycle the sort criterion applied to the
    /// visible list. Rotates natural → 34 → 60 → 52 → natural. Preserves
    /// the message currently under the cursor when possible.
    CycleSortKey,
}

pub fn map_event(ev: &Event, mode: InputMode) -> Action {
    match ev {
        Event::Key(k) => match mode {
            InputMode::Normal => map_normal_key(k),
            InputMode::Command => map_command_key(k),
            InputMode::Search => map_search_key(k),
        },
        Event::Resize(w, h) => Action::Resize(*w, *h),
        Event::Tick => Action::None,
    }
}

/// Variant of [`map_event`] used when `pending_prefix` is `m` or `'` in
/// Normal mode. Any ASCII digit becomes [`Action::Letter`] regardless of
/// its dedicated binding (notably `0` → `ScrollHome`), so marks `m0`..`m9`
/// and `'0`..`'9` always complete without triggering a shortcut.
///
/// Non-digit keys stay on the standard mapping so `Esc`, `Ctrl+C`,
/// resize events, ticks, and any accidental letter press continue to
/// behave normally — the app layer will drop the stale prefix and
/// dispatch the letter's regular action.
pub fn map_event_digit_priority(ev: &Event) -> Action {
    match ev {
        Event::Key(k) => {
            if let KeyCode::Char(c) = k.code
                && c.is_ascii_digit()
                && !k.modifiers.contains(KeyModifiers::CONTROL)
            {
                return Action::Letter(c);
            }
            map_normal_key(k)
        }
        Event::Resize(w, h) => Action::Resize(*w, *h),
        Event::Tick => Action::None,
    }
}

pub fn map_normal_key(k: &KeyEvent) -> Action {
    match (k.code, k.modifiers) {
        (KeyCode::Char('q'), KeyModifiers::NONE) => Action::Quit,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Quit,
        // `:` arrives as Char(':') with either NONE or SHIFT depending on keymap.
        (KeyCode::Char(':'), _) => Action::EnterCommand,
        // `?` mirrors `:help` without entering command mode.
        (KeyCode::Char('?'), _) => Action::OpenHelp,
        // Vim-like navigation.
        (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, KeyModifiers::NONE) => {
            Action::CursorDown
        }
        (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, KeyModifiers::NONE) => {
            Action::CursorUp
        }
        (KeyCode::Char('g'), KeyModifiers::NONE) => Action::CursorTop,
        (KeyCode::Char('G'), _) => Action::CursorBottom,
        (KeyCode::Char('d'), KeyModifiers::CONTROL) | (KeyCode::PageDown, KeyModifiers::NONE) => {
            Action::CursorHalfPageDown
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) | (KeyCode::PageUp, KeyModifiers::NONE) => {
            Action::CursorHalfPageUp
        }
        (KeyCode::Char('F'), _) => Action::ToggleMode,
        (KeyCode::Char('/'), _) => Action::EnterSearch,
        (KeyCode::Char('n'), KeyModifiers::NONE) => Action::SearchNext,
        (KeyCode::Char('N'), _) => Action::SearchPrev,
        (KeyCode::Char('y'), KeyModifiers::NONE) => Action::YankPrefix,
        (KeyCode::Char('Y'), _) => Action::YankPretty,
        (KeyCode::Char('O'), _) => Action::OpenOrderTimeline,
        (KeyCode::Char('d'), KeyModifiers::NONE) => Action::DiffPrefix,
        (KeyCode::Char('D'), _) => Action::DiffSlotB,
        (KeyCode::Char('m'), KeyModifiers::NONE) => Action::MarkPrefix,
        (KeyCode::Char('\''), _) => Action::JumpPrefix,
        (KeyCode::Char('c'), KeyModifiers::NONE) => Action::ToggleSkipCommon,
        (KeyCode::Char('H'), _) => Action::ToggleHideHeartbeat,
        (KeyCode::Char('r'), KeyModifiers::NONE) => Action::ToggleRawDetail,
        (KeyCode::Right, _) => Action::ScrollRight,
        (KeyCode::Left, _) => Action::ScrollLeft,
        (KeyCode::Char('0'), KeyModifiers::NONE) => Action::ScrollHome,
        (KeyCode::Tab, _) => Action::FocusNext,
        (KeyCode::BackTab, _) => Action::FocusPrev,
        (KeyCode::Char('f'), KeyModifiers::NONE) => Action::FilterFromDetail { negated: false },
        (KeyCode::Char('x'), KeyModifiers::NONE) => Action::FilterFromDetail { negated: true },
        (KeyCode::Char('o'), KeyModifiers::NONE) => Action::CycleSortKey,
        (KeyCode::Esc, _) => Action::OverlayClose,
        (KeyCode::Enter, _) => Action::OverlayApply,
        // Letter is only meaningful after m / ' prefix; let the app layer
        // disambiguate. We emit it for any uppercase/lowercase ASCII char
        // that doesn't already have a dedicated binding in the table
        // above (those early-return via the earlier arms).
        (KeyCode::Char(c), _) if c.is_ascii_alphabetic() => Action::Letter(c),
        _ => Action::None,
    }
}

pub fn map_search_key(k: &KeyEvent) -> Action {
    match (k.code, k.modifiers) {
        (KeyCode::Esc, _) => Action::SearchCancel,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::SearchCancel,
        (KeyCode::Enter, _) => Action::SearchSubmit,
        (KeyCode::Backspace, _) => Action::SearchBackspace,
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => Action::SearchChar(c),
        _ => Action::None,
    }
}

pub fn map_command_key(k: &KeyEvent) -> Action {
    match (k.code, k.modifiers) {
        (KeyCode::Esc, _) => Action::CommandCancel,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::CommandCancel,
        (KeyCode::Enter, _) => Action::CommandSubmit,
        (KeyCode::Backspace, _) => Action::CommandBackspace,
        (KeyCode::Up, _) => Action::CommandHistoryPrev,
        (KeyCode::Down, _) => Action::CommandHistoryNext,
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => Action::CommandChar(c),
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn q_maps_to_quit_in_normal_mode() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('q'), KeyModifiers::NONE)),
            Action::Quit
        );
    }

    #[test]
    fn ctrl_c_maps_to_quit_in_normal_mode() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }

    #[test]
    fn colon_enters_command_mode() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char(':'), KeyModifiers::NONE)),
            Action::EnterCommand
        );
        // Shift+; on US layouts still arrives as Char(':').
        assert_eq!(
            map_normal_key(&key(KeyCode::Char(':'), KeyModifiers::SHIFT)),
            Action::EnterCommand
        );
    }

    #[test]
    fn question_mark_opens_help() {
        // `?` arrives as Char('?') with or without SHIFT depending on keymap.
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('?'), KeyModifiers::NONE)),
            Action::OpenHelp
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('?'), KeyModifiers::SHIFT)),
            Action::OpenHelp
        );
    }

    #[test]
    fn shift_q_is_not_quit() {
        // `Q` is now a generic `Letter` (completion candidate for
        // `m<letter>` / `'<letter>` sequences); the app layer only acts
        // on it when a prefix is pending.
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('Q'), KeyModifiers::NONE)),
            Action::Letter('Q')
        );
    }

    #[test]
    fn unbound_letter_keys_are_letter() {
        // Any ASCII alphabetic char without a dedicated binding emits
        // `Letter(c)` so mark-set / mark-jump sequences work. Non-letter
        // keys stay `Action::None`. `z` is unbound today.
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('z'), KeyModifiers::NONE)),
            Action::Letter('z')
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::OverlayApply
        );
    }

    #[test]
    fn f_and_x_map_to_filter_from_detail() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('f'), KeyModifiers::NONE)),
            Action::FilterFromDetail { negated: false }
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('x'), KeyModifiers::NONE)),
            Action::FilterFromDetail { negated: true }
        );
    }

    #[test]
    fn command_mode_collects_chars() {
        assert_eq!(
            map_command_key(&key(KeyCode::Char('f'), KeyModifiers::NONE)),
            Action::CommandChar('f')
        );
        assert_eq!(
            map_command_key(&key(KeyCode::Char('5'), KeyModifiers::NONE)),
            Action::CommandChar('5')
        );
    }

    #[test]
    fn command_mode_handles_editing_keys() {
        assert_eq!(
            map_command_key(&key(KeyCode::Backspace, KeyModifiers::NONE)),
            Action::CommandBackspace
        );
        assert_eq!(
            map_command_key(&key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::CommandSubmit
        );
        assert_eq!(
            map_command_key(&key(KeyCode::Esc, KeyModifiers::NONE)),
            Action::CommandCancel
        );
        assert_eq!(
            map_command_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::CommandCancel
        );
    }

    #[test]
    fn command_mode_ignores_ctrl_chars() {
        // Ctrl+something that isn't our bound set should not land in the
        // command buffer.
        assert_eq!(
            map_command_key(&key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            Action::None
        );
    }

    #[test]
    fn y_and_shift_y_map_to_yank_actions() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('y'), KeyModifiers::NONE)),
            Action::YankPrefix
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('Y'), KeyModifiers::SHIFT)),
            Action::YankPretty
        );
    }

    #[test]
    fn slash_enters_search_mode() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('/'), KeyModifiers::NONE)),
            Action::EnterSearch
        );
    }

    #[test]
    fn n_and_shift_n_map_to_search_iterators() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('n'), KeyModifiers::NONE)),
            Action::SearchNext
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('N'), KeyModifiers::SHIFT)),
            Action::SearchPrev
        );
    }

    #[test]
    fn search_mode_collects_chars_and_submits() {
        assert_eq!(
            map_search_key(&key(KeyCode::Char('D'), KeyModifiers::NONE)),
            Action::SearchChar('D')
        );
        assert_eq!(
            map_search_key(&key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::SearchSubmit
        );
        assert_eq!(
            map_search_key(&key(KeyCode::Esc, KeyModifiers::NONE)),
            Action::SearchCancel
        );
    }

    #[test]
    fn lowercase_c_toggles_skip_common() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('c'), KeyModifiers::NONE)),
            Action::ToggleSkipCommon
        );
    }

    #[test]
    fn ctrl_c_still_quits_not_toggle() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }

    #[test]
    fn uppercase_h_toggles_hide_heartbeat() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('H'), KeyModifiers::SHIFT)),
            Action::ToggleHideHeartbeat
        );
    }

    #[test]
    fn uppercase_f_toggles_mode() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('F'), KeyModifiers::SHIFT)),
            Action::ToggleMode
        );
    }

    #[test]
    fn lowercase_r_toggles_raw_detail() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('r'), KeyModifiers::NONE)),
            Action::ToggleRawDetail
        );
    }

    #[test]
    fn arrow_keys_scroll_focused_panel() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Right, KeyModifiers::NONE)),
            Action::ScrollRight
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Left, KeyModifiers::NONE)),
            Action::ScrollLeft
        );
        // Shift is ignored on arrows (both modifiers land on the same action).
        assert_eq!(
            map_normal_key(&key(KeyCode::Right, KeyModifiers::SHIFT)),
            Action::ScrollRight
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Left, KeyModifiers::SHIFT)),
            Action::ScrollLeft
        );
    }

    #[test]
    fn tab_switches_focus() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Tab, KeyModifiers::NONE)),
            Action::FocusNext
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Action::FocusPrev
        );
    }

    #[test]
    fn zero_resets_horizontal_scroll() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('0'), KeyModifiers::NONE)),
            Action::ScrollHome
        );
    }

    #[test]
    fn navigation_keys_map_to_cursor_actions() {
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('j'), KeyModifiers::NONE)),
            Action::CursorDown
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Down, KeyModifiers::NONE)),
            Action::CursorDown
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('k'), KeyModifiers::NONE)),
            Action::CursorUp
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Up, KeyModifiers::NONE)),
            Action::CursorUp
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('g'), KeyModifiers::NONE)),
            Action::CursorTop
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('G'), KeyModifiers::SHIFT)),
            Action::CursorBottom
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Action::CursorHalfPageDown
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            Action::CursorHalfPageUp
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::PageDown, KeyModifiers::NONE)),
            Action::CursorHalfPageDown
        );
        assert_eq!(
            map_normal_key(&key(KeyCode::PageUp, KeyModifiers::NONE)),
            Action::CursorHalfPageUp
        );
    }

    #[test]
    fn command_mode_history_arrows() {
        assert_eq!(
            map_command_key(&key(KeyCode::Up, KeyModifiers::NONE)),
            Action::CommandHistoryPrev
        );
        assert_eq!(
            map_command_key(&key(KeyCode::Down, KeyModifiers::NONE)),
            Action::CommandHistoryNext
        );
    }

    #[test]
    fn resize_maps_through() {
        assert_eq!(
            map_event(&Event::Resize(120, 40), InputMode::Normal),
            Action::Resize(120, 40)
        );
    }

    #[test]
    fn tick_is_none() {
        assert_eq!(map_event(&Event::Tick, InputMode::Normal), Action::None);
        assert_eq!(map_event(&Event::Tick, InputMode::Command), Action::None);
    }

    #[test]
    fn digit_priority_promotes_all_digits_including_zero() {
        // `0` is bound to `ScrollHome` under the standard mapping; under
        // digit-priority it must become `Letter('0')` so `m0` / `'0`
        // complete without triggering ScrollHome.
        for c in ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'] {
            assert_eq!(
                map_event_digit_priority(&Event::Key(key(KeyCode::Char(c), KeyModifiers::NONE))),
                Action::Letter(c),
                "digit {c} should be promoted under digit-priority"
            );
        }
    }

    #[test]
    fn digit_priority_preserves_letters_and_specials() {
        // Letters fall through to their normal bindings — an accidental
        // letter press after `m`/`'` must not be silently swallowed as
        // a mark character. The app layer clears the stale prefix.
        assert_eq!(
            map_event_digit_priority(&Event::Key(key(KeyCode::Char('c'), KeyModifiers::NONE))),
            Action::ToggleSkipCommon
        );
        assert_eq!(
            map_event_digit_priority(&Event::Key(key(KeyCode::Esc, KeyModifiers::NONE))),
            Action::OverlayClose
        );
        assert_eq!(
            map_event_digit_priority(&Event::Key(key(KeyCode::Char('c'), KeyModifiers::CONTROL))),
            Action::Quit
        );
        assert_eq!(
            map_event_digit_priority(&Event::Resize(10, 20)),
            Action::Resize(10, 20)
        );
    }
}
