# `fixlog-tui`

Interactive ratatui + crossterm frontend. Lives at `crates/fixlog-tui/`.
Library-only; the `fixlog tui` CLI subcommand is a thin wrapper over
[`fixlog_tui::run`] that lives in `fixlog-cli`.

## Files

- `src/lib.rs` — `TuiConfig`, `run`, top-level `draw` layout.
- `src/app.rs` — `App` (wraps `AppState`), action dispatch, yank/search handlers.
- `src/state.rs` — `AppState`, `ViewMode`, `InputMode`, `StatusMessage`, detail cache types, `bootstrap`, `apply_filter`, `snapshot_filter`, `restore_filter`.
- `src/input.rs` — `Action` enum + `map_key` / `map_command_key` / `map_search_key`.
- `src/event.rs` — `Event { Key, Resize, Tick }` over `crossterm::event::poll` (250 ms).
- `src/terminal.rs` — `TerminalGuard` (RAII alt-screen + panic hook).
- `src/command.rs` — `:q`, `:filter <expr>`, `:help` parser + executor, plus `live_preview` invoked per keystroke.
- `src/search.rs` — `next_match` (forward/backward, wrapping) over `visible`.
- `src/follow.rs` — `FollowWatcher`: stat-based polling, re-mmap + `append_from_offset`, rebuild on truncation.
- `src/clipboard.rs` — arboard wrapper for `yy`/`yY`, with `raw_to_text` / `pretty_text`.
- `src/theme.rs` — `color_for_msg_type` default map.
- `src/io.rs` — `mmap_file` / `head` (duplicated from `fixlog-cli/src/io.rs`).
- `src/view/{list,detail,status,command,search}.rs` — main per-region renderers.
- `src/view/{sessions,orders,diff,marks,histogram}.rs` — overlay renderers (Fase 4). Each paints a centered `Clear` + bordered widget over the main layout.
- `src/export.rs` — `:export <fmt> <path>` writers (csv/json/fix/pretty).
- `tests/{bootstrap,navigation,command,search,yank}.rs` — integration tests driving `App` with synthesised key events.
- `benches/frame.rs` — frame budget + bootstrap + filter apply on 1M messages.

## Fase 4 additions — overlays, bookmarks, diff, export

`AppState.overlay: Option<Overlay>` (`Sessions`, `Orders`, `Diff`, `Marks`,
`Histogram`) — at most one at a time. When an overlay is open, `j`/`k`/
`Enter`/`Esc` are intercepted by `App::overlay_intercept` and routed to
overlay-local cursors; other keys (`q`, `:`, `F`, etc.) still work.

Two-key sequences extend the `yy`/`yY` pattern:

| Sequence      | Effect                                                             |
|---------------|--------------------------------------------------------------------|
| `m<letter>`   | `bookmarks[letter] = visible[cursor]`                              |
| `'<letter>`   | Jump cursor to `bookmarks[letter]` if present in current `visible` |
| `dd`          | Set `diff_slots[0]` to `visible[cursor]`                           |
| `dD`          | Set `diff_slots[1]`; opens diff overlay when both are full        |

`pending_prefix: Option<char>` buffers the first key; any subsequent
action is either a valid completion (handled inline and cleared) or
triggers a fall-through (prefix cleared, action re-dispatched as fresh).

Commands added in Fase 4:

| Command             | Effect                                                     |
|---------------------|------------------------------------------------------------|
| `:sessions`         | Build `SessionMap` on-demand, open overlay                 |
| `:orders [id]`      | Build `OrderTimeline` for `id` or cursor's tag 11, overlay |
| `:marks`            | Open bookmarks table                                       |
| `:histogram [dur]`  | Build histogram at `dur` bucket (default `1s`), overlay    |
| `:export <fmt> <path>` | Dump `state.visible` to disk (`csv`/`json`/`fix`/`pretty`) |
| `:diff clear`       | Clear both diff slots + close overlay                      |

## Public API

```rust
pub struct TuiConfig { pub path: PathBuf, pub follow: bool, pub initial_filter: Option<String> }
pub fn run(cfg: TuiConfig) -> anyhow::Result<()>;
```

Re-exports from `fixlog-core` cover everything else the TUI consumes
(parser, index, query, dict).

## Layout

