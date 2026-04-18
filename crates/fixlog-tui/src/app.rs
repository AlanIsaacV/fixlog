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
use crate::input::{Action, map_event};
use crate::search::{self, Direction, Hit};
use crate::state::{
    AppState, Focus, InputMode, Overlay, StatusMessage, ViewMode, bootstrap,
    recompute_effective_filter, restore_filter, snapshot_filter,
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
        let state = bootstrap(&cfg.path, cfg.initial_filter.as_deref())?;
        Ok(Self {
            state,
            should_quit: false,
        })
    }

    /// Dispatch one event: map it to an [`Action`] and apply it.
    pub fn on_event(&mut self, ev: &Event) {
        let action = map_event(ev, self.state.input_mode);
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
                // Bare `D` without a prior `d` is meaningless here.
                self.state.status = StatusMessage::warn("press dD after dd to open diff");
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
        }
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
        if !c.is_ascii_alphabetic() {
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
                // Orders / Histogram / Marks / Diff: no intra-overlay nav
                // wired yet; let the action through so the main list still
                // responds (though visually the overlay hides it).
                false
            }
            Action::OverlayApply => {
                if matches!(self.state.overlay, Some(Overlay::Sessions { .. })) {
                    self.apply_session_filter();
                }
                true
            }
            _ => false,
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
        let Some(text) = self.state.search_last_text.clone() else {
            self.state.status = StatusMessage::warn("no previous search");
            return;
        };
        let Ok(expr) = parse_query(&text) else {
            self.state.status = StatusMessage::error("previous search no longer parses");
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
    let step = state.last_detail_height.max(1) / 2;
    let current = state.detail_v_offset;
    state.detail_v_offset = match delta {
        MoveDelta::Delta(d) => signed_saturating_add(current, d),
        MoveDelta::Top => 0,
        // Let the renderer clamp `u16::MAX` down to the real last row
        // based on field count — the dispatch doesn't know how many
        // fields are in the cached message without re-parsing.
        MoveDelta::Bottom => u16::MAX,
        MoveDelta::HalfPageDown => signed_saturating_add(current, step as i64),
        MoveDelta::HalfPageUp => signed_saturating_add(current, -(step as i64)),
    };
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
