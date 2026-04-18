//! TUI application state and bootstrap logic.
//!
//! [`AppState`] is the full runtime model the render path reads from and that
//! input handlers mutate. It's deliberately a plain struct — no interior
//! mutability, no trait boundaries — because the event loop is single-threaded.
//!
//! The only shared-ownership shape is `Arc<Mmap>`: in `--follow` mode we
//! re-mmap the file on growth and need to swap the backing buffer atomically
//! so any in-flight borrow (e.g. a detail-panel cache holding `&mmap[..]`) can
//! still drop cleanly. The `Arc` lets the old mapping outlive the swap until
//! its last borrower goes away.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use memmap2::Mmap;

use fixlog_analysis::histogram::Histogram;
use fixlog_analysis::orders::OrderTimeline;
use fixlog_analysis::sessions::SessionMap;
use fixlog_core::QueryExpr;
use fixlog_core::dict::{ResolvedMessage, chain_field_by_tag, resolve};
use fixlog_core::format::{LogFormat, sniff};
use fixlog_core::index::{LogIndex, build_from_bytes_parallel};
use fixlog_core::parser::parse_one_with_format;
use fixlog_core::query::parse as parse_query;

use crate::io::{head, mmap_file};

/// Scroll mode. `Follow` keeps the cursor anchored to the end of `visible` so
/// incoming messages stay on screen; `Browse` freezes the cursor so the user
/// can read past messages without being yanked forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Follow,
    Browse,
}

/// Which panel receives navigation keys (`j/k/g/G/Ctrl+D/Ctrl+U`, arrows).
/// Switched with `Tab` / `Shift+Tab` in Normal mode. In raw detail mode the
/// list isn't rendered, so the app.rs dispatch treats the effective focus
/// as `Detail` regardless of this field's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Detail,
}

/// Input mode.
///
/// - `Normal`: navigation + vim bindings.
/// - `Command`: keystrokes append to `command_buffer`; `:q`, `:filter …`.
/// - `Search`: keystrokes append to `search_buffer`; Enter jumps the cursor
///   to the next match, then `n`/`N` iterate without re-opening the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Command,
    Search,
}

/// Severity of a transient message shown in the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusLevel {
    Info,
    Warn,
    Error,
}

/// Transient message rendered in the status bar.
///
/// Used for "filter applied", "search wrapped", "clipboard unavailable", etc.
/// `expires_at` is driven by the event loop's tick; when elapsed, the status
/// bar reverts to its default summary.
#[derive(Debug, Clone)]
pub struct StatusMessage {
    pub text: String,
    pub level: StatusLevel,
    pub expires_at: Option<Instant>,
}

impl Default for StatusMessage {
    fn default() -> Self {
        Self {
            text: String::new(),
            level: StatusLevel::Info,
            expires_at: None,
        }
    }
}

impl StatusMessage {
    pub fn info(text: impl Into<String>) -> Self {
        Self::transient(text, StatusLevel::Info, Duration::from_secs(3))
    }

    pub fn warn(text: impl Into<String>) -> Self {
        Self::transient(text, StatusLevel::Warn, Duration::from_secs(3))
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::transient(text, StatusLevel::Error, Duration::from_secs(5))
    }

    fn transient(text: impl Into<String>, level: StatusLevel, ttl: Duration) -> Self {
        Self {
            text: text.into(),
            level,
            expires_at: Some(Instant::now() + ttl),
        }
    }

    pub fn is_active(&self, now: Instant) -> bool {
        match self.expires_at {
            None => !self.text.is_empty(),
            Some(exp) => now < exp,
        }
    }
}

/// Owned counterpart of [`fixlog_core::dict::ResolvedField`] — the field's
/// `value` is copied into a `Vec<u8>` instead of borrowing the mmap. Needed
/// because the mmap can be swapped under `--follow`; a borrowed cache would
/// outlive the buffer it borrowed from.
#[derive(Debug, Clone)]
pub struct ResolvedFieldOwned {
    pub tag: u32,
    pub name: Option<&'static str>,
    pub field_type: Option<&'static str>,
    pub value: Vec<u8>,
    pub value_label: Option<&'static str>,
}

