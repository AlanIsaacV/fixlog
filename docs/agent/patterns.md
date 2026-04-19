# Cross-cutting patterns

Recurring idioms used across crates. Reference when adding new code so it fits the existing style.

## Zero-copy lifetime pattern

Core types borrow from the source buffer:

```rust
pub struct RawMessage<'a> {
    pub offset: u64,
    pub raw: &'a [u8],
    pub tags: SmallVec<[(u32, &'a [u8]); 32]>,
}
```

Lifetime rule: anything derived from a `RawMessage<'a>` that still references wire bytes must carry the same `'a`. `ResolvedMessage<'a>` follows this:

```rust
pub struct ResolvedField<'a> { pub value: &'a [u8], /* static strs for name/label */ }
pub struct ResolvedMessage<'a> { pub fields: Vec<ResolvedField<'a>>, /* … */ }
```

Static strings (`name: &'static str`, `value_label: Option<&'static str>`) come from the generated dictionary tables, so they don't extend the lifetime.

**Do not** collect into owned `Vec<u8>` / `String` inside hot paths. Materialize only at presentation boundaries (CLI output).

## Error handling split

- **Library crates** (`fixlog-parser`, `fixlog-format`, `fixlog-dict`, `fixlog-core`): public errors are `thiserror`-derived enums (`ParseError`, `SniffError`). `#![forbid(unsafe_code)]` at the top.
- **Binary crate** (`fixlog-cli`): uses `anyhow::Result` end-to-end. `.with_context(|| ...)` at I/O boundaries. `main` returns `anyhow::Result<()>`.
- **Tests**: `unwrap()` / `expect()` freely. Production code must not.

When library code wants to distinguish fatal vs non-fatal conditions, emit the non-fatal ones through `tracing::debug!` / `warn!` and return `Ok(...)` anyway. See the checksum-mismatch pattern in `crates/parser.md`.

## Tracing conventions

- `error!` — condition that prevents producing a result the caller asked for.
- `warn!` — recoverable anomaly the user probably wants to know about.
- `info!` — high-level milestones (e.g. "parsed N messages from FILE").
- `debug!` — fine-grained diagnostic; per-message events go here. Checksum mismatches use this level.
- `trace!` — not currently used; reserve for byte-level tracing.

CLI exposes `-v` (info), `-vv` (debug). Respects `RUST_LOG` env var via `EnvFilter::try_from_default_env()`.

## `memmap2` pattern

Binaries (`fixlog-cli`) and the TUI library (`fixlog-tui`) each own a tiny `io::mmap_file` that performs the single `unsafe` `Mmap::map`. Identical code in both places — duplicated rather than hoisted to `fixlog-core` until a third consumer appears.

```rust
let file = File::open(path)?;
// SAFETY: we only ever hand out shared references to the mapping.
let mmap = unsafe { Mmap::map(&file) }?;
```

Library crates below (parser, index, query, dict) take `&[u8]`. Never pass an `Mmap` around — deref it at the boundary.

### Unsafe lint boundary

`fixlog-parser`, `fixlog-format`, `fixlog-dict`, `fixlog-core` use `#![forbid(unsafe_code)]`. The TUI uses `#![deny(unsafe_code)]` at `lib.rs` plus a module-scoped `#[allow(unsafe_code)]` in `io.rs`. `forbid` is stricter and can't be opted out of; prefer it for pure library crates. Use `deny + allow` only when you actually need `unsafe` (mmap) but want to block accidental new uses.

## `Arc<Mmap>` swap for tailing (`--follow`)

Under `--follow` the file grows and we re-map on each poll. The TUI stores the buffer as `Arc<Mmap>` so the swap is atomic from the point of view of any cache still holding a reference:

```rust
pub struct AppState {
    pub mmap: Arc<Mmap>,           // swapped under --follow
    pub index: LogIndex,           // offsets are absolute, survive the swap
    pub detail_cache: Option<...>, // ResolvedMessageOwned — no borrow on mmap
    …
}

// on growth:
let new_mmap = Arc::new(mmap_file(&path)?);
state.mmap = new_mmap;  // old Arc dropped when last borrower releases
state.index.append_from_offset(&state.mmap, state.index.consumed, &state.format)?;
```

**Invariant**: nothing cached in `AppState` may borrow from `mmap`. Anything derived from a message that must survive the swap is materialised (`ResolvedMessageOwned` is the canonical example). Cheap for one message at a time, not cheap for the whole visible list — that's why rendering re-parses lazily from the current `mmap` each frame instead of caching.

