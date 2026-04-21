//! Event-loop-level application wrapper.
//!
//! `App` owns the [`AppState`] (data model) plus event-loop bookkeeping
//! (`should_quit`). Keybindings are mapped to [`Action`]s in `input.rs`;
//! `App::apply` is the single site that mutates state in response to actions.

use anyhow::Result;

use crate::TuiConfig;
use crate::clipboard;
use crate::command::{self, Outcome};
use crate::event::Event;
use crate::input::{Action, map_event, map_event_digit_priority};
use crate::search::{self, Direction, Hit};
use crate::state::{
    AppState, Focus, InputMode, Overlay, StatusMessage, ViewMode, bootstrap_with_sort,
    recompute_effective_filter, restore_filter, snapshot_filter, sort_visible,
};
use crate::view::sessions::session_at;

use fixlog_core::query::parse as parse_query;

pub struct App {
    pub state: AppState,
    pub should_quit: bool,
}

impl App {
    /// Build the app by bootstrapping its state from the config.
    pub fn bootstrap(cfg: &TuiConfig) -> Result<Self> {
        let state = bootstrap_with_sort(&cfg.path, cfg.initial_filter.as_deref(), cfg.sort_key)?;
        Ok(Self {
            state,
            should_quit: false,
        })
    }

    /// Dispatch one event: map it to an [`Action`] and apply it.
    pub fn on_event(&mut self, ev: &Event) {
        // While a `m` or `'` prefix is pending in Normal mode, route any
        // ASCII digit to `Letter(_)` so `m0`..`m9` and `'0`..`'9` always
        // complete regardless of dedicated bindings (notably `0` →
        // `ScrollHome`). Letters fall through to their regular actions
        // so an accidental keystroke never silently mis-sets a mark.
        let action = match (self.state.pending_prefix, self.state.input_mode) {
            (Some('m'), InputMode::Normal) | (Some('\''), InputMode::Normal) => {
                map_event_digit_priority(ev)
            }
            _ => map_event(ev, self.state.input_mode),
        };
        self.apply(action);
    }