/// Owned counterpart of [`fixlog_core::dict::ResolvedMessage`]. See
/// [`ResolvedFieldOwned`] for the reason we materialise.
#[derive(Debug, Clone)]
pub struct ResolvedMessageOwned {
    pub offset: u64,
    pub msg_type_name: Option<&'static str>,
    pub fields: Vec<ResolvedFieldOwned>,
}

impl ResolvedMessageOwned {
    pub fn from_resolved(resolved: ResolvedMessage<'_>) -> Self {
        let chain = resolved.chain;
        let fields = resolved
            .fields
            .iter()
            .map(|f| ResolvedFieldOwned {
                tag: f.tag,
                name: f.name,
                field_type: chain_field_by_tag(chain, f.tag).map(|d| d.field_type),
                value: f.value.to_vec(),
                value_label: f.value_label,
            })
            .collect();
        Self {
            offset: resolved.offset,
            msg_type_name: resolved.msg_type_name,
            fields,
        }
    }
}

/// Cached detail-panel payload. Key is the ordinal in `index.messages`; we
/// store `Err(msg)` for messages that failed to parse so we don't re-try the
/// parse on every frame.
pub type DetailEntry = (u32, Result<ResolvedMessageOwned, String>);

/// Snapshot of filter-affected state captured when the user enters command
/// mode so we can restore it if the typed command is cancelled. Stores the
/// filter *source text* rather than the compiled expression (which isn't
/// `Clone`) — [`restore_filter`] re-parses on the way back.
#[derive(Debug, Clone)]
pub struct FilterSnapshot {
    pub filter_text: Option<String>,
    pub user_filter_text: Option<String>,
    pub hide_heartbeat: bool,
    pub visible: Vec<u32>,
    pub cursor: usize,
    pub viewport_top: usize,
}

/// Stacked-single overlay. At any time the TUI may have **zero or one**
/// overlay active over the main list/detail layout. Use `Esc` to close.
///
/// Sessions / Orders / Diff / Marks / Histogram all reuse the same
/// `overlay` field rather than introducing new `ViewMode` variants; the
/// main navigation state (follow/browse) is orthogonal.
#[derive(Debug, Clone)]
pub enum Overlay {
    Sessions {
        map: SessionMap,
        cursor: usize,
    },
    Orders {
        timeline: OrderTimeline,
        scroll: usize,
    },
    Diff,
    Marks,
    Histogram {
        histogram: Histogram,
        width: usize,
    },
}