See `crates/fixlog-tui/src/follow.rs`.

## Owned counterparts of borrowed types

When a borrowed struct like `ResolvedMessage<'a>` needs to survive longer than its backing buffer, pair it with an owned twin:

```rust
pub struct ResolvedFieldOwned { pub tag: u32, pub name: Option<&'static str>, pub value: Vec<u8>, … }
pub struct ResolvedMessageOwned { pub offset: u64, pub fields: Vec<ResolvedFieldOwned>, … }

impl ResolvedMessageOwned {
    pub fn from_resolved(r: ResolvedMessage<'_>) -> Self { /* copy into Vec<u8> */ }
}
```

Only materialise at the boundary where the lifetime becomes a problem (cache, cross-thread). Do not expose an `Owned` variant for the 99% path that stays synchronous and short-lived.

## Stat-based file watcher (vs `notify`)

For a single-threaded event loop that already blocks on `crossterm::event::poll(timeout)`, piggy-back on that timeout as a file-poll cadence:

```rust
pub struct FollowWatcher { path: PathBuf, last_poll: Option<Instant> }

impl FollowWatcher {
    pub fn poll(&mut self, state: &mut AppState) -> Result<()> {
        if let Some(last) = self.last_poll
            && Instant::now().duration_since(last) < POLL_INTERVAL { return Ok(()); }
        let current_len = fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        let mapped_len = state.mmap.len() as u64;
        if current_len == mapped_len { return Ok(()); }
        // grew / shrank paths: re-mmap + append, or rebuild
    }
}
```

Trade-off vs `notify`:
- Adds up to one `POLL_INTERVAL` (250 ms) of latency.
- No extra thread, no mpsc channel, no cross-platform event interpretation.
- Works fine on networked filesystems where `notify` is flaky.

Use `notify` when the consumer is a tight pipe (e.g. `grep --follow | jq`). Use stat-based when a human is in the loop. See `crates/fixlog-tui/src/follow.rs` (stat) vs `crates/fixlog-cli/src/commands/grep.rs` (notify).

## Building a synthetic FIX message for tests

Use this helper (see `crates/fixlog-dict/src/resolver.rs::tests::build_with_trailer`):

```rust
fn build_with_trailer(body: &[u8], begin_string: &[u8]) -> Vec<u8> {
    let mut head = Vec::new();
    head.extend_from_slice(b"8=");
    head.extend_from_slice(begin_string);
    head.push(0x01);
    head.extend_from_slice(format!("9={}\x01", body.len()).as_bytes());
    head.extend_from_slice(body);
    let cs = (head.iter().map(|&b| b as u32).sum::<u32>() % 256) as u8;
    head.extend_from_slice(format!("10={cs:03}\x01").as_bytes());
    head
}
```

`body` must be `tag=value<SOH>…` starting at `35=…` and ending with `<SOH>`. `BodyLength` = `body.len()`. Checksum is computed over `head` at the point just before the trailer is appended (i.e. includes everything from `8=` through the SOH that closes the last body tag).

## Finding a tag in a `RawMessage`

Linear scan over `msg.tags`. For most messages (<32 tags) this is a handful of comparisons, faster than building any map:

```rust
fn find_tag<'a>(msg: &RawMessage<'a>, tag: u32) -> Option<&'a [u8]> {
    msg.tags.iter().find(|(t, _)| *t == tag).map(|(_, v)| *v)
}
```

See `crates/fixlog-dict/src/resolver.rs` for the reference implementation.

## Writing to stdout from a command

`BufWriter` around `std::io::stdout().lock()` is the pattern. Flush at the end:

```rust
let stdout = std::io::stdout();
let mut out = BufWriter::new(stdout.lock());
/* write! writeln! write_all! */
out.flush()?;
```

This dominates over raw `println!` for any loop that emits many messages.

## Event-loop action dispatch

The TUI keeps all keybinding logic pure in `input.rs`:

```rust
pub fn map_event(ev: &Event, mode: InputMode) -> Action {
    match ev {
        Event::Key(k) => match mode {
            InputMode::Normal  => map_normal_key(k),
            InputMode::Command => map_command_key(k),
            InputMode::Search  => map_search_key(k),
        },
        Event::Resize(w, h) => Action::Resize(*w, *h),
        Event::Tick => Action::None,
    }
}
```

`App::apply(Action)` is the single site that mutates state. Bindings are tested without a live `AppState` by exercising `map_key` directly against synthesised `KeyEvent`s; state transitions are tested through `App` integration tests in `crates/fixlog-tui/tests/`.