    pub fn apply(&mut self, action: Action) {
        // Overlay-first dispatch: if a navigation overlay is open, j/k/Enter
        // operate on the overlay cursor rather than the main list.
        if self.state.overlay.is_some()
            && self.state.input_mode == InputMode::Normal
            && self.overlay_intercept(&action)
        {
            return;
        }

        // Two-key prefix sequences. If a prefix is pending, try to consume
        // this action as the completion; if it isn't a valid continuation,
        // drop the prefix and re-process the action as a fresh key.
        if let Some(prefix) = self.state.pending_prefix.take() {
            match (prefix, &action) {
                ('y', Action::YankPrefix) => {
                    self.yank_raw();
                    return;
                }
                ('y', Action::YankPretty) => {
                    self.yank_pretty();
                    return;
                }
                ('d', Action::DiffPrefix) => {
                    self.set_diff_slot(0);
                    return;
                }
                ('d', Action::DiffSlotB) => {
                    self.set_diff_slot(1);
                    return;
                }
                ('m', Action::Letter(c)) => {
                    self.set_bookmark(*c);
                    return;
                }
                ('\'', Action::Letter(c)) => {
                    self.jump_bookmark(*c);
                    return;
                }
                _ => {
                    // Prefix was pending but the action didn't complete it.
                    // Fall through and handle `action` as a fresh key.
                }
            }
        }

        match action {
            Action::None | Action::Resize(_, _) => {}
            Action::Quit => self.should_quit = true,
            Action::EnterCommand => {
                self.state.input_mode = InputMode::Command;
                self.state.command_buffer.clear();
                self.state.command_history_idx = None;
                self.state.filter_snapshot = Some(snapshot_filter(&self.state));
            }
            Action::CommandCancel => {
                self.state.input_mode = InputMode::Normal;
                self.state.command_buffer.clear();
                self.state.command_history_idx = None;
                if let Some(snap) = self.state.filter_snapshot.take() {
                    restore_filter(&mut self.state, snap);
                }
            }
            Action::CommandChar(c) => {
                self.state.command_buffer.push(c);
                self.state.command_history_idx = None;
                self.live_preview();
            }
            Action::CommandBackspace => {
                self.state.command_buffer.pop();
                self.state.command_history_idx = None;
                self.live_preview();
            }
            Action::CommandSubmit => self.submit_command(),
            Action::CommandHistoryPrev => self.nav_history(HistoryDir::Prev),
            Action::CommandHistoryNext => self.nav_history(HistoryDir::Next),
            Action::CursorDown => nav(&mut self.state, MoveDelta::Delta(1)),
            Action::CursorUp => nav(&mut self.state, MoveDelta::Delta(-1)),
            Action::CursorTop => nav(&mut self.state, MoveDelta::Top),
            Action::CursorBottom => nav(&mut self.state, MoveDelta::Bottom),
            Action::CursorHalfPageDown => nav(&mut self.state, MoveDelta::HalfPageDown),
            Action::CursorHalfPageUp => nav(&mut self.state, MoveDelta::HalfPageUp),
            Action::ToggleMode => toggle_mode(&mut self.state),
            Action::EnterSearch => {
                self.state.input_mode = InputMode::Search;
                self.state.search_buffer.clear();
            }
            Action::SearchChar(c) => {
                self.state.search_buffer.push(c);
            }
            Action::SearchBackspace => {
                self.state.search_buffer.pop();
            }
            Action::SearchCancel => {
                self.state.input_mode = InputMode::Normal;
                self.state.search_buffer.clear();
            }
            Action::SearchSubmit => self.submit_search(),
            Action::SearchNext => self.iterate_search(Direction::Forward),
            Action::SearchPrev => self.iterate_search(Direction::Backward),
            Action::YankPrefix => {
                // First `y` of a sequence. Set the prefix and wait for the
                // second key.
                self.state.pending_prefix = Some('y');
            }
            Action::YankPretty => {
                // `Y` without a prior `y` does nothing — matches vim's
                // behaviour for incomplete operator-pending motions.
                self.state.status = StatusMessage::warn("press yy or yY to yank");
            }
            Action::OpenOrderTimeline => {
                crate::command::open_orders_overlay(&mut self.state, None);
            }
            Action::DiffPrefix => {
                // First `d`: set prefix and wait for the second key.
                self.state.pending_prefix = Some('d');
            }
            Action::DiffSlotA => {
                // Bare `DiffSlotA` without prefix is meaningless.
                self.state.status = StatusMessage::warn("press dd to set diff slot A");
            }
            Action::DiffSlotB => {
                // `D` without a prior `d` prefix is still useful: if slot
                // A is already set (from a previous `dd`), treat it as
                // the completion that sets slot B and opens the diff.
                // The 4-key path `dd` + `dD` still works via the prefix
                // arm above.
                if self.state.diff_slots[0].is_some() {
                    self.set_diff_slot(1);
                } else {
                    self.state.status = StatusMessage::warn("press dd to set diff slot A first");
                }
            }
            Action::MarkPrefix => {
                self.state.pending_prefix = Some('m');
            }
            Action::JumpPrefix => {
                self.state.pending_prefix = Some('\'');
            }
            Action::Letter(_) => {
                // Only meaningful as a completion for m / ' — handled above.
            }
            Action::OpenHelp => {
                self.state.overlay = Some(Overlay::Help { scroll: 0 });
            }
            Action::OverlayClose => {
                if self.state.overlay.is_some() {
                    self.state.overlay = None;
                } else if self.state.pending_prefix.is_some() {
                    // Swallow Esc to also clear a stuck prefix.
                } else {
                    // No overlay, no prefix: Esc in normal mode is a no-op.
                }
            }
            Action::OverlayApply => {
                // Enter in normal mode with no overlay is a no-op.
            }
            Action::ToggleSkipCommon => {
                self.state.skip_common = !self.state.skip_common;
                // Clamp detail_cursor to the new filtered field count so
                // highlight + `f`/`x` don't index past the end.
                let len = self.state.detail_fields_len();
                if len == 0 {
                    self.state.detail_cursor = 0;
                } else if self.state.detail_cursor >= len {
                    self.state.detail_cursor = len - 1;
                }
                let msg = if self.state.skip_common {
                    "detail: common fields hidden"
                } else {
                    "detail: showing all fields"
                };
                self.state.status = StatusMessage::info(msg);
            }
            Action::ToggleHideHeartbeat => {
                self.state.hide_heartbeat = !self.state.hide_heartbeat;
                recompute_effective_filter(&mut self.state);
                let msg = if self.state.hide_heartbeat {
                    format!("heartbeats hidden — {} match", self.state.visible.len())
                } else {
                    format!("heartbeats shown — {} match", self.state.visible.len())
                };
                self.state.status = StatusMessage::info(msg);
            }
            Action::ToggleRawDetail => {
                self.state.raw_detail_mode = !self.state.raw_detail_mode;
                let msg = if self.state.raw_detail_mode {
                    "detail: raw FIX bytes (wrapped, full width) — press `yy` to copy"
                } else {
                    "detail: resolved fields"
                };
                self.state.status = StatusMessage::info(msg);
            }
            Action::ScrollRight => match effective_focus(&self.state) {
                Focus::List => {
                    self.state.list_h_offset = self.state.list_h_offset.saturating_add(SCROLL_STEP);
                }
                Focus::Detail => {
                    self.state.detail_h_offset =
                        self.state.detail_h_offset.saturating_add(SCROLL_STEP);
                }
            },
            Action::ScrollLeft => match effective_focus(&self.state) {
                Focus::List => {
                    self.state.list_h_offset = self.state.list_h_offset.saturating_sub(SCROLL_STEP);
                }
                Focus::Detail => {
                    self.state.detail_h_offset =
                        self.state.detail_h_offset.saturating_sub(SCROLL_STEP);
                }
            },
            Action::ScrollHome => {
                self.state.list_h_offset = 0;
                self.state.detail_h_offset = 0;
                self.state.detail_v_offset = 0;
            }
            Action::FocusNext | Action::FocusPrev => {
                self.state.focus = match self.state.focus {
                    Focus::List => Focus::Detail,
                    Focus::Detail => Focus::List,
                };
                let label = match effective_focus(&self.state) {
                    Focus::List => "focus: list",
                    Focus::Detail => "focus: detail",
                };
                self.state.status = StatusMessage::info(label);
            }
            Action::FilterFromDetail { negated } => self.filter_from_detail(negated),
            Action::CycleSortKey => self.cycle_sort_key(),
        }
    }