/// Runtime state. Fields are `pub` so view modules can read freely; all
/// mutations go through dedicated methods on the input/filter modules.
pub struct AppState {
    /// Path of the file being viewed. Held so the header can show it and the
    /// `--follow` watcher can re-open on rotation.
    pub path: PathBuf,
    /// Backing file buffer. Swapped (`Arc` replaced) when the file grows under
    /// `--follow`; old instance stays alive while any borrow outlives the swap.
    pub mmap: Arc<Mmap>,
    pub format: LogFormat,
    pub index: LogIndex,
    /// Active filter expression, compiled once. `None` means "show everything".
    pub filter: Option<QueryExpr>,
    /// Source text of the active filter, kept in sync with `filter`. Used
    /// for status-bar display and for snapshot/restore on cancelled
    /// live-preview edits. This is the **effective** filter — the raw text
    /// the engine evaluates, which may differ from what the user typed
    /// when `hide_heartbeat` is on (see [`recompute_effective_filter`]).
    pub filter_text: Option<String>,
    /// The expression the user actually typed into `:f <expr>` (or via the
    /// initial `--filter` flag). `filter_text` is derived from this plus
    /// `hide_heartbeat`. `None` means "no user-supplied filter".
    pub user_filter_text: Option<String>,
    /// When true, `NOT 35=0` is ANDed into the effective filter so
    /// Heartbeat messages drop out of `visible`. Toggled with `H`.
    pub hide_heartbeat: bool,
    /// Ordinals into `index.messages` that pass `filter`. Re-evaluated whenever
    /// the filter changes or an append lands that could add more matches.
    pub visible: Vec<u32>,
    /// Cursor position *within* `visible`. Range: `0..visible.len()`, with
    /// `0` when `visible` is empty.
    pub cursor: usize,
    /// First row of `visible` drawn in the list viewport. The list view is
    /// responsible for keeping `cursor` inside `[viewport_top, viewport_top+h)`.
    pub viewport_top: usize,
    pub mode: ViewMode,
    /// Count of messages appended since the user entered `Browse`. Rendered
    /// as `⬇ N new` and cleared on mode switch back to `Follow`.
    pub new_since_browse: u32,
    pub status: StatusMessage,
    /// Cached resolved representation of the message currently under the
    /// cursor. `None` when nothing is selected; keyed by ordinal so the
    /// detail view only re-resolves when the user moves.
    pub detail_cache: Option<DetailEntry>,
    /// Current input mode — `Normal` (navigation keys) vs `Command`
    /// (keystrokes append to `command_buffer`).
    pub input_mode: InputMode,
    /// Buffer for the command line. Always empty in `Normal` mode; holds
    /// characters typed after `:` in `Command` mode.
    pub command_buffer: String,
    /// History of executed commands. Traversed with `↑/↓` while editing.
    pub command_history: Vec<String>,
    /// Position in `command_history` while browsing; `None` when editing
    /// a fresh line.
    pub command_history_idx: Option<usize>,
    /// Height (rows) of the list viewport from the most recent render.
    /// `Ctrl+D` / `Ctrl+U` use this as the half-page step. Updated by
    /// `view::list::render`; the event loop reads it.
    pub last_list_height: usize,
    /// Pre-command snapshot of filter + visible + cursor, captured when the
    /// user opens the command bar with `:`. Cleared on submit. Used by the
    /// live-filter preview (P3-T09) to roll back on `Esc`.
    pub filter_snapshot: Option<FilterSnapshot>,
    /// Single-character prefix waiting for a completing key. Used to
    /// implement multi-key bindings like `yy` / `yY`. Cleared by any
    /// non-continuation key.
    pub pending_prefix: Option<char>,
    /// Buffer being edited while `input_mode == Search`.
    pub search_buffer: String,
    /// Last committed search expression, re-used by `n`/`N`.
    pub search_last: Option<QueryExpr>,
    /// Source text of `search_last`, kept for status-bar display.
    pub search_last_text: Option<String>,
    /// Active overlay (`:sessions`, `O`/`:orders`, diff, bookmarks,
    /// `:histogram`). `None` when only the main layout is visible.
    pub overlay: Option<Overlay>,
    /// Diff slots selected via `dd` (slot A) and `dD` (slot B). Each is an
    /// ordinal; once both are set, the diff overlay can be opened.
    pub diff_slots: [Option<u32>; 2],
    /// Letter-keyed bookmarks. Value is an ordinal into `index.messages`.
    pub bookmarks: HashMap<char, u32>,
    /// When true, the detail view hides session-layer header/trailer tags
    /// (`8, 9, 10, 34, 35, 49, 52, 56`) so the payload is easier to scan.
    /// Toggled with `c` in Normal mode.
    pub skip_common: bool,
    /// When true, the detail panel renders the raw FIX bytes of the message
    /// under the cursor (SOH → `|`, non-printable → `.`) instead of the
    /// resolved tag table. The raw view wraps (no horizontal scroll) and
    /// expands to the full body width so the list below it can't bleed
    /// into the user's terminal selection. Toggled with `r` in Normal mode.
    pub raw_detail_mode: bool,
    /// Horizontal scroll offset (in columns) applied to the list panel
    /// via `Paragraph::scroll`. Independent of `detail_h_offset` so each
    /// panel can be scrolled without disturbing the other.
    /// Driven by `Left`/`Right` in Normal mode; `0` resets it.
    pub list_h_offset: u16,
    /// Horizontal scroll offset (in columns) applied to the detail panel
    /// when rendering resolved-mode fields. Raw mode ignores this — it
    /// wraps instead. Driven by arrow keys when focus is on Detail; `0`
    /// resets.
    pub detail_h_offset: u16,
    /// Vertical scroll offset (in rows) applied to the detail panel so
    /// the user can see fields that fall below the viewport when a
    /// message has more fields than fit. Driven by `j`/`k`/`Ctrl+D/U`/
    /// `g`/`G` when focus is on Detail. Reset to 0 whenever the ordinal
    /// under the cursor changes (see [`AppState::refresh_detail_cache`]).
    pub detail_v_offset: u16,
    /// Height (rows) of the detail viewport from the most recent render.
    /// `Ctrl+D`/`Ctrl+U` use this for the half-page step, and the
    /// resolved-mode renderer uses it to clamp `detail_v_offset` so that
    /// `G` (set to `u16::MAX`) lands on the last field exactly.
    pub last_detail_height: usize,
    /// Which panel receives navigation keys. `Tab`/`Shift+Tab` toggles.
    pub focus: Focus,
}