**Rule**: do not branch on `KeyCode` inside `app.rs` — the action enum is the interface. Adding a binding means (1) new `Action` variant, (2) new arm in `map_*_key`, (3) new arm in `App::apply`.

## Multi-key prefixes (vim-style `yy` / `yY`)

For two-key sequences, keep the prefix in state (not in `input.rs`, which is stateless):

```rust
pub struct AppState { pub pending_prefix: Option<char>, … }

pub fn apply(&mut self, action: Action) {
    if let Some(prefix) = self.state.pending_prefix.take()
        && prefix == 'y'
    {
        match action {
            Action::YankPrefix => { self.yank_raw();    return; }
            Action::YankPretty => { self.yank_pretty(); return; }
            _ => { /* fall through, process action normally */ }
        }
    }
    // ... normal dispatch; a bare `y` sets pending_prefix = Some('y') ...
}
```

- `take()` the prefix at the top so it clears even if the action consumes it.
- If the action isn't a valid continuation, fall through and handle it as a fresh key press — `y` + `j` must still move the cursor down after clearing the prefix.
- No timeout needed for two-key sequences; vim doesn't timeout these either.

## Snapshot / restore for cancellable previews

When a UI mode previews mutations live (e.g. live filter preview in `:filter …`), snapshot the affected state on entry and restore on cancel:

```rust
pub struct FilterSnapshot {
    pub filter_text: Option<String>,  // source, not compiled (QueryExpr isn't Clone)
    pub visible: Vec<u32>,
    pub cursor: usize,
    pub viewport_top: usize,
}

// EnterCommand:
state.filter_snapshot = Some(snapshot_filter(&state));

// CommandCancel:
if let Some(snap) = state.filter_snapshot.take() { restore_filter(&mut state, snap); }

// CommandSubmit (commit):
state.filter_snapshot = None;
```

**Gotcha**: `fixlog_query::Expr` does not derive `Clone` (regex state, etc.). Snapshot the source text and re-parse on restore. Keep a `filter_text: Option<String>` field on state alongside the compiled expression to avoid losing the source.

## JSON escaping

Manual escape function in `crates/fixlog-cli/src/commands/parse.rs::write_json_string`. Handles: `"`, `\\`, `\n`, `\r`, `\t`, control chars (`\u00XX`), lossy UTF-8 via `String::from_utf8_lossy`. Do not pull in `serde_json` just for this — the output format is fixed and manual is ~25 LoC.

## Clippy gotchas encountered

- `collapsible_if` — nested `if let`s must use `&& let`:
  ```rust
  if let Some(x) = a && let Some(y) = b(x) { /* use y */ }
  ```
  Also applies to `if let Some(x) = a { if x == y { … } }` → rewrite as `if let Some(x) = a && x == y { … }`. Required in Rust 2024 edition.
- `useless_asref` — comparing `&[u8]` to `Vec<u8>::as_slice()`, don't `value.as_ref()`. Just `value`.
- `assertions_on_constants` — in test files, replace `#[test] fn f() { assert!(CONST >= N); }` with `const _: () = { assert!(CONST >= N); };` for compile-time checks (requires `const fn` accessors).
- `non_snake_case` applies to test function names too (workspace lints `-D warnings` catch them). Avoid `fn yY_copies_pretty()`; use `fn y_shift_y_copies_pretty()` or `#[allow(non_snake_case)]` per-test if the test name is load-bearing.

## `const fn` for compile-time assertions

Accessor functions that need to feed `const { assert!(...) }` must themselves be `const fn`. See `field_count` / `message_count` in `fixlog-dict`.

## Overlay state (Fase 4)

The TUI uses a single `AppState.overlay: Option<Overlay>` field — at
most one overlay active at a time. Each overlay variant carries its own
local state (e.g. `Sessions { map, cursor }`, `Orders { timeline, cursor }`,
`Marks { cursor }`, `Help { scroll }`). The render layer draws overlays
last so they stack over the main list/detail layout; each view paints a
centered `ratatui::widgets::Clear` over its rect to avoid bleed-through.

`App::overlay_intercept` runs before the main action dispatch when an
overlay is open: it routes `j`/`k`/`Enter`/`Esc` to overlay-local
handlers and lets everything else fall through so global keys (`q`,
`:`, `F`) still work. Adding a new overlay is: (1) add a variant on
`Overlay`, (2) create `view/<name>.rs`, (3) render it in `draw()`,
(4) wire it into `overlay_intercept` for navigation/commit.

