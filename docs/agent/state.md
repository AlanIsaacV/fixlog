# Project state

> Snapshot of what exists and what's next. Authoritative over `PHASE*_PLAN.md` files when they disagree.

## Phase

**Fase 4 — Análisis avanzado** — **effectively complete** (P4-T01 through P4-T14, T16, T17 landed; T15 symbolic names deferred to Fase 5 by default). New crate `fixlog-analysis` implements session tracking, order lifecycle, temporal histogram. CLI gains `sessions` / `orders` / `histogram` subcommands. TUI gains sessions / orders / diff / marks / histogram overlays, bookmark set/jump (`m<letter>` / `'<letter>`), diff slots (`dd` / `dD`), and `:export` (csv/json/fix/pretty). Hot-tag pre-filter drops `tui_filter/apply_35eqD_1M` from ~477 ms to ~156 µs (~3000× speedup).

**Fase A — TUI rediseño (inspirado en fixparser.targetcompid.com, 2026-04-18)** — **done, committed**. List columns `TIME | MESSAGE | CLIENT ORDER ID | STATUS | DETAIL`; `c` toggle for skipping common header/trailer tags in the detail panel; `H` toggle composing `AND NOT 35=0` through `recompute_effective_filter` + extended `FilterSnapshot`. TUI re-parse sites migrated to `parse_one_with_format` so pipe-separated logs resolve.

**Fase B — TUI rediseño, riqueza semántica (2026-04-18)** — **done, committed**. New `summary` module with a per-MsgType table (D/8/F/G/3/j/A/5/0/1/2) producing `MessageSummary { badges, client_order_id, detail }`; unknown MsgTypes fall through to the generic Side/Qty/Symbol fallback. `list::build_line` consumes `summary::summarize` directly; duplicated helpers dropped. Panel focus extended: `AppState.detail_cursor` + `detail_fields_len` + `detail_cursor_field`; `j`/`k`/`g`/`G`/`Ctrl+D/U` in `Focus::Detail` move the per-field cursor (viewport auto-scrolls); the renderer highlights the cursor row. `Action::FilterFromDetail { negated }` bound to `f`/`x` composes `tag=value` / `NOT (tag=value)` into `user_filter_text` via `recompute_effective_filter`; bareword-vs-quoted value heuristic honours the DSL grammar.

**Fase 3 — TUI básico (ratatui)** — **effectively complete**. All P3-T01..T16 landed. TUI renders <1 ms per frame on 1M messages (22× under the 16 ms budget); parser and index benches stable within noise. Integration tests cover bootstrap, navigation, command bar, search, yank, and follow watcher end-to-end.

**Fase 2 — Indexación + Tailing + Query DSL** — effectively complete. P2-T01..T09 all landed; only P2-T10 (`fixlog index` subcommand with serialized cache) remains, and it's marked as optional/stretch in the plan — best deferred to Phase 5 alongside config persistence.

Fase 1 is closed (all T01-T16 done; T17 only informal sign-off).

## Crates in the workspace (Cargo.toml members)

- `crates/fixlog-parser` — zero-copy tokenizer.
- `crates/fixlog-format` — format sniffer.
- `crates/fixlog-dict` — multi-version dictionary + resolver.
- `crates/fixlog-index` — offset-based index with append-only growth, secondary hot-tag map, and rayon-parallel build. See `crates/index.md`.
- `crates/fixlog-query` — filter DSL (AST + parser + evaluator). See `crates/query.md`.
- `crates/fixlog-core` — facade that re-exports all of the above.
- `crates/fixlog-cli` — binary `fixlog` with `sniff` / `parse` / `stats` / `grep` (with `--follow`) / `tui` (with `--follow`). See `crates/cli.md`.
- `crates/fixlog-tui` — interactive ratatui + crossterm frontend: virtual list, resolved detail, status + command + search bars, vim navigation, follow/browse, live filter preview, yank to clipboard, MsgType colouring, stat-based follow watcher. See `crates/tui.md`.
- `crates/fixlog-analysis` — session tracking, order lifecycle, temporal histogram (Phase 4). Pure library; depends on `fixlog-core`. Consumed directly by `fixlog-cli` and `fixlog-tui`. Not re-exported from `fixlog-core` (by design — analysis composes on top of core primitives and stays distinct). See `crates/analysis.md`.