impl AppState {
    /// Number of messages currently visible (after filtering).
    #[inline]
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// Clamp `cursor` to the valid range after `visible` changes. Callers
    /// that mutate `visible` should invoke this to avoid out-of-bounds reads.
    pub fn clamp_cursor(&mut self) {
        self.cursor = clamp_cursor(self.cursor, self.visible.len());
    }

    /// Re-populate `detail_cache` to match the current cursor. No-op if the
    /// cache already points at the right ordinal (cheap per-frame check).
    /// Called by the detail view before rendering; also called once at
    /// bootstrap so the first frame has something to show.
    pub fn refresh_detail_cache(&mut self) {
        if self.visible.is_empty() {
            self.detail_cache = None;
            return;
        }
        let ord = self.visible[self.cursor];
        if let Some((cached, _)) = &self.detail_cache
            && *cached == ord
        {
            return;
        }
        // New message under the cursor: restart detail-panel vertical
        // scroll so the user sees fields from the top.
        self.detail_v_offset = 0;
        let entry = resolve_ordinal(&self.mmap, &self.index, &self.format, ord);
        self.detail_cache = Some((ord, entry));
    }

    /// Adjust `viewport_top` so the cursor lies inside
    /// `[viewport_top, viewport_top + viewport_height)`. Called once per
    /// frame by the list view with the real height.
    pub fn ensure_cursor_visible(&mut self, viewport_height: usize) {
        if viewport_height == 0 || self.visible.is_empty() {
            self.viewport_top = 0;
            return;
        }
        if self.cursor < self.viewport_top {
            self.viewport_top = self.cursor;
        } else if self.cursor >= self.viewport_top + viewport_height {
            self.viewport_top = self.cursor + 1 - viewport_height;
        }
        let max_top = self.visible.len().saturating_sub(viewport_height);
        self.viewport_top = self.viewport_top.min(max_top);
    }
}

/// Pure helper behind [`AppState::clamp_cursor`]. Extracted so view and input
/// code can reason about the clamping rule without reaching through
/// `AppState`.
#[inline]
pub fn clamp_cursor(cursor: usize, visible_len: usize) -> usize {
    if visible_len == 0 {
        0
    } else {
        cursor.min(visible_len - 1)
    }
}