### Navigable-overlay pattern (Sessions / Orders / Marks)

All three follow the same shape. Use it for any new overlay that
presents a selectable list:

1. **State**: add `cursor: usize` to the overlay variant. Initialise to
   `0` where the overlay is opened (see `command::open_orders_overlay`,
   the `:marks` handler, etc.).
2. **Navigation**: extend the `CursorDown | CursorUp`,
   `CursorHalfPageDown | CursorHalfPageUp`, and `CursorTop | CursorBottom`
   arms of `overlay_intercept`. Clamp to `[0, len - 1]`; on empty lists,
   return `true` without touching anything (no crash when the overlay
   renders a placeholder).
3. **Commit**: `Action::OverlayApply` (bound to `Enter` in Normal mode)
   dispatches to a dedicated method — `apply_session_filter` /
   `jump_to_orders_selection` / `jump_to_marks_selection`. Jump helpers
   share a shape:
   ```rust
   let Some(Overlay::Marks { cursor }) = self.state.overlay else { return };
   /* resolve the selected row → target ordinal */
   if let Some(idx) = self.state.visible.iter().position(|&o| o == target) {
       self.state.cursor = idx;
       self.state.mode = ViewMode::Browse;
       self.state.overlay = None;
       self.state.status = StatusMessage::info(...);
   } else {
       self.state.status = StatusMessage::warn("... hidden by current filter — clear filter to see it");
   }
   ```
   If the ordinal is filtered out of `visible`, **warn without closing**
   so the user can decide whether to clear the filter; do not silently
   rewrite the filter.
4. **Render**: use `ratatui::widgets::TableState::default().select(cursor)`
   + `render_stateful_widget` (Orders) or a `Paragraph` with manual
   `patch_style(REVERSED)` on the selected line (Marks). The Marks
   overlay shares the exact list schema by pulling `header_line()`,
   `build_line_for_ord(state, ord)`, and the `COL_*` / `pad_cell`
   helpers from `view::list` as `pub(crate)` exports — prefer that over
   duplicating column math.

## Multi-key sequences via `pending_prefix`

`AppState.pending_prefix: Option<char>` buffers the first key of a
two-character sequence. Currently used by:

- `yy` / `yY` — yank raw / yank pretty.
- `dd` — set diff slot A. Diff slot B opens via `dD` (4-key) **or** a
  bare `D` after `dd` has already set slot A (3-key). See `DiffSlotB`
  handler — it checks `diff_slots[0].is_some()` so a stray `D`
  completes the diff instead of warning when the user clearly meant it.
- `m<digit>` — set bookmark (0..9 only).
- `'<digit>` — jump to bookmark.

The dispatch shape in `App::apply`:

```rust
if let Some(prefix) = self.state.pending_prefix.take() {
    match (prefix, &action) {
        ('y', Action::YankPrefix) => { self.yank_raw(); return; }
        ('d', Action::DiffPrefix) => { self.set_diff_slot(0); return; }
        ('m', Action::Letter(c))  => { self.set_bookmark(*c); return; }
        _ => { /* fall through: re-process as a fresh key */ }
    }
}
```

Invariant: the prefix is consumed or dropped on **every** action, never
left over. If the completion is invalid, the fresh action is handled
normally as if no prefix had been pending. This avoids mode-like state
that could strand the user without a visual cue.

### Digit-priority routing (marks)

Marks accept digits only (`m0`..`m9`, `'0`..`'9`) so an accidental
letter keystroke never silently triggers a dedicated shortcut under a
mark prefix. But `0` has a dedicated binding (`ScrollHome`), so the
naïve flow would turn `m0` into `ScrollHome`.

Fix: when `pending_prefix == Some('m' | '\'')` in Normal mode,
`App::on_event` routes events through `input::map_event_digit_priority`
instead of `map_event`. The variant promotes any ASCII digit to
`Action::Letter(c)` *before* falling back to `map_normal_key`; letters
still go through `map_normal_key` unchanged, so a stray letter drops
the prefix and runs its own shortcut instead of being silently
consumed.

```rust
let action = match (self.state.pending_prefix, self.state.input_mode) {
    (Some('m'), InputMode::Normal) | (Some('\''), InputMode::Normal) => {
        map_event_digit_priority(ev)
    }
    _ => map_event(ev, self.state.input_mode),
};
```

`set_bookmark` / `jump_bookmark` still validate `is_ascii_digit()` —
defense in depth for any future caller that invokes them directly.

## Analysis builders over `LogIndex`