```text
+-------------------------------------------+  title         (1 row)
|                                           |
| list (60%)         │  detail (40%)        |  body          (flex)
|                    │                      |
+-------------------------------------------+  status        (1 row)
|  /35=D▏                                   |  search/cmd    (1 row when active)
+-------------------------------------------+
```

The list view shows five semantic columns: `TIME | MESSAGE | CLIENT ORDER ID
| STATUS | DETAIL`. `TIME` is derived from tag 52 (`SendingTime`) as
`HH:MM:SS`; `MESSAGE` uses `fixlog-dict::chain_msg_type_label` with the
chain picked from the message's `BeginString`; `CLIENT ORDER ID` is tag 11;
`STATUS` is populated for `35=8` (ExecutionReport) as `ExecType · OrdStatus`
with enum labels resolved via `chain_enum_value_label` (tags 150 and 39);
`DETAIL` shows `Side OrderQty Symbol[ @ Price]` when tags 54/38/55/44 are
present — `Side` resolves through the dictionary (`BUY`, `SELL`, …), `OrderQty`
gets thousands separators.

`draw()` in `lib.rs` reserves the bottom row conditionally — 0 rows in
Normal mode, 1 row in Command or Search. The row is shared; only one of
`view::command::render` / `view::search::render` actually paints based on
`AppState.input_mode`.

## INVARIANTs

- **Zero-copy in hot render paths.** The list view lazily re-parses each
  visible row from the mmap; we never materialise `RawMessage::raw` into
  `Vec<u8>`. The only materialisation is `ResolvedMessageOwned` for the
  detail panel, which is cached per-ordinal and refreshed on cursor move.
- **`ResolvedMessageOwned` is owned by design.** The mmap is an
  `Arc<Mmap>` that gets swapped under `--follow`. If the cache borrowed
  from it, the swap would leave dangling references. Values are `Vec<u8>`,
  static `&'static str`s are fine.
- **`AppState.filter` and `AppState.filter_text` stay in sync.** Every
  mutation path goes through `apply_filter(state, Some(expr), Some(text))`
  (or both `None`). The source text is needed because `QueryExpr` is not
  `Clone`, so `snapshot_filter` / `restore_filter` rely on re-parsing.
- **`last_list_height` is set by the list view before `ensure_cursor_visible`.**
  `Ctrl+D`/`Ctrl+U` (half page) reads it to pick the step size; if a key
  arrives before the first render it falls back to a 1-row step.
- **`pending_prefix` clears on any action.** Multi-key sequences are only
  `yy` and `yY`; any non-continuation action takes the prefix slot and
  also runs normally (e.g. `y` then `j` still moves the cursor down).
- **Neither `h_offset` clamps upward.** Over-scrolling just shows an
  empty viewport — we don't track per-row max widths (would require a
  full re-parse pass every scroll) and that's intentional. `Left`/`0`
  are the recovery path.
- **`raw_detail_mode` drops the list view's render pass.** `lib::draw`
  checks the flag once per frame and either gives the detail panel the
  full body or splits 60/40. List state (cursor, viewport_top, visible)
  continues to update but isn't painted. This is what keeps terminal
  drag-select clean inside raw mode.
- **Raw mode renders with no left border.** `view::detail::render` skips
  `Borders::LEFT` when `raw_detail_mode` is on so `│` chars don't appear
  in pasted selections. Resolved mode keeps the border as a panel
  separator.
- **Letters bound to actions are unavailable as bookmark/jump completions.**
  `r` binds to `ToggleRawDetail`, so `m` + `r` fails the prefix match and
  falls through: the prefix is cleared and the action runs normally (raw
  mode toggles). This is the same fall-through used for any bound letter;
  currently only `r` removes a bookmark letter. The other 25 still work.

## Display toggles (Fase A)

Three Normal-mode toggles shape what the panels render without touching the
user's typed filter:

| Key | State flag                | Effect                                                                             |
|-----|---------------------------|------------------------------------------------------------------------------------|
| `c` | `AppState.skip_common`    | Detail view hides tags 8, 9, 10, 34, 35, 49, 52, 56.                               |
| `H` | `AppState.hide_heartbeat` | Effective filter gains `AND NOT 35=0` (or `NOT 35=0` if the user filter is empty). |
| `r` | `AppState.raw_detail_mode`| Detail panel renders the raw FIX bytes (SOH → `\|`, non-printable → `.`) instead of the resolved tag table. Title shows `· raw`. |