## Completed (vs PHASE1_PLAN.md)

| Task | Status | Notes |
|------|--------|-------|
| T01 Setup workspace | done | |
| T02 Fixtures corpus | done | `fixtures/real/` + `fixtures/synthetic/` |
| T03 Parser stub + types | done | |
| T04 SOH tokenizer + validator | done | Prefix-agnostic scan; checksum mismatch non-fatal. See `crates/parser.md`. |
| T05 Parser integration tests | done | `crates/fixlog-parser/tests/synthetic.rs` + `with_format.rs` |
| T06 Sniffer implementation | done | Separator + line-prefix detection. See `crates/format.md`. |
| T07 Parser ↔ sniffer integration | done | `parse_all_with_format` ignores `line_prefix` — parser scans for `8=FIX` itself. |
| T08 Dict crate + build.rs | done | 3 versions generated. See `crates/dict.md`. |
| T09 Resolver | done | Chain-based, auto-selects from `BeginString` + `ApplVerID`. |
| T10 CLI stub | done | `clap` derive, `-v`/`-vv` tracing. |
| T11 `sniff` command | done | |
| T12 `parse` command | done | `--first N`, `--format pretty\|json`; JSON validates with `jq`. |
| T13 `stats` command | done | Total/errors, time range, sessions, top-10 MsgTypes. |
| T14 Multi-version dicts | done (scope reduced) | FIX44 + FIXT11 + FIX50SP2. FIX50/FIX50SP1 skipped (rarely seen in practice; easy to add later). |
| T15 `fixlog-core` facade | done | CLI imports only from `fixlog-core`. |
| T16 Criterion benchmarks | done | `crates/fixlog-parser/benches/parse.rs` — synthetic (soh/pipe/prefix), real (fix44_om/fixt11_md), and a `parse_known_soh/8MiB` microbench. Baseline below. |
| T17 Manual E2E validation | **partial** | CLI has been smoke-tested against real fixtures; no formal sign-off. |

## Completed (vs PHASE2_PLAN.md)

| Task | Status | Notes |
|------|--------|-------|
| P2-T01 `fixlog-index` skeleton | done | `MessageOffset`, `LogIndex`, `IndexError`. |
| P2-T02 Single-thread builder | done | Reuses `parse_all_with_format`; tests against three synthetic shapes + two real fixtures. |
| P2-T03 Secondary index (hot tags) | done | `HashMap<(tag, SmallVec<[u8;16]>), Vec<u32>>`. Default set: `35,49,56,11,34,37`. Configurable via `HotTags`. See `crates/index.md`. |
| P2-T04 Append-only growth | done | `consumed` watermark + `append_from_offset`; partial writes re-scanned safely. |
| P2-T05 Parallel builder | done | Rayon chunks with boundary ownership; **2.4-5.1× speedup** (see bench below). Bit-identical to single-thread output. |
| P2-T06 Query AST + parser | done | EBNF with `=`/`!=`/`~`/`AND`/`OR`/`NOT`/parens, case-insensitive keywords. Positional errors. |
| P2-T07 Evaluator | done | Zero-alloc, short-circuiting, regexes pre-compiled. |
| P2-T08 `grep` subcommand | done | `fixlog grep <file> --filter "<expr>" [--format json\|pretty]`. grep(1) exit codes. |
| P2-T09 `--follow` tailing | done | `notify` + re-mmap + rotation/truncation detection. 500 ms poll fallback. |
| P2-T10 `index` subcommand | **deferred** | Optional / stretch; move to Phase 5 alongside config + cache persistence. |

## Completed (vs PHASE3_PLAN.md)