    /// Advance `sort_key` to the next value and re-sort the currently
    /// visible list in place. The cursor tracks the message it pointed
    /// at before the sort so the user doesn't lose their spot when
    /// rotating through modes.
    fn cycle_sort_key(&mut self) {
        let anchor_ord = self.state.visible.get(self.state.cursor).copied();
        self.state.sort_key = self.state.sort_key.cycle();
        sort_visible(
            &mut self.state.visible,
            &self.state.mmap,
            &self.state.index,
            &self.state.format,
            self.state.sort_key,
        );
        if let Some(ord) = anchor_ord
            && let Some(idx) = self.state.visible.iter().position(|&o| o == ord)
        {
            self.state.cursor = idx;
        } else {
            self.state.cursor = self.state.visible.len().saturating_sub(1);
        }
        self.state.viewport_top = 0;
        self.state.status = StatusMessage::info(format!("sort: {}", self.state.sort_key.label()));
    }

    /// Build a filter predicate from the row currently highlighted in the
    /// detail panel and compose it into `user_filter_text`. Only active
    /// when `focus == Detail`; otherwise reports a warning. The predicate
    /// is `tag=value` (or `NOT (tag=value)` when `negated`). Raw value
    /// bytes are taken from the message — not the decoded label — so the
    /// comparison matches byte-for-byte against the secondary index.
    fn filter_from_detail(&mut self, negated: bool) {
        if effective_focus(&self.state) != Focus::Detail {
            self.state.status =
                StatusMessage::warn("press Tab to focus detail, then f/x on a field");
            return;
        }
        let Some((tag, value)) = self.state.detail_cursor_field() else {
            self.state.status = StatusMessage::warn("no field under detail cursor");
            return;
        };
        let Some(expr) = filter_expr_from_field(tag, &value, negated) else {
            self.state.status =
                StatusMessage::warn("cannot quote field value (contains unsafe bytes)");
            return;
        };
        let new_user = match self.state.user_filter_text.take() {
            Some(prev) if !prev.is_empty() => format!("({prev}) AND {expr}"),
            _ => expr.clone(),
        };
        self.state.user_filter_text = Some(new_user);
        recompute_effective_filter(&mut self.state);
        self.state.status = StatusMessage::info(format!("filter: {expr}"));
    }