`hide_heartbeat` is kept orthogonal to what the user typed: `AppState`
holds `user_filter_text` (what the user entered) and `filter_text` (the
composed text actually evaluated). `recompute_effective_filter` rebuilds
`filter_text` from `user_filter_text` + `hide_heartbeat` and applies it via
`apply_filter`. Snapshot/restore preserve both inputs so `Esc`-cancelled
live-preview edits leave the toggles intact.

The status bar shows `[no hb]` (yellow) when `hide_heartbeat` is on and
a `[list]` (magenta) / `[detail]` (cyan) indicator for the current
effective focus (always `[detail]` while raw mode is on). The
detail-panel title shows `· common skipped` (dim) when `skip_common` is
on and `· raw` (dim) when `raw_detail_mode` is on (both suffixes can
appear together).

## Focus + scroll model

Navigation state in `AppState`:

- `focus: Focus` — `{ List, Detail }`. Toggled by `Tab` / `Shift+Tab`.
- `list_h_offset: u16` — horizontal scroll of the list panel.
- `detail_h_offset: u16` — horizontal scroll of the detail panel
  (resolved mode only; raw mode wraps).
- `detail_v_offset: u16` — vertical scroll of the detail panel. Reset
  to 0 whenever `refresh_detail_cache` detects a new ordinal (the user
  jumped to a different message).
- `last_detail_height: usize` — most-recent detail viewport row count;
  used for `Ctrl+D/U` half-page step sizing and for clamping
  `detail_v_offset` when `G` blasts it to `u16::MAX`.

The `app::effective_focus` helper returns `Focus::Detail` whenever
`raw_detail_mode` is on, regardless of `state.focus`, because the list
isn't rendered in raw mode and routing `j`/`k` to a non-visible panel
would be actively confusing.

Dispatch (`app::nav` wraps this):

| Focus     | `j`/`k`/`g`/`G`/`Ctrl+D`/`Ctrl+U` | `Left`/`Right`       |
|-----------|-----------------------------------|----------------------|
| `List`    | `move_cursor` (message cursor)    | `list_h_offset`      |
| `Detail`  | `detail_scroll` (`detail_v_offset`) | `detail_h_offset`  |

All offsets saturate at 0 on underflow; none is clamped above at
dispatch time. Resolved-mode detail renderer (`view::detail::render_fields`)
clamps `detail_v_offset` each frame based on `fields.len()` so `G` lands
on the real last row. Raw-mode over-scroll leaves a blank viewport —
`g` or `k` recovers.

`0` (zero) resets all three offsets but leaves `focus` intact — the user
rarely wants focus flipped by an accident.

Rendering strategies (one per surface):

- **List view** (`view::list`): the list is a `Paragraph<Vec<Line>>` (not
  a `Table`) precisely so `Paragraph::scroll((0, list_h_offset))` can
  slide the whole row left. Each row is pre-laid-out with spans
  padded/truncated to fixed widths (`COL_TIME`=9, `COL_MESSAGE`=22,
  `COL_CLORDID`=18, `COL_STATUS`=24, single-space separators); the
  `DETAIL` column holds the remainder. Cursor highlight is applied via
  `Line::patch_style(REVERSED)`; MsgType coloring via `Line::style`. The
  header is a separate `Paragraph` in a 1-row sub-`Rect`, scrolled with
  the same offset so columns stay aligned with their titles.
- **Detail view — resolved mode** (`view::detail::render_fields`): `Table`
  layout is preserved (column widths 6/18/9/20/rest). Rows are filtered
  by `skip_common` and then sliced by `detail_v_offset..+viewport_rows`
  to give vertical scroll. The `raw` and `decoded` cells are also
  char-sliced by `detail_h_offset`; fixed short columns (`tag`/`name`/
  `type`) stay in place. Returns a `RenderFieldsOutcome` so the caller
  can write back the clamped `detail_v_offset` and the measured
  `last_detail_height` without fighting the borrow of `detail_cache`.
- **Detail view — raw mode** (`view::detail::render_raw`):
  `Paragraph::new(raw_value(bytes)).wrap(Wrap { trim: false }).scroll((detail_v_offset, 0))`.
  Wrap handles horizontal display; vertical scroll via `scroll((v, 0))`
  drops the first v wrapped lines. Bytes come from
  `state.index.message_bytes(&state.mmap, ord)` with `ord` read out of
  `state.detail_cache`.

## Raw detail mode — full-body layout