/// Load the file at `path`, sniff its format, build the full index in
/// parallel, and compile the optional initial filter. Returns a state with
/// cursor anchored to the end in `Follow` mode.
pub fn bootstrap(path: &Path, initial_filter: Option<&str>) -> Result<AppState> {
    let mmap = Arc::new(mmap_file(path)?);
    let sample = head(&mmap, 64 * 1024);
    let format = sniff(sample).with_context(|| format!("sniffing {}", path.display()))?;
    let index = build_from_bytes_parallel(&mmap, &format);

    let user_filter_text = initial_filter.map(|s| s.to_string());
    // Validate now so bootstrap still fails loudly on a malformed initial
    // filter; the effective filter is built below via compose/apply.
    if let Some(expr) = initial_filter {
        parse_query(expr).with_context(|| format!("parsing initial filter `{expr}`"))?;
    }

    let mut state = AppState {
        path: path.to_path_buf(),
        mmap,
        format,
        index,
        filter: None,
        filter_text: None,
        user_filter_text,
        hide_heartbeat: false,
        visible: Vec::new(),
        cursor: 0,
        viewport_top: 0,
        mode: ViewMode::Follow,
        new_since_browse: 0,
        status: StatusMessage::default(),
        detail_cache: None,
        input_mode: InputMode::Normal,
        command_buffer: String::new(),
        command_history: Vec::new(),
        command_history_idx: None,
        last_list_height: 0,
        filter_snapshot: None,
        pending_prefix: None,
        search_buffer: String::new(),
        search_last: None,
        search_last_text: None,
        overlay: None,
        diff_slots: [None, None],
        bookmarks: HashMap::new(),
        skip_common: false,
        raw_detail_mode: false,
        list_h_offset: 0,
        detail_h_offset: 0,
        detail_v_offset: 0,
        last_detail_height: 0,
        focus: Focus::List,
    };
    recompute_effective_filter(&mut state);
    // Bootstrap anchors to the end of `visible` in Follow mode.
    state.cursor = state.visible.len().saturating_sub(1);
    state.refresh_detail_cache();
    Ok(state)
}

fn resolve_ordinal(
    mmap: &[u8],
    index: &LogIndex,
    format: &LogFormat,
    ord: u32,
) -> Result<ResolvedMessageOwned, String> {
    let off = index
        .messages
        .get(ord as usize)
        .ok_or_else(|| format!("ordinal {ord} out of range"))?;
    let bytes = mmap
        .get(off.range())
        .ok_or_else(|| format!("offset {} out of buffer", off.start))?;
    let (raw, _) = parse_one_with_format(bytes, format).map_err(|e| format!("{e}"))?;
    Ok(ResolvedMessageOwned::from_resolved(resolve(&raw)))
}

/// Capture the state needed to revert a live filter preview. Clones only
/// the filter *text* (the compiled expression is rebuilt on restore).
pub fn snapshot_filter(state: &AppState) -> FilterSnapshot {
    FilterSnapshot {
        filter_text: state.filter_text.clone(),
        user_filter_text: state.user_filter_text.clone(),
        hide_heartbeat: state.hide_heartbeat,
        visible: state.visible.clone(),
        cursor: state.cursor,
        viewport_top: state.viewport_top,
    }
}

/// Replace the active filter with `expr` (paired with its source `text`)
/// and re-evaluate `visible`. Both may be `None` to clear the filter.
/// Resets cursor/viewport so the preview starts from a clean position.
pub fn apply_filter(state: &mut AppState, expr: Option<QueryExpr>, text: Option<String>) {
    state.filter = expr;
    state.filter_text = text;
    state.visible = evaluate_visible(
        &state.mmap,
        &state.index,
        &state.format,
        state.filter.as_ref(),
    );
    state.cursor = state.visible.len().saturating_sub(1);
    state.viewport_top = 0;
    state.detail_cache = None;
    state.refresh_detail_cache();
}

/// Restore a previous filter snapshot. Re-parses the stored text (which is
/// known good because we wouldn't have committed it otherwise); if parsing
/// somehow fails the filter is cleared rather than crashing.
pub fn restore_filter(state: &mut AppState, snap: FilterSnapshot) {
    let expr = snap.filter_text.as_ref().and_then(|t| parse_query(t).ok());
    state.filter = expr;
    state.filter_text = snap.filter_text;
    state.user_filter_text = snap.user_filter_text;
    state.hide_heartbeat = snap.hide_heartbeat;
    state.visible = snap.visible;
    state.cursor = snap.cursor;
    state.viewport_top = snap.viewport_top;
    state.detail_cache = None;
    state.refresh_detail_cache();
}