    fn set_diff_slot(&mut self, slot: usize) {
        if self.state.visible.is_empty() {
            self.state.status = StatusMessage::warn("no message under cursor");
            return;
        }
        let ord = self.state.visible[self.state.cursor];
        self.state.diff_slots[slot] = Some(ord);
        if let (Some(a), Some(b)) = (self.state.diff_slots[0], self.state.diff_slots[1]) {
            if a == b {
                self.state.status = StatusMessage::info("diff: both slots same");
            } else {
                self.state.overlay = Some(Overlay::Diff);
            }
        } else {
            let label = if slot == 0 { "A" } else { "B" };
            self.state.status = StatusMessage::info(format!("diff slot {label} set to #{ord}"));
        }
    }

    fn set_bookmark(&mut self, c: char) {
        if !c.is_ascii_digit() {
            self.state.status = StatusMessage::warn("marks accept digits 0-9 only");
            return;
        }
        if self.state.visible.is_empty() {
            self.state.status = StatusMessage::warn("no message under cursor");
            return;
        }
        let ord = self.state.visible[self.state.cursor];
        self.state.bookmarks.insert(c, ord);
        self.state.status = StatusMessage::info(format!("mark '{c}' set to #{ord}"));
    }

    fn jump_bookmark(&mut self, c: char) {
        if !c.is_ascii_digit() {
            self.state.status = StatusMessage::warn("marks accept digits 0-9 only");
            return;
        }
        let Some(&ord) = self.state.bookmarks.get(&c) else {
            self.state.status = StatusMessage::warn(format!("mark '{c}' not set"));
            return;
        };
        if let Some(idx) = self.state.visible.iter().position(|&o| o == ord) {
            self.state.cursor = idx;
            self.state.mode = ViewMode::Browse;
            self.state.status = StatusMessage::info(format!("jumped to '{c}' (#{ord})"));
        } else {
            self.state.status = StatusMessage::warn(format!("mark '{c}' not in filtered view"));
        }
    }

    /// When an overlay owns the foreground, consume actions targeted at
    /// its cursor or at closing it. Returns `true` if the action was
    /// handled and the caller should stop dispatching.
    ///
    /// Unhandled actions fall through so `q`, `:`, etc. still work.
    fn overlay_intercept(&mut self, action: &Action) -> bool {
        match action {
            Action::OverlayClose => {
                self.state.overlay = None;
                true
            }
            Action::Quit => false,
            Action::EnterCommand => false,
            Action::CursorDown | Action::CursorUp => {
                if let Some(Overlay::Sessions { map, cursor }) = &mut self.state.overlay {
                    let len = map.by_key.len();
                    if len == 0 {
                        return true;
                    }
                    *cursor = match action {
                        Action::CursorDown => (*cursor + 1).min(len - 1),
                        _ => cursor.saturating_sub(1),
                    };
                    return true;
                }
                if let Some(Overlay::Orders { timeline, cursor }) = &mut self.state.overlay {
                    let len = timeline.events.len();
                    if len == 0 {
                        return true;
                    }
                    *cursor = match action {
                        Action::CursorDown => (*cursor + 1).min(len - 1),
                        _ => cursor.saturating_sub(1),
                    };
                    return true;
                }
                if let Some(Overlay::Marks { cursor }) = &mut self.state.overlay {
                    let len = self.state.bookmarks.len();
                    if len == 0 {
                        return true;
                    }
                    *cursor = match action {
                        Action::CursorDown => (*cursor + 1).min(len - 1),
                        _ => cursor.saturating_sub(1),
                    };
                    return true;
                }
                if let Some(Overlay::Help { scroll }) = &mut self.state.overlay {
                    *scroll = match action {
                        Action::CursorDown => scroll.saturating_add(1),
                        _ => scroll.saturating_sub(1),
                    };
                    return true;
                }
                // Histogram / Diff: no intra-overlay nav
                // wired yet; let the action through so the main list still
                // responds (though visually the overlay hides it).
                false
            }
            Action::CursorHalfPageDown | Action::CursorHalfPageUp => {
                if let Some(Overlay::Orders { timeline, cursor }) = &mut self.state.overlay {
                    let len = timeline.events.len();
                    if len == 0 {
                        return true;
                    }
                    const STEP: usize = 10;
                    *cursor = match action {
                        Action::CursorHalfPageDown => (*cursor + STEP).min(len - 1),
                        _ => cursor.saturating_sub(STEP),
                    };
                    return true;
                }
                if let Some(Overlay::Marks { cursor }) = &mut self.state.overlay {
                    let len = self.state.bookmarks.len();
                    if len == 0 {
                        return true;
                    }
                    const STEP: usize = 10;
                    *cursor = match action {
                        Action::CursorHalfPageDown => (*cursor + STEP).min(len - 1),
                        _ => cursor.saturating_sub(STEP),
                    };
                    return true;
                }
                if let Some(Overlay::Help { scroll }) = &mut self.state.overlay {
                    const STEP: u16 = 10;
                    *scroll = match action {
                        Action::CursorHalfPageDown => scroll.saturating_add(STEP),
                        _ => scroll.saturating_sub(STEP),
                    };
                    return true;
                }
                false
            }
            Action::CursorTop | Action::CursorBottom => {
                if let Some(Overlay::Orders { timeline, cursor }) = &mut self.state.overlay {
                    let len = timeline.events.len();
                    if len > 0 {
                        *cursor = match action {
                            Action::CursorTop => 0,
                            _ => len - 1,
                        };
                    }
                    return true;
                }
                if let Some(Overlay::Marks { cursor }) = &mut self.state.overlay {
                    let len = self.state.bookmarks.len();
                    if len > 0 {
                        *cursor = match action {
                            Action::CursorTop => 0,
                            _ => len - 1,
                        };
                    }
                    return true;
                }
                if let Some(Overlay::Help { scroll }) = &mut self.state.overlay {
                    *scroll = match action {
                        Action::CursorTop => 0,
                        // Clamp to content length so G lands exactly at the
                        // last line even with a tall terminal.
                        _ => u16::try_from(crate::view::help::content_len().saturating_sub(1))
                            .unwrap_or(u16::MAX),
                    };
                    return true;
                }
                false
            }
            Action::OverlayApply => {
                if matches!(self.state.overlay, Some(Overlay::Sessions { .. })) {
                    self.apply_session_filter();
                } else if matches!(self.state.overlay, Some(Overlay::Orders { .. })) {
                    self.jump_to_orders_selection();
                } else if matches!(self.state.overlay, Some(Overlay::Marks { .. })) {
                    self.jump_to_marks_selection();
                }
                true
            }
            _ => false,
        }
    }