| Task | Status | Notes |
|------|--------|-------|
| P3-T01 `fixlog-tui` stub | done | Crate at `crates/fixlog-tui/` with `TuiConfig`, `TuiError`, `run()` returning `bail!("stub")`. Deps: `ratatui 0.29`, `crossterm 0.28`, `arboard 3`, `fixlog-core`, `anyhow`, `thiserror`, `tracing`. Workspace member added. |
| P3-T02 Event loop + terminal setup | done | `TerminalGuard` (RAII drop restores terminal) + one-shot panic hook in `terminal.rs`. `event::next()` maps `crossterm` to `Event::{Key,Resize,Tick}` with 250 ms poll. `App::on_event` handles `q`/`Ctrl+C`. `run()` renders a placeholder frame (border + path title). 6 new tests (4 on `App`, 2 on `draw` via `TestBackend`). |
| P3-T03 `AppState` + bootstrap | done | `state.rs` with `AppState { path, mmap: Arc<Mmap>, format, index, filter, visible, cursor, viewport_top, mode, new_since_browse, status }`. `bootstrap(path, initial_filter)` mmaps + sniffs + `build_from_bytes_parallel` + compiles filter + populates `visible`. `ViewMode::{Follow,Browse}`, `StatusMessage` with TTL, `clamp_cursor` + `ensure_cursor_visible` helpers. Integration tests against real fixtures: 5419/8229 counts, filter reduction, `consumed` invariant, error context. `#![deny(unsafe_code)]` with `#[allow]` on `io.rs` (memmap). |
| P3-T04 Virtual list view | done | `view/list.rs` renders a ratatui `Table` with columns `#ord \| offset \| type \| sender→target \| raw preview`. Lazy per-row `parse_one` from mmap, SOH→`\|` in preview, 200-byte truncation. Cursor row highlighted via `row_highlight_style(REVERSED)`. Refactor: `App` now wraps `AppState`; keybinding dispatch extracted to `input.rs` (pure `map_key(KeyEvent) -> Action`). Title bar shows `fixlog <path> — <cursor>/<shown> (<total>)`. Integration tests render fix44-om.log through a `TestBackend` and assert ordinal/MsgType/empty-placeholder/scroll-to-visible behaviours. |
| P3-T05 Detail panel | done | `view/detail.rs` renders resolved message as `tag \| name \| type \| raw \| decoded`. New `ResolvedMessageOwned` / `ResolvedFieldOwned` in `state.rs` materialise values to `Vec<u8>` so the cache outlives mmap swaps under `--follow`. Cache is `Option<(ord, Result<Owned, String>)>` keyed by ordinal, refreshed only on cursor move; parse errors render as `<parse error> …` in red instead of crashing. Layout split to 60/40 horizontal (list/detail). Field type column uses `dict::chain_field_by_tag`. 5 new tests (cache hit, cache miss on cursor move, empty filter, render header, ASCII raw rendering). |
| P3-T06 Status + command bar | done | New `InputMode::{Normal,Command}`. Normal mode: `q`/`Ctrl+C` quit, `:` enters Command. Command mode: chars append to buffer, Enter submits, Esc cancels, Backspace edits, Up/Down walk history. `command.rs` parses/executes `:q`, `:help`, `:filter <expr>`, `:filter` (clear), with status-bar feedback on invalid filter. `view/status.rs` renders `sep | filter-state | cursor/visible (total)` or transient colored message when `StatusMessage::is_active`. `view/command.rs` only renders in Command mode. Command history deduped at push. Integration suite in `tests/command.rs` drives `App` via synthesised key events through the full input → app → command chain. |
| P3-T07 Vim navigation | done | `j/k/↓/↑` step 1, `g` top, `G` bottom, `Ctrl+D/U` + `PageDown/Up` half page. Mode contract: any movement except `G` drops to `Browse`; `G` snaps to `Follow`. Half-page step uses `AppState.last_list_height` which `view/list.rs` writes on every render. Integration tests in `tests/navigation.rs` drive `App` with real fixture and assert clamping, mode flips, empty-visible no-op. |
| P3-T08 Follow/Browse toggle | done | `F` toggles `ViewMode` via `Action::ToggleMode`. Switching back to `Follow` resets `new_since_browse` and snaps cursor to end. Public helper `app::on_index_grew(state, delta)` for the future `--follow` hook: in Follow keeps cursor glued to the end; in Browse accumulates `new_since_browse` without disturbing cursor. Status-bar left segment now shows `[follow]`/`[browse]` (green/yellow) and `⬇ N new` in cyan when browsing with pending arrivals. |
| P3-T09 Live filter | done | **Decision**: live preview triggered inside the command bar (`:filter <expr>` / `:f <expr>`) instead of a dedicated `/`-activated mode — `/` is reserved for search in P3-T10 per the ROADMAP. Each keystroke calls `command::live_preview`, which recompiles `filter` and re-evaluates `visible` when the expression parses; malformed/partial expressions freeze the previous preview. `Esc` rolls back to the pre-command snapshot (`AppState.filter_snapshot`); `Enter` commits. `AppState` now carries `filter_text: Option<String>` so the source survives snapshot/restore (compiled `QueryExpr` isn't `Clone`). Status-bar middle segment shows the active filter text. New `apply_filter` / `snapshot_filter` / `restore_filter` helpers in `state.rs`; integration tests in `tests/command.rs` cover live update, Esc revert, and partial-expression freeze. |
| P3-T10 Search `/` + `n/N` | done | New `InputMode::Search`; `/` opens the bar with its own buffer. `search.rs` provides `next_match(state, expr, Direction)` with wrap-around over `visible`. Enter submits, jumping the cursor to the first forward match and remembering `search_last_text`. `n` / `N` iterate forward/backward using the remembered expression (re-parsed each time — `QueryExpr` isn't `Clone`). Wraps surface via `StatusMessage::info("search wrapped")`; no match → `warn("no match")`; `n` without prior search → `warn("no previous search")`. Any search hit drops the view into `Browse` so the user's position doesn't get yanked by incoming tail. |
| P3-T11 MsgType colouring | done | `theme::color_for_msg_type(&[u8]) -> Option<Color>` with the P3 defaults (`D` green, `8` blue, `3`/`j` red, `0` dark gray). Applied to the whole row style in `view/list.rs` so MsgType, sender→target, and raw preview all share the color. Unknown types render with terminal default. Persistent config deferred to Phase 5. |
| P3-T12 Yank to clipboard | done | `clipboard.rs` wraps arboard with `copy` + helpers `raw_to_text` (SOH → `\|`, non-printable → `.`) and `pretty_text` (resolved fields table). Two-key sequences `yy` / `yY` implemented via `AppState.pending_prefix`: first `y` sets `Some('y')`; completion triggers yank and clears; any other action consumes the prefix and is re-dispatched normally. Clipboard failures (headless CI, no DISPLAY) surface as `StatusMessage::error("clipboard unavailable: …")` without crashing. Integration tests assert state transitions but tolerate clipboard-unavailable status text. |
| P3-T13 `--follow` integration | done | `FollowWatcher` in `follow.rs` polls `fs::metadata(path).len()` every 250 ms from the event loop (reusing the `crossterm::event::poll` timeout as the cadence — no `notify` dependency, no extra thread). Growth → new `Arc<Mmap>`, `append_from_offset` with the old `consumed`, `extend_visible` appends ordinals that pass the active filter, `app::on_index_grew(delta)` drives cursor in Follow mode and `new_since_browse` in Browse. Truncation/rotation → full `bootstrap` preserving `filter_text`. Unit tests exercise growth, cursor behavior per mode, active-filter preservation, and truncation rebuild using tmp files. |
| P3-T14 `fixlog tui` subcommand | done | `fixlog-cli` gains `tui` subcommand (`--filter`, `-F`/`--follow`). `crates/fixlog-cli/src/commands/tui.rs` is a 10-line wrapper around `fixlog_tui::run`. New dep `fixlog-tui` in `fixlog-cli/Cargo.toml`. `fixlog tui --help` appears in `fixlog --help` output. |
| P3-T15 Frame budget bench | done | `crates/fixlog-tui/benches/frame.rs` with `tui_bootstrap/1M_messages`, `tui_frame/list_detail_status_200x50`, `tui_filter/apply_35eqD_1M`. Amplifies `minimal_4.4.log × 100k` to tempfile; uses `TestBackend`. Frame ~737 µs (target <16 ms — ~22× margin). See §"TUI performance" below. Parser and index baselines confirmed within ±5%. |
| P3-T16 Docs + state sync | done | New `docs/agent/crates/tui.md` (layout, invariants, keybindings, input modes, follow strategy, bench numbers). `docs/agent/INDEX.md` updated (routing entry + file list). `state.md` (this file) updated at every task close. |

## Completed (vs PHASE4_PLAN.md)

| Task | Status | Notes |
|------|--------|-------|
| P4-T01 Scaffold `fixlog-analysis` | done | New crate with `sessions`, `orders`, `histogram`, `util` modules, `AnalysisError` (thiserror). `#![forbid(unsafe_code)]`. Deps: `fixlog-core`, `smallvec`, `thiserror`, `tracing`. Not re-exported from `fixlog-core` (analysis is a composition on top, not a core primitive). |
| P4-T02 Session tracker | done | `SessionMap::build` + `append_from`. Canonical key `(sender, target)` = lex-sorted `(49, 56)` pair so both directions collapse into one entry. `in_count` / `out_count` split by role within the pair. Gap detection re-parses ordinals per session, sorts by `MsgSeqNum` per direction, emits `SeqGap` for each hole. Tests: 4 unit (synthetic 2-pair + injected gap) + 1 integration (fix44-om.log → 1 session, 0 gaps). |
| P4-T03 Order lifecycle | done | `OrderTimeline::build` via secondary lookup on tag 11 → collect tag 37s → secondary lookup on 37 → dedup/sort by ordinal → materialize events. Handles `F` (cancel) and `G` (replace) where ClOrdID changes but OrderID links the chain. `render_gantt(timeline, width)` helper with `N`/`X`/`C`/`R`/`!`/`?` char map. Tests: 5 unit (cancel/replace synthetic fixture, unknown id, Gantt width clamp) + 1 integration (fix44-om.log). Synthetic fixture generated in-test, not on disk. |
| P4-T04 Temporal histogram | done | `Histogram::build` single-pass over `index.messages` with clamped `bucket_ns`. `render_sparkline(width)` uses **percentile-based** scaling (p95 = full) so outlier bursts don't flatten the chart. `peaks(k)` top-k bins by count. Tests: 5 unit + 1 integration (fix44-om.log: binned + dropped == 5419). |
| P4-T05 `fixlog sessions` CLI | done | `--format pretty\|json`. Pretty: ASCII table `session \| msgs \| by-msg-type \| seq-range \| gaps`. JSON: JSONL per session. Exit 1 if no sessions. Validated with `jq empty`. |
| P4-T06 `fixlog orders` CLI | done | `--id <clordid>` prints timeline + Gantt; without `--id` lists top-N (default 20) by event count. `--format pretty\|json`. |
| P4-T07 `fixlog histogram` CLI | done | `--bucket <dur>` (1s default), `--width <cols>` (80 default), `--peaks <N>` (5 default). Manual duration parser (`1s`, `500ms`, `100us`, `2m`). |
| P4-T08 TUI sessions overlay | done | `:sessions` builds map on-demand, opens centered overlay. `j`/`k` move overlay cursor; `Enter` applies filter `49=X AND 56=Y` and closes; `Esc` closes without applying. Gap counts rendered red/bold. |
| P4-T09 TUI order lifecycle overlay | done | `O` keybind opens overlay for ordinal-under-cursor's tag 11 (or `:orders [id]`). Gantt bar + event table. Status-bar warn if the message has no tag 11. Per-event row coloured by `exec_type` (PendingNew gray, New/Replaced blue, Partial yellow, Fill green, Cancel/Reject red). |
| P4-T10 Diff view | done | `pending_prefix` machinery: `d` sets prefix; `d` after `d` sets slot A; `D` after `d` sets slot B and opens overlay when both slots are full (warn "both slots same" when A == B). Side-by-side table union-of-tags coloured: equal dim, only-A yellow, only-B cyan, distinct red. `:diff clear` resets. |
| P4-T11 Bookmarks | done | `m` + letter sets `bookmarks['x'] = visible[cursor]`. `'` + letter jumps if ordinal is in current filtered view; warns otherwise. `:marks` opens table overlay `mark \| ordinal \| preview`. Persistence deferred to Phase 5. |
| P4-T12 Export | done | `:export <fmt> <path>` with `fmt ∈ {csv, json, fix, pretty}`. Writes `state.visible` to disk synchronously; status bar reports count or error. **Decision deviation**: keep render helpers inline in `fixlog-tui/src/export.rs` rather than extracting a new `fixlog-render` crate — the duplication with `fixlog-cli/src/commands/parse.rs` is ~100 LOC and adding a crate for it triples the dependency graph. Can be promoted later if a third consumer appears. |
| P4-T13 TUI histogram overlay | done | `:histogram [bucket]` (default 1s) builds histogram, opens overlay with sparkline + top-20 peaks table. |
| P4-T14 Hot-tag pre-filter | done | `Expr::hot_equalities()` on `QueryExpr` returns `Some(Vec<(u32, &[u8])>)` iff the expression is a pure AND of `Eq` predicates. `SecondaryIndex::has_tag(tag)` convenience added. `state::evaluate_visible` intersects the sorted ordinal lists when all tags are hot; falls back to full scan otherwise. **Bench**: `tui_filter/apply_35eqD_1M` drops from ~477 ms to ~156 µs (3000× speedup, target was <100 ms). Correctness test compares fast-path ordinals against full-scan for `35=D`, `35=8`, `49=...`, `AND` combos. |
| P4-T15 Symbolic query names | **deferred** | To Fase 5 per default, unchanged. |
| P4-T16 Benches | done | `crates/fixlog-analysis/benches/analysis.rs` with `session_build_1M` (~1.16 s, target <1 s — slight miss on --quick single-sample run), `order_lookup_1M` (~5 µs, target <50 ms), `histogram_build_1M` (~500 ms, at target). Frame bench automatically reflects hot-tag pushdown (~156 µs). |
| P4-T17 Agent docs | done | New `docs/agent/crates/analysis.md` (module layout, invariants, algorithms, perf). INDEX.md routing table updated. `state.md` (this file) refreshed. `patterns.md` / `tui.md` / `cli.md` updates pending — see "Known gaps" below. |

## Test & quality gates (status)

All green as of last run:

- `cargo test --all` — all green post-Fase B (Fase A +17, Fase B / summary +9, Fase B / focus +8 over Fase 4 baseline of 220).
  - parser: 12 unit + 3 synthetic + 2 with_format
  - format: 7 unit
  - dict: 4 unit + 7 integration
  - index: 24 unit (builder + secondary + parallel) + 4 real_fixtures
  - query: 18 unit + 6 new AST hot_equalities tests
  - analysis: 19 unit (sessions + orders + histogram + util) + 3 real_fixtures
  - cli: 3 grep unit + 2 histogram (duration parser) + 5 integration (grep + grep --follow)
  - tui: 92 unit (includes Fase B summary unit tests) + 8 focus integration + display_toggles + horizontal_scroll + earlier bootstrap/command/navigation/search/yank + 1 hot-tag correctness
- `cargo clippy --all-targets --all-features -- -D warnings` — clean.
- `cargo fmt --all --check` — clean.

## Real-fixture parse metrics (last measured)

| Fixture | Size | Messages parsed | Parse errors |
|---------|------|-----------------|--------------|
| `fixtures/real/fix44-om.log` | 2.1 MB | 5419 | 0 |
| `fixtures/real/fixt11-md.log` | 8.7 MB | 8229 | 0 |

Checksum mismatches are **non-fatal** (see `crates/parser.md`); they are emitted as valid messages and logged at `debug` level.

## Parser baseline throughput (2026-04-17, criterion `--quick`)

Run: `cargo bench -p fixlog-parser --bench parse`. Machine: Darwin 25.3.0.

| Bench | Throughput |
|-------|------------|
| `parse_synthetic/soh` (4 MiB amplified) | ~244 MiB/s |
| `parse_synthetic/pipe` (4 MiB amplified) | ~243 MiB/s |
| `parse_synthetic/prefix` (4 MiB amplified) | ~289 MiB/s |
| `parse_real/fix44_om` (2.1 MB, OM log) | ~314 MiB/s |
| `parse_real/fixt11_md` (8.7 MB, MD log) | ~1.1 GiB/s |
| `parse_known_soh/8MiB` (SOH, no prefix, no sniff) | ~243 MiB/s |

FIXT MD is fastest because market-data messages are long (fewer `read_field` iterations per byte). Use the SOH amplified and `parse_known_soh` lines as the anti-regression anchors for Phase 2.

## Index build throughput (2026-04-17, criterion `--quick`)

Run: `cargo bench -p fixlog-index --bench index`. Machine: Darwin 25.3.0, `rayon::current_num_threads()` ≈ 10 on the bench host.

| Bench | Throughput |
|-------|------------|
| `index_real/single_thread/fix44_om` (2.1 MiB) | ~203 MiB/s |
| `index_real/parallel/fix44_om`                | ~492 MiB/s (**2.4×**) |
| `index_real/single_thread/fixt11_md` (8.7 MiB)| ~730 MiB/s |
| `index_real/parallel/fixt11_md`               | ~2.2 GiB/s (**3.0×**) |
| `index_amplified/single_thread_40MiB`         | ~211 MiB/s |
| `index_amplified/parallel_40MiB`              | ~1.08 GiB/s (**5.1×**) |

Buffers below 1 MiB auto-fall-back to the single-thread path (thread dispatch overhead dominates below that).

## TUI performance (2026-04-17, criterion `--quick`)

Run: `cargo bench -p fixlog-tui --bench frame`. Machine: Darwin 25.3.0. Amplified fixture = `minimal_4.4.log × 100k ≈ 1M messages`.

| Bench | Time | Notes |
|-------|------|-------|
| `tui_bootstrap/1M_messages` | ~123 ms | mmap + sniff + `build_from_bytes_parallel` + initial `visible` scan |
| `tui_frame/list_detail_status_200x50` | **~737 µs** | list + detail + status render against `TestBackend`; target <16 ms, achieved ~22× under |
| `tui_filter/apply_35eqD_1M` | **~156 µs** | post-hot-tag pushdown (P4-T14). Was ~477 ms pre-P4-T14; 3000× speedup. Target was <100 ms. |

## Analysis performance (2026-04-17, criterion `--quick`)

Run: `cargo bench -p fixlog-analysis --bench analysis`. Machine: Darwin 25.3.0.

| Bench | Time | Target | Status |
|-------|------|--------|--------|
| `analysis/session_build_1M` | ~1.16 s | <1 s | slight miss (single-sample `--quick`); full run should land closer |
| `analysis/order_lookup_1M` (real fix44-om) | ~5 µs | <50 ms | ~9000× under — secondary lookup is O(events in chain) |
| `analysis/histogram_build_1M` | ~500 ms | <500 ms | at target; dominated by `parse_sending_time` |

## What's next, ordered by value

1. **Phase 5** — symbolic query names (P4-T15 deferred), persistent config (`~/.config/fixlog/config.toml` for theme, hot-tag set, saved filters, keybindings, persisted bookmarks), P2-T10 `index` subcommand with serialized cache, FIX 5.0 / 5.0 SP1 dictionaries.
2. **P2-T10 `index` subcommand** (optional / Phase 5) — serialize an index to `<file>.fixlog-idx` with content hash. Unlocks <1s reopen for 100 MiB+ logs; probably lives alongside config persistence.
3. **Hot-tag pre-filter for TUI filter path** — cuts `apply_filter` time on 1M messages from ~480 ms to <100 ms when the expression reduces to AND-of-equalities on hot tags. Implementation lives in `fixlog-tui/src/state.rs` and exploits `SecondaryIndex::lookup` already exposed by `fixlog-index`.
4. **Dict-aware query names** — `MsgType=NewOrderSingle` instead of `35=D`. Thin adapter over `fixlog-dict`; gated by a concrete user ask.
5. **`--strict` flag on parse** — treat checksum mismatches as errors (user-requested feature flag from earlier session).
6. **FIX 5.0 / 5.0SP1 dictionaries** — add to `DICTIONARIES` list in `crates/fixlog-dict/build.rs` if a real fixture needs them.
7. **Persistent TUI config** (Phase 5) — `~/.config/fixlog/config.toml` for theme, hot tags, saved filters, keybindings.

## Known gaps / decisions deferred

- **TUI live filter uses `:filter <expr>` (not `/`)**: Phase 3 plan suggested `/` for the live filter bar; we reserved `/` for search (vim convention, matches ROADMAP) and routed live preview through command mode (`:filter` / `:f`). Every keystroke inside a `filter` command triggers a re-evaluation; `Esc` rolls back via `FilterSnapshot`.
- **TUI filter full-scan, no hot-tag pre-filter yet**: `state::evaluate_visible` scans all messages for every filter. `SecondaryIndex` is populated but unused by the TUI — a future optimisation wins ~5× on hot-tag filters (`35=D`, `49=BROKER1`). Acceptable for <1M messages.
- **TUI follow watcher is stat-based, not `notify`-based**: the event loop already polls at 250 ms via `crossterm::event::poll`; piggy-backing there beats adding a `notify` thread + channel. Up to 250 ms visual latency vs ~instant for `grep --follow`. If a TUI user complains about lag, wire `notify` the way `fixlog-cli/src/commands/grep.rs` does.
- **TUI `io.rs` duplicates `fixlog-cli/src/io.rs`**: two tiny copies of `mmap_file` / `head`. If a third consumer appears, promote to `fixlog-core`.
- **`QueryExpr` is not `Clone`**: bug-adjacent inconvenience that forced `filter_text: Option<String>` on `AppState` (source text kept alongside the compiled expression so snapshot/restore and `n`/`N` can re-parse). Fixing upstream (derive `Clone` where feasible, or redesign `Regex` ownership) would remove the string round-trip.
- **ResolvedMessageOwned duplicates ResolvedMessage<'a>**: we materialise because `Arc<Mmap>` gets swapped under `--follow`, but the duplication is a smell. Could be folded into `fixlog-dict` if the pattern repeats.
- **Parser ignores `LogFormat.line_prefix`**: the tokenizer scans for `8=FIX` via memmem, so variable-length prefixes "just work". `LinePrefix::Fixed(n)` is kept in the sniffer output for display only.
- **Chain selection is per-message, not per-session**: the resolver reads `BeginString` + `ApplVerID` from each message. For FIXT sessions where `ApplVerID` only appears on Logon, non-Logon messages fall back to the default `CHAIN_FIXT11_FIX50SP2`. A session-aware cache could improve accuracy but is out of scope.
- **Custom tags beyond dictionary range**: show as `?` in pretty output, `"name": null` in JSON.
- **`LogIndex.consumed` semantics**: points at the byte immediately past the last *successfully* indexed message, not at the end of the buffer. This is intentional — trailing partial messages (producer flushed half) are re-scanned by `append_from_offset` instead of being claimed-and-lost. Callers that expected `file_size == buf.len()` need to track that separately.
- **Query DSL is tag-number only**: no `MsgType=NewOrderSingle` yet — the parser stays dict-agnostic and `fixlog-query` doesn't depend on `fixlog-dict`. Symbolic names belong in a future thin adapter if we need them, probably in Phase 3 alongside the TUI.
- **Query `!=` and repeating groups**: `N!=X` is true iff *no* occurrence of tag N equals X. For repeating groups with multiple instances of the same tag, this is stricter than "some instance is different" — see module docs in `fixlog-query/src/eval.rs`. Change if real usage complains.
- **Secondary index representation**: `HashMap<(tag, SmallVec<[u8;16]>), Vec<u32>>` — not the `RoaringBitmap` that `ARCHITECTURE.md` aspired to. Roaring is denser for huge files but adds a dep and is slower to iterate; default is faster unless the memory budget becomes tight.
- **`--follow` event handling**: we watch the parent directory non-recursively and accept `Modify(Data|Name|Any)` / `Create` / `Remove` as triggers. `Access(Read)` events are ignored. Polling fallback on 500 ms timeout catches coalesced-away writes on macOS.
- **SIGINT in follow mode**: the follow loop never returns normally. The process relies on the default SIGINT handler to terminate cleanly; we do not install our own handler (no shared state to flush — stdout is flushed after every match).