/// Compose the effective filter from `user_filter_text` and `hide_heartbeat`,
/// compile it, and apply it via [`apply_filter`].
///
/// Composition rules:
/// - user only: `<user>`
/// - hide_heartbeat only: `NOT 35=0`
/// - both: `(<user>) AND NOT 35=0`
/// - neither: no filter
///
/// Called whenever either input changes: the user submits `:f <expr>`,
/// `:filter` clears, live-preview types, or `H` toggles the heartbeat flag.
pub fn recompute_effective_filter(state: &mut AppState) {
    let effective_text = compose_effective(state.user_filter_text.as_deref(), state.hide_heartbeat);
    let (expr, text) = match effective_text {
        None => (None, None),
        Some(s) => match parse_query(&s) {
            Ok(compiled) => (Some(compiled), Some(s)),
            // Defensive: if the composed expression somehow fails to parse
            // (only plausible when the user's expression was injected raw
            // and had syntax issues), fall back to just the heartbeat mask
            // or no filter, so the TUI stays usable.
            Err(_) => (None, None),
        },
    };
    apply_filter(state, expr, text);
}

fn compose_effective(user: Option<&str>, hide_hb: bool) -> Option<String> {
    match (user, hide_hb) {
        (None, false) => None,
        (None, true) => Some("NOT 35=0".to_string()),
        (Some(u), false) => Some(u.to_string()),
        (Some(u), true) => Some(format!("({u}) AND NOT 35=0")),
    }
}

/// Compute the list of ordinals that pass `filter`.
///
/// **Fast path**: if `filter` is a pure AND of equality predicates and every
/// tag involved is in the secondary index, we intersect the sorted ordinal
/// lists from `SecondaryIndex::lookup(tag, value)`. This is O(output) instead
/// of O(messages) — the TUI filter bench on 1M msgs drops from ~477 ms to
/// under ~10 ms for `35=D`.
///
/// **Fallback**: anything with `Or`/`Not`/`Ne`/`Re`, or with a tag not in the
/// hot-tag set, re-parses every message and evaluates normally.
///
/// Messages that fail to parse are skipped silently — matches the indexer's
/// "warn + skip" contract and keeps the view stable over corrupt tails.
pub fn evaluate_visible(
    buf: &[u8],
    index: &LogIndex,
    format: &LogFormat,
    filter: Option<&QueryExpr>,
) -> Vec<u32> {
    let total = index.messages.len();
    let Some(expr) = filter else {
        return (0..total as u32).collect();
    };

    if let Some(eqs) = expr.hot_equalities()
        && eqs.iter().all(|(t, _)| index.secondary.has_tag(*t))
    {
        return intersect_secondary_lookups(&index.secondary, &eqs);
    }

    let mut visible = Vec::with_capacity(total / 4);
    for (ord, off) in index.messages.iter().enumerate() {
        let bytes = &buf[off.range()];
        if let Ok((msg, _)) = parse_one_with_format(bytes, format)
            && expr.matches(&msg)
        {
            visible.push(ord as u32);
        }
    }
    visible
}