    /// `Enter` inside the Orders overlay: jump the main cursor to the
    /// ordinal of the selected event and close the overlay so the user
    /// lands on that message in the list/detail panes. If the event's
    /// ordinal is outside the currently filtered view, warn the user —
    /// we don't silently clear filters.
    fn jump_to_orders_selection(&mut self) {
        let Some(Overlay::Orders { timeline, cursor }) = self.state.overlay.clone() else {
            return;
        };
        let Some(event) = timeline.events.get(cursor) else {
            self.state.overlay = None;
            return;
        };
        let target = event.ordinal;
        if let Some(idx) = self.state.visible.iter().position(|&o| o == target) {
            self.state.cursor = idx;
            self.state.mode = ViewMode::Browse;
            self.state.overlay = None;
            self.state.status = StatusMessage::info(format!("jumped to #{target}"));
        } else {
            self.state.status = StatusMessage::warn(format!(
                "message #{target} is hidden by current filter — clear filter to see it"
            ));
        }
    }

    /// `Enter` inside the Marks overlay: jump the main cursor to the
    /// ordinal of the selected bookmark and close the overlay. Entries
    /// are sorted by mark character (matches the overlay's render
    /// order). If the ordinal is outside the current filtered view,
    /// warn instead of silently clearing the filter.
    fn jump_to_marks_selection(&mut self) {
        let Some(Overlay::Marks { cursor }) = self.state.overlay else {
            return;
        };
        let mut entries: Vec<(char, u32)> =
            self.state.bookmarks.iter().map(|(c, o)| (*c, *o)).collect();
        entries.sort_by_key(|(c, _)| *c);
        let Some((c, target)) = entries.get(cursor).copied() else {
            self.state.overlay = None;
            return;
        };
        if let Some(idx) = self.state.visible.iter().position(|&o| o == target) {
            self.state.cursor = idx;
            self.state.mode = ViewMode::Browse;
            self.state.overlay = None;
            self.state.status = StatusMessage::info(format!("jumped to '{c}' (#{target})"));
        } else {
            self.state.status = StatusMessage::warn(format!(
                "mark '{c}' (#{target}) is hidden by current filter — clear filter to see it"
            ));
        }
    }