Any new analyzer in `fixlog-analysis` should follow the same shape as
`SessionMap::build` / `OrderTimeline::build` / `Histogram::build`:

```rust
pub fn build(index: &LogIndex, buf: &[u8], format: &LogFormat) -> Self {
    for ord in 0..index.len() {
        let Some(bytes) = index.message_bytes(buf, ord) else { continue };
        let Ok((msg, _)) = parse_one_with_format(bytes, format) else { continue };
        // … use util::find_tag(&msg, T) / util::parse_sending_time(...) …
    }
    …
}
```

- Re-parse lazily from mmap; never store `&[u8]` references across the
  mmap boundary. Materialize via `Vec<u8>` / `SmallVec<[u8; N]>`.
- Skip messages that fail `parse_one_with_format` silently (warn + skip
  is the library-wide contract).
- If the analyzer needs hot-tag lookup, check
  `index.secondary.has_tag(tag)` before trusting `lookup`.

### Narrow tag extraction for single-tag hot paths

If the analyzer only needs **one** tag per message (e.g. `Histogram`
only reads tag 52 / SendingTime), skip the full tokenizer and use
`util::extract_tag_raw(bytes, format, tag) -> Option<&[u8]>` instead.
It scans `tag=value<sep>` pairs left-to-right and short-circuits on
the first match — O(tag_position) instead of O(n_tags × parse).

Coupled with `rayon::into_par_iter` over `0..index.len()`, this gave
`Histogram::build` a 94% runtime cut on 1M messages (573 ms → 32 ms).
The pattern:

```rust
let results: Vec<Option<u128>> = (0..index.len())
    .into_par_iter()
    .map(|ord| {
        let bytes = index.message_bytes(buf, ord)?;
        let raw = extract_tag_raw(bytes, format, TAG_SENDING_TIME)?;
        parse_sending_time(raw).and_then(system_time_to_nanos)
    })
    .collect();
// single sequential fold collects min/max/drops alongside the vec.
```

Only worth it when: (1) exactly one tag needed, (2) many messages, and
(3) you can express the post-processing as a per-ordinal map. For
multi-tag needs, keep `parse_one_with_format` — the extractor would
scan the body once per tag requested and compound the cost.

## Hot-tag pushdown

`fixlog-query::Expr::hot_equalities(&self) -> Option<Vec<(u32, &[u8])>>`
returns `Some(...)` only for a pure AND-of-`Eq` expression. `None` for
anything with `Or`, `Not`, `Ne`, or `Re`.

`fixlog-tui::state::evaluate_visible` uses it to short-circuit the full
scan: it intersects the sorted `SecondaryIndex::lookup(tag, value)`
slices (which are sorted ascending by construction — see
`crates/index.md`). This turns `35=D` on 1M messages from ~477 ms into
~156 µs. Partial pushdowns (some-tags-hot-some-not) are intentionally
skipped — complexity for little gain; see `docs/PHASE4_PLAN.md` §P4-T14.

## Do not

- Do not add `serde` / `serde_json` unless a consumer genuinely needs structured data beyond JSONL output. The manual writers are fine.
- Do not add `tokio` anywhere. fixlog is synchronous end-to-end, including the TUI — its event loop is a blocking `crossterm::event::poll(250ms)` that also serves as the file-watcher tick. If latency pressure ever forces async, it should be an exception, not the default.
- Do not add `log` + `env_logger`; the project uses `tracing` + `tracing-subscriber` exclusively.
- Do not re-introduce prefix-length handling in the parser — the tokenizer scans for `8=FIX` via memmem, so variable-length prefixes work for free (see `crates/parser.md` INVARIANT).
- Do not cache borrowed references across `--follow` ticks — the `Arc<Mmap>` gets swapped. Store ordinals or materialise with an `Owned` twin (see `ResolvedMessageOwned` above).
- Do not branch on `KeyCode` inside `app.rs`; add a new `Action` variant in `input.rs` and a match arm in `App::apply`. Keeps the keybinding table testable in isolation.
- Do not use `parse_one(bytes)` on slices that come from a sniffed log file — it hardcodes SOH and silently fails with `UnexpectedEof` on pipe-, caret-, or semicolon-separated logs (QuickFIX-J renders `|`). Use `parse_one_with_format(bytes, &state.format)` (or `&format` where available). The example in `## Analysis builders over LogIndex` above predates this rule; `fixlog-analysis` still calls `parse_one` and will misbehave on non-SOH fixtures — pending migration (see `project_fixlog_status.md`).