/// Intersect the sorted ordinal lists produced by
/// [`SecondaryIndex::lookup`]. Each list is already sorted ascending (they
/// are built in ordinal order), so a linear k-way merge-intersect works.
fn intersect_secondary_lookups(
    sec: &fixlog_core::SecondaryIndex,
    eqs: &[(u32, &[u8])],
) -> Vec<u32> {
    if eqs.is_empty() {
        return Vec::new();
    }
    // Gather the ordinal slices and sort by length so we iterate the
    // shortest list as the "driver". This bounds the outer loop.
    let mut lists: Vec<&[u32]> = eqs.iter().map(|(t, v)| sec.lookup(*t, v)).collect();
    if lists.iter().any(|l| l.is_empty()) {
        return Vec::new();
    }
    lists.sort_by_key(|l| l.len());
    let (first, rest) = lists.split_first().unwrap();

    let mut out = Vec::with_capacity(first.len());
    for &candidate in *first {
        if rest.iter().all(|l| l.binary_search(&candidate).is_ok()) {
            out.push(candidate);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_info_sets_ttl() {
        let m = StatusMessage::info("hello");
        assert!(m.expires_at.is_some());
        assert_eq!(m.level, StatusLevel::Info);
    }

    #[test]
    fn status_default_is_inactive() {
        let m = StatusMessage::default();
        assert!(!m.is_active(Instant::now()));
    }

    #[test]
    fn clamp_cursor_empty() {
        assert_eq!(clamp_cursor(0, 0), 0);
        assert_eq!(clamp_cursor(42, 0), 0);
    }

    #[test]
    fn clamp_cursor_in_range_is_identity() {
        assert_eq!(clamp_cursor(0, 10), 0);
        assert_eq!(clamp_cursor(5, 10), 5);
        assert_eq!(clamp_cursor(9, 10), 9);
    }

    #[test]
    fn clamp_cursor_oob_trims_to_last() {
        assert_eq!(clamp_cursor(10, 10), 9);
        assert_eq!(clamp_cursor(1_000, 10), 9);
    }

    // Extracted-helper versions of ensure_cursor_visible so we can test the
    // scrolling rule without constructing a full AppState (which needs mmap).
    fn ensure_cursor_visible(
        cursor: usize,
        viewport_top: usize,
        visible_len: usize,
        viewport_height: usize,
    ) -> usize {
        if viewport_height == 0 || visible_len == 0 {
            return 0;
        }
        let mut top = viewport_top;
        if cursor < top {
            top = cursor;
        } else if cursor >= top + viewport_height {
            top = cursor + 1 - viewport_height;
        }
        let max_top = visible_len.saturating_sub(viewport_height);
        top.min(max_top)
    }

    #[test]
    fn ensure_visible_cursor_below_viewport_scrolls_down() {
        // cursor at 100, viewport at [0, 50) → scroll so cursor is last row.
        let top = ensure_cursor_visible(100, 0, 1000, 50);
        assert_eq!(top, 51);
    }

    #[test]
    fn ensure_visible_cursor_above_viewport_scrolls_up() {
        let top = ensure_cursor_visible(5, 50, 1000, 50);
        assert_eq!(top, 5);
    }

    #[test]
    fn ensure_visible_clamps_top_so_viewport_never_overruns() {
        // 10 items, 50-tall viewport, cursor at 9 → top must be 0.
        let top = ensure_cursor_visible(9, 0, 10, 50);
        assert_eq!(top, 0);
    }

    #[test]
    fn ensure_visible_handles_empty() {
        let top = ensure_cursor_visible(5, 3, 0, 20);
        assert_eq!(top, 0);
    }

    /// The hot-tag pushdown must produce byte-identical results to the
    /// full-scan path. Uses the real `fix44-om.log` fixture so the test
    /// exercises a realistic secondary index.
    #[test]
    fn hot_tag_pushdown_matches_full_scan() {
        use fixlog_core::query::parse as parse_query;
        use fixlog_core::{build_from_bytes, sniff};
        const FIX44_OM: &[u8] = include_bytes!("../../../fixtures/real/fix44-om.log");
        let fmt = sniff(FIX44_OM).unwrap();
        let index = build_from_bytes(FIX44_OM, &fmt);

        for expr_text in ["35=D", "35=8", "49=KALDMA1TRI", "35=D AND 49=KALDMA1TRI"] {
            let expr = parse_query(expr_text).unwrap();
            // Fast path (via `evaluate_visible`).
            let fast = evaluate_visible(FIX44_OM, &index, &fmt, Some(&expr));
            // Slow path: scan-only, re-parse every message.
            let mut slow: Vec<u32> = Vec::new();
            for (ord, off) in index.messages.iter().enumerate() {
                let bytes = &FIX44_OM[off.range()];
                if let Ok((msg, _)) = parse_one_with_format(bytes, &fmt)
                    && expr.matches(&msg)
                {
                    slow.push(ord as u32);
                }
            }
            assert_eq!(fast, slow, "mismatch for expr {expr_text}");
        }
    }
}