    /// Apply a filter derived from the sessions overlay selection and close
    /// the overlay. No-op if there's no sessions overlay currently open.
    pub fn apply_session_filter(&mut self) {
        let Some(Overlay::Sessions { map, cursor }) = self.state.overlay.clone() else {
            return;
        };
        let Some((sender, target)) = session_at(&map, cursor) else {
            self.state.overlay = None;
            return;
        };
        let expr_text = format!(
            "49={} AND 56={}",
            String::from_utf8_lossy(&sender),
            String::from_utf8_lossy(&target),
        );
        match parse_query(&expr_text) {
            Ok(_) => {
                self.state.user_filter_text = Some(expr_text);
                recompute_effective_filter(&mut self.state);
                self.state.overlay = None;
                self.state.status =
                    StatusMessage::info(format!("filter: {} match", self.state.visible.len()));
            }
            Err(e) => {
                self.state.status =
                    StatusMessage::error(format!("bad session filter {expr_text:?}: {e}"));
            }
        }
    }

    fn yank_raw(&mut self) {
        if self.state.visible.is_empty() {
            self.state.status = StatusMessage::warn("nothing to yank");
            return;
        }
        let ord = self.state.visible[self.state.cursor] as usize;
        let Some(bytes) = self.state.index.message_bytes(&self.state.mmap, ord) else {
            self.state.status = StatusMessage::error("cursor out of range");
            return;
        };
        let text = clipboard::raw_to_text(bytes);
        match clipboard::copy(&text) {
            Ok(()) => {
                self.state.status =
                    StatusMessage::info(format!("yanked raw ({} bytes)", bytes.len()))
            }
            Err(e) => self.state.status = StatusMessage::error(e),
        }
    }

    fn yank_pretty(&mut self) {
        // Make sure the cache is fresh before reading it.
        self.state.refresh_detail_cache();
        let text = match &self.state.detail_cache {
            Some((_, Ok(resolved))) => clipboard::pretty_text(resolved),
            Some((_, Err(e))) => {
                self.state.status = StatusMessage::error(format!("cannot yank: {e}"));
                return;
            }
            None => {
                self.state.status = StatusMessage::warn("nothing to yank");
                return;
            }
        };
        match clipboard::copy(&text) {
            Ok(()) => self.state.status = StatusMessage::info("yanked pretty"),
            Err(e) => self.state.status = StatusMessage::error(e),
        }
    }

    fn submit_search(&mut self) {
        let raw = std::mem::take(&mut self.state.search_buffer);
        self.state.input_mode = InputMode::Normal;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return;
        }
        match parse_query(trimmed) {
            Ok(expr) => {
                let hit = search::next_match(&mut self.state, &expr, Direction::Forward);
                self.state.search_last = Some(expr);
                self.state.search_last_text = Some(trimmed.to_string());
                self.report_hit(hit);
                if matches!(hit, Hit::Match { .. }) {
                    // Leaving Follow because the user explicitly moved via search.
                    self.state.mode = ViewMode::Browse;
                }
            }
            Err(e) => {
                self.state.status = StatusMessage::error(format!("invalid search: {e}"));
            }
        }
    }

    fn iterate_search(&mut self, dir: Direction) {
        // `search_last` already holds the compiled expression — no need to
        // re-parse from `search_last_text` (which is retained only for the
        // status bar).
        let Some(expr) = self.state.search_last.clone() else {
            self.state.status = StatusMessage::warn("no previous search");
            return;
        };
        let hit = search::next_match(&mut self.state, &expr, dir);
        self.report_hit(hit);
        if matches!(hit, Hit::Match { .. }) {
            self.state.mode = ViewMode::Browse;
        }
    }

    fn report_hit(&mut self, hit: Hit) {
        match hit {
            Hit::NoMatch => {
                self.state.status = StatusMessage::warn("no match");
            }
            Hit::Match { wrapped: true, .. } => {
                self.state.status = StatusMessage::info("search wrapped");
            }
            Hit::Match { wrapped: false, .. } => {}
        }
    }

    fn submit_command(&mut self) {
        let raw = std::mem::take(&mut self.state.command_buffer);
        self.state.input_mode = InputMode::Normal;
        self.state.command_history_idx = None;
        // Live preview committed — drop the snapshot so Esc on a future
        // command doesn't roll back this one.
        self.state.filter_snapshot = None;

        let trimmed = raw.trim();
        if !trimmed.is_empty()
            && self.state.command_history.last().map(|s| s.as_str()) != Some(trimmed)
        {
            self.state.command_history.push(trimmed.to_string());
        }

        let cmd = command::parse(&raw);
        match command::execute(&mut self.state, cmd) {
            Outcome::Quit => self.should_quit = true,
            Outcome::Continue => {}
        }
    }

    fn live_preview(&mut self) {
        let buffer = self.state.command_buffer.clone();
        command::live_preview(&mut self.state, &buffer);
    }

    fn nav_history(&mut self, dir: HistoryDir) {
        if self.state.command_history.is_empty() {
            return;
        }
        let len = self.state.command_history.len();
        let new_idx = match (self.state.command_history_idx, dir) {
            (None, HistoryDir::Prev) => Some(len - 1),
            (Some(0), HistoryDir::Prev) => Some(0),
            (Some(i), HistoryDir::Prev) => Some(i - 1),
            (None, HistoryDir::Next) => None,
            (Some(i), HistoryDir::Next) if i + 1 >= len => None,
            (Some(i), HistoryDir::Next) => Some(i + 1),
        };
        self.state.command_history_idx = new_idx;
        self.state.command_buffer = match new_idx {
            Some(i) => self.state.command_history[i].clone(),
            None => String::new(),
        };
    }
}