When `raw_detail_mode` is on, `lib::draw` collapses the body to a single
full-width detail panel (the list isn't rendered for that frame). Two
reasons:

1. **Terminal-native selection cleanliness.** Selection is char-based by
   visual row; without hiding the list, a drag across the wrapped raw
   bytes would also capture list content at the same y-coordinate. With
   the list gone the user can select just the raw message.
2. **Readability.** A typical FIX message is 80–250 bytes — a full-width
   wrap uses two or three rows instead of four or five in the 40%-wide
   detail panel.

The detail panel also **drops its left border** in raw mode (the `│`
character would otherwise be included in every wrap-boundary of a pasted
selection). This is the one place the border convention is broken on
purpose.

Terminal selection of wrapped raw text still produces `\n` at each wrap
boundary — terminals don't know about logical lines. For a clean
single-line copy the user should press `yy`, which `arboard` delivers
without wrap breaks (the status-bar hint on toggle reminds them).

## Input modes

| Mode     | Entered by        | Exited by            | Buffer             |
|----------|-------------------|----------------------|--------------------|
| Normal   | (default)         | —                    | —                  |
| Command  | `:`               | `Enter` / `Esc`      | `command_buffer`   |
| Search   | `/`               | `Enter` / `Esc`      | `search_buffer`    |

`Esc` in Command rolls back any live-preview filter via
`FilterSnapshot`; `Esc` in Search just closes the bar without moving the
cursor.

## Keybindings (Normal mode)

| Key            | Action                                                   |
|----------------|----------------------------------------------------------|
| `q` / `Ctrl+C` | Quit                                                     |
| `:`            | Open command bar                                         |
| `/`            | Open search bar                                          |
| `n` / `N`      | Next / previous match of the last search                 |
| `j` / `↓`      | Cursor down                                              |
| `k` / `↑`      | Cursor up                                                |
| `g`            | Top (enters Browse)                                      |
| `G`            | Bottom (enters Follow)                                   |
| `Ctrl+D` / `PageDown` | Half page down                                    |
| `Ctrl+U` / `PageUp`   | Half page up                                      |
| `F`            | Toggle Follow/Browse                                     |
| `c`            | Toggle "skip common fields" in detail (hide 8/9/10/34/35/49/52/56) |
| `H`            | Toggle "hide heartbeats" (composes `NOT 35=0` with the user filter) |
| `yy`           | Yank the raw bytes of the current message                |
| `yY`           | Yank the pretty-printed (resolved) representation        |
| `r`            | Toggle raw-FIX detail mode (wraps, full-width layout)    |
| `Tab`          | Move focus to the next panel (List ↔ Detail)             |
| `Shift+Tab`    | Move focus to the previous panel (same toggle today)     |
| `→` / `Right`  | Scroll the **focused** panel right by 8 columns          |
| `←` / `Left`   | Scroll the **focused** panel left by 8 columns           |
| `0`            | Reset all three scroll offsets (list h, detail h, detail v) |

**Focus-sensitive keys.** `j`/`k`/`g`/`G`/`Ctrl+D`/`Ctrl+U` and the
arrows all respect `AppState.focus`:

- `Focus::List` (default): `j`/`k` move the message cursor (historical
  behavior, also flips `ViewMode`); arrows scroll `list_h_offset`.
- `Focus::Detail`: `j`/`k` scroll `detail_v_offset`; `g`→0, `G`→`u16::MAX`
  (clamped by the resolved-mode renderer to the real last row); arrows
  scroll `detail_h_offset`.

In raw detail mode the list isn't rendered, so the nav dispatch forces
the effective focus to `Detail` regardless of `state.focus`.

Any cursor motion other than `G` drops the view out of `Follow` into
`Browse`. `G` snaps to `Follow` and resets `new_since_browse`.

## View modes

- **Follow**: cursor stays glued to the last row; `on_index_grew` pulls
  it along. Title-bar indicator: `[follow]` (green).
- **Browse**: cursor free; incoming messages increment `new_since_browse`
  (`⬇ N new` in the status bar). `F` returns to Follow and clears the
  counter.

## Command grammar

```
:q                      → quit
:quit                   → quit
:help / :h              → show keybinding cheatsheet in status
:filter <expr>          → apply filter (see fixlog-query grammar)
:f <expr>               → short form
:filter                 → clear filter
```

Any other head keyword sets `status.level = Error` with `unknown command`.

## Live filter preview

Each keystroke in command mode calls `command::live_preview`, which
recompiles the expression from `command_buffer` if it starts with
`filter ` / `f `. Parse failures are silent — the last valid preview is
preserved. `Esc` rolls back via `FilterSnapshot` (captured on
`EnterCommand`, dropped on `CommandSubmit`).

## Follow watcher

`FollowWatcher::poll` compares `fs::metadata(path).len()` against
`state.mmap.len()` at most once per 250 ms. Three outcomes:

1. Unchanged → no-op.
2. Grew → new `Arc<Mmap>`; `LogIndex::append_from_offset` with the old
   `consumed`; `extend_visible` appends new ordinals that pass the filter;
   `app::on_index_grew(delta)` advances the cursor (Follow) or increments
   `new_since_browse` (Browse).
3. Shrunk / out-of-sync → full rebuild via `state::bootstrap`, keeping
   `filter_text`. Status bar shows `file rotated or truncated — rebuilt`.

No `notify` dependency; the event-loop timeout doubles as the file poll
cadence. `grep --follow` in `fixlog-cli` does use `notify` for lower
latency because its output is piped to tools that may be polling tightly.

## Clipboard (`yy` / `yY`)

`yy` copies `index.message_bytes(mmap, ord)` through `clipboard::raw_to_text`
(SOH → `|`, non-printables → `.`). `yY` calls `refresh_detail_cache` and
feeds the resolved message through `clipboard::pretty_text`. Failures —
common in SSH-without-forwarding / headless CI — surface as
`StatusMessage::error("clipboard unavailable: …")`; the TUI never panics.

## Theme (P3-T11 defaults)

| MsgType | Label                   | Color      |
|---------|-------------------------|------------|
| `D`     | NewOrderSingle          | Green      |
| `8`     | ExecutionReport         | Blue       |
| `3` / `j` | Reject / BusinessReject | Red     |
| `0`     | Heartbeat               | Dark gray  |
| other   | —                       | default    |

Persistent theme config (`~/.config/fixlog/config.toml`) is Phase 5.

## Performance (2026-04-17, `cargo bench -p fixlog-tui --bench frame --quick`)

Darwin 25.3.0. Amplified fixture = `minimal_4.4.log` × 100 000 ≈ 1M messages.

| Bench                           | Time          |
|---------------------------------|---------------|
| `tui_bootstrap/1M_messages`     | ~123 ms       |
| `tui_frame/list_detail_status_200x50` | **~737 µs** |
| `tui_filter/apply_35eqD_1M`     | ~477 ms       |

Frame budget target was <16 ms; achieved ~0.74 ms — ~22× under budget.
Bootstrap benefits from `build_from_bytes_parallel`. The filter pass is a
full scan; the P3-T09 "hot-tag pre-filter" optimisation remains deferred
(see `state.md` gaps).

## When to modify this crate

- **New keybinding**: add an `Action` variant, bind it in `input.rs`,
  dispatch in `app.rs`. Update this doc's table.
- **New command**: extend `command::parse` and `command::execute`. Live
  preview lives in `command::live_preview`; extend there too if the new
  command should preview.
- **New view**: `src/view/<name>.rs` + `view/mod.rs` entry + slot in
  `lib.rs` `draw`. Always take `&AppState` unless the view must mutate
  view-state bookkeeping (e.g. `viewport_top` — see `list.rs`).
- **Follow strategy**: `FollowWatcher::poll` is the hot path. If CI needs
  sub-100ms latency, wire in `notify` following the pattern in
  `fixlog-cli/src/commands/grep.rs`.
- **Theme overrides**: `theme.rs`; keep `color_for_msg_type` pure so it
  stays table-driven for Phase 5.

## Do not

- Do not call `terminal.draw` outside `run` — the `TerminalGuard` must
  be alive; panics without it leave the terminal broken.
- Do not hold `&mmap` references across follow-ticks — store ordinals or
  owned copies. The mmap is swapped by `FollowWatcher` on growth.
- Do not add blocking I/O to the event loop; it's single-threaded and
  will stall rendering.
- Do not introduce `tokio` or other async runtime. Synchronous event
  loop is the design.
- Do not remove `#![deny(unsafe_code)]` at the lib root. The single
  `#[allow(unsafe_code)]` in `io.rs` is the only permitted escape hatch,
  and it wraps `memmap2::Mmap::map`.