#[derive(Debug, Clone, Copy)]
enum HistoryDir {
    Prev,
    Next,
}

/// How the cursor should jump in response to a navigation action.
#[derive(Debug, Clone, Copy)]
enum MoveDelta {
    Delta(i64),
    Top,
    Bottom,
    HalfPageDown,
    HalfPageUp,
}

/// Apply a cursor movement to `state`, clamping to `visible` and flipping
/// `ViewMode` to keep the follow-mode contract: any movement except
/// `Bottom` drops out of `Follow`, and `Bottom` enters it.
fn move_cursor(state: &mut AppState, delta: MoveDelta) {
    if state.visible.is_empty() {
        return;
    }
    let last = state.visible.len() - 1;
    let step = state.last_list_height.max(1) / 2;

    let new_cursor = match delta {
        MoveDelta::Delta(d) => offset_cursor(state.cursor, d, last),
        MoveDelta::Top => 0,
        MoveDelta::Bottom => last,
        MoveDelta::HalfPageDown => offset_cursor(state.cursor, step as i64, last),
        MoveDelta::HalfPageUp => offset_cursor(state.cursor, -(step as i64), last),
    };
    state.cursor = new_cursor;

    state.mode = match delta {
        MoveDelta::Bottom => ViewMode::Follow,
        _ => ViewMode::Browse,
    };
}

/// Base horizontal-scroll step, in columns. Applied per keypress to
/// whichever panel-specific offset (`list_h_offset` or `detail_h_offset`)
/// the action targets.
const SCROLL_STEP: u16 = 8;

/// The focus actually in effect for nav routing. When raw mode is on the
/// list isn't rendered, so `j`/`k` must scroll the detail regardless of
/// what the user-visible `state.focus` says.
fn effective_focus(state: &AppState) -> Focus {
    if state.raw_detail_mode {
        Focus::Detail
    } else {
        state.focus
    }
}

/// Navigation dispatch: routes `j/k/g/G/Ctrl+D/U` to the currently
/// focused panel.
///
/// - `Focus::List`: cursor moves through messages (existing behavior,
///   delegates to [`move_cursor`] so `ViewMode` flipping stays correct).
/// - `Focus::Detail`: shift `state.detail_v_offset` — the user is reading
///   field rows within the same message. `G` sets to `u16::MAX` and the
///   detail renderer clamps during its pass based on `last_detail_height`
///   and the cached field count.
fn nav(state: &mut AppState, delta: MoveDelta) {
    match effective_focus(state) {
        Focus::List => move_cursor(state, delta),
        Focus::Detail => detail_scroll(state, delta),
    }
}

fn detail_scroll(state: &mut AppState, delta: MoveDelta) {
    // When raw mode is on we don't have a field-level cursor; treat
    // j/k/g/G as a plain paragraph scroll.
    if state.raw_detail_mode {
        let step = state.last_detail_height.max(1) / 2;
        let current = state.detail_v_offset;
        state.detail_v_offset = match delta {
            MoveDelta::Delta(d) => signed_saturating_add(current, d),
            MoveDelta::Top => 0,
            MoveDelta::Bottom => u16::MAX,
            MoveDelta::HalfPageDown => signed_saturating_add(current, step as i64),
            MoveDelta::HalfPageUp => signed_saturating_add(current, -(step as i64)),
        };
        return;
    }

    // Resolved mode: move the per-field `detail_cursor`, then let the
    // renderer auto-scroll `detail_v_offset` to keep it visible.
    let field_count = state.detail_fields_len();
    if field_count == 0 {
        return;
    }
    let last = field_count - 1;
    let step = state.last_detail_height.max(1) / 2;
    state.detail_cursor = match delta {
        MoveDelta::Delta(d) => offset_cursor(state.detail_cursor, d, last),
        MoveDelta::Top => 0,
        MoveDelta::Bottom => last,
        MoveDelta::HalfPageDown => offset_cursor(state.detail_cursor, step as i64, last),
        MoveDelta::HalfPageUp => offset_cursor(state.detail_cursor, -(step as i64), last),
    };

    // Bring `detail_cursor` into the viewport. `last_detail_height` is
    // "viewport rows including header"; the renderer subtracts 1 for the
    // table header, so data rows ≈ height - 1.
    let data_rows = state.last_detail_height.saturating_sub(1).max(1);
    let top = state.detail_v_offset as usize;
    if state.detail_cursor < top {
        state.detail_v_offset = state.detail_cursor as u16;
    } else if state.detail_cursor >= top + data_rows {
        state.detail_v_offset =
            u16::try_from(state.detail_cursor + 1 - data_rows).unwrap_or(u16::MAX);
    }
}

/// Build a filter predicate string from a `(tag, raw_value)` pair. Uses a
/// bareword when the value is ASCII-safe (no whitespace, no DSL
/// operators, no quote/backslash); otherwise emits a quoted string with
/// `"` and `\\` escaped. Returns `None` if the value contains a byte that
/// can't be represented in either form (NUL, control chars).
fn filter_expr_from_field(tag: u32, value: &[u8], negated: bool) -> Option<String> {
    // Reject control bytes and non-UTF-8; the DSL is string-oriented.
    let text = std::str::from_utf8(value).ok()?;
    if text.chars().any(|c| c.is_control() && c != ' ') {
        return None;
    }

    let safe_bareword = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '+'));
    let predicate = if safe_bareword {
        format!("{tag}={text}")
    } else {
        let escaped: String = text
            .chars()
            .flat_map(|c| match c {
                '\\' => vec!['\\', '\\'],
                '"' => vec!['\\', '"'],
                c => vec![c],
            })
            .collect();
        format!("{tag}=\"{escaped}\"")
    };
    if negated {
        Some(format!("NOT ({predicate})"))
    } else {
        Some(predicate)
    }
}

fn signed_saturating_add(base: u16, delta: i64) -> u16 {
    let n = base as i64 + delta;
    if n < 0 {
        0
    } else if n > u16::MAX as i64 {
        u16::MAX
    } else {
        n as u16
    }
}

fn offset_cursor(cursor: usize, delta: i64, last: usize) -> usize {
    let n = cursor as i64 + delta;
    if n < 0 {
        0
    } else if n as usize > last {
        last
    } else {
        n as usize
    }
}

/// Flip between `Follow` and `Browse`. Going back into `Follow` snaps the
/// cursor to the end of `visible` and clears the "new since browse"
/// counter — the user has caught up.
fn toggle_mode(state: &mut AppState) {
    match state.mode {
        ViewMode::Follow => {
            state.mode = ViewMode::Browse;
        }
        ViewMode::Browse => {
            state.mode = ViewMode::Follow;
            state.new_since_browse = 0;
            if !state.visible.is_empty() {
                state.cursor = state.visible.len() - 1;
            }
        }
    }
}

/// Hook called by the `--follow` watcher (lands in P3-T13) after the
/// underlying index grows by `delta` messages. In `Follow` we keep the
/// cursor glued to the end; in `Browse` we simply tally new arrivals so
/// the list view can render the `⬇ N new` indicator without interrupting
/// the user's reading position.
pub fn on_index_grew(state: &mut AppState, delta: usize) {
    if delta == 0 {
        return;
    }
    match state.mode {
        ViewMode::Follow => {
            state.new_since_browse = 0;
            if !state.visible.is_empty() {
                state.cursor = state.visible.len() - 1;
            }
        }
        ViewMode::Browse => {
            state.new_since_browse = state.new_since_browse.saturating_add(delta as u32);
        }
    }
}
