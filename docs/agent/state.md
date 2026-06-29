# Project state

> Snapshot of what exists and what's next. The single source of truth for status (the original `PHASE*_PLAN.md` files have been retired; their task tables live below).

## Phase

**Fase 4 — Análisis avanzado** — **effectively complete** (P4-T01 through P4-T14, T16, T17 landed; T15 symbolic names deferred to Fase 5 by default). New crate `fixlog-analysis` implements session tracking, order lifecycle, temporal histogram. CLI gains `sessions` / `orders` / `histogram` subcommands. TUI gains sessions / orders / diff / marks / histogram overlays, bookmark set/jump (`m<letter>` / `'<letter>`), diff slots (`dd` / `dD`), and `:export` (csv/json/fix/pretty). Hot-tag pre-filter drops `tui_filter/apply_35eqD_1M` from ~477 ms to ~156 µs (~3000× speedup).

**Fase A — TUI rediseño (inspirado en fixparser.targetcompid.com, 2026-04-18)** — **done, committed**. List columns `TIME | MESSAGE | CLIENT ORDER ID | STATUS | DETAIL`; `c` toggle for skipping common header/trailer tags in the detail panel; `H` toggle composing `AND NOT 35=0` through `recompute_effective_filter` + extended `FilterSnapshot`. TUI re-parse sites migrated to `parse_one_with_format` so pipe-separated logs resolve.

**Fase B — TUI rediseño, riqueza semántica (2026-04-18)** — **done, committed**. New `summary` module with a per-MsgType table (D/8/F/G/3/j/A/5/0/1/2) producing `MessageSummary { badges, client_order_id, detail }`; unknown MsgTypes fall through to the generic Side/Qty/Symbol fallback. `list::build_line` consumes `summary::summarize` directly; duplicated helpers dropped. Panel focus extended: `AppState.detail_cursor` + `detail_fields_len` + `detail_cursor_field`; `j`/`k`/`g`/`G`/`Ctrl+D/U` in `Focus::Detail` move the per-field cursor (viewport auto-scrolls); the renderer highlights the cursor row. `Action::FilterFromDetail { negated }` bound to `f`/`x` composes `tag=value` / `NOT (tag=value)` into `user_filter_text` via `recompute_effective_filter`; bareword-vs-quoted value heuristic honours the DSL grammar.

**Fase 5 partial (2026-04-18)** — **4 items done, committed**. (1) `fixlog-analysis` session / order / histogram builders + `fixlog-cli orders` migrated to `parse_one_with_format` so pipe-separated logs resolve (regression test: `crates/fixlog-analysis/tests/pipe_separated.rs`). (2) `QueryExpr` now derives `Clone` via `Op::Re(Arc<Regex>)`; TUI `FilterSnapshot` carries the compiled expression directly and `iterate_search` uses `search_last.clone()` — no more re-parse on `Esc` rollback or `n`/`N`. Frame bench neutral at ~156 µs. (3) New `fixlog-render` crate (`0.1.0`) with `write_pretty` / `write_jsonl` / `write_fix` / `write_csv_header+row`, re-exported from `fixlog-core::render`; CLI `parse` / `grep` and TUI `:export` all consume it. (4) FIX 5.0 and 5.0 SP1 dictionaries added (XMLs vendored from QuickFIX v1.15.1); `FixVersion::{Fix50, Fix50Sp1}`, chains `CHAIN_FIXT11_FIX50` / `CHAIN_FIXT11_FIX50SP1`, `chain_for` routes ApplVerID `7`/`8` accordingly (SP2 remains default for unknown/missing). Plan atómico formalizado en `docs/PHASE5_PLAN.md` (2026-04-19, P5-T01..T04 marcadas `[x]`; P5-T05..T11 backlog).

**Docs refresh (2026-04-19)** — **done, uncommitted**. README expanded with Fase 4 / A / B / 5 surface: new `sessions` / `orders` / `histogram` CLI sections, full TUI keybinding / command / overlay / two-key-sequence tables, detail-panel navigation docs, hot-tag performance row, FIX 5.0 / SP1 in the supported-versions table. New `:help` overlay (`crates/fixlog-tui/src/view/help.rs` + `Overlay::Help { scroll }`) replaces the single-line status message; supports `j`/`k`/`Ctrl+D/U`/`g`/`G` scrolling via `app::overlay_intercept`, closes on `Esc`. 6 integration tests in `crates/fixlog-tui/tests/help.rs`. `docs/PHASE5_PLAN.md` created.

**UX polish pass (2026-04-19)** — **5 fixes + 2 derived overlay rewrites landed this session**.

1. **Histogram perf — 94% faster.** `Histogram::build` now parallelizes the timestamp extraction pass over `rayon::into_par_iter` and uses a new narrow scan helper `util::extract_tag_raw(bytes, format, tag)` that short-circuits on the first match instead of tokenising the whole message via `parse_one_with_format`. Min/max/drop-count/timestamp collection fold into a single sequential pass over the parallel results — no separate `.iter().min()/.max()` scans, no intermediate Vec of parsed messages. `analysis/histogram_build_1M` went from ~573 ms to ~32 ms (−94%, bench-confirmed). `rayon` added as a dep of `fixlog-analysis`.
2. **`:orders` follows OrigClOrdID.** `OrderTimeline::build` now operates on a worklist over the family of ClOrdIDs reachable through tag 41 (OrigClOrdID), iterating to fixed point so `A → B → C` replacement chains surface via a single query. Tag 41 is indexed as a new hot tag (`TAG_ORIG_CL_ORD_ID`; added to `HotTags::default_set`). `OrderEvent` also exposes `last_px: Option<SmallVec<[u8; 16]>>` (tag 31, LastPx). 2 new unit tests: `cancel_request_without_order_id_is_included`, `replace_chain_is_followed_transitively`.
3. **TUI — `?` opens `:help`.** New `Action::OpenHelp` mapped from `KeyCode::Char('?')` (any modifier) in Normal mode; handler opens `Overlay::Help { scroll: 0 }` directly without going through command mode.
4. **TUI — 3-key `dd` + `D` diff.** Pressing `D` (shift-d, no prior `d` prefix) now opens the diff overlay if slot A is already set, so `dd` then `D` works as a 3-keystroke sequence. The 4-keystroke `dd` + `dD` path still works via the existing pending-prefix arm. Error text tightened to `"press dd to set diff slot A first"` when neither slot is set.
5. **TUI — marks restricted to digits 0–9.** `m<letter>` and `'<letter>` replaced by `m<digit>` / `'<digit>`. When `pending_prefix` is `m` or `'`, `on_event` routes through the new `map_event_digit_priority` instead of `map_event`: ASCII digits become `Action::Letter(c)` regardless of dedicated bindings (notably `0` → `ScrollHome` is shadowed for the duration of the prefix), and letters fall through to their normal shortcuts so an accidental follow-up keypress can never silently mis-set a mark on the wrong character. `set_bookmark`/`jump_bookmark` validate `is_ascii_digit()` and warn on anything else.

**Derived overlay rewrites (2026-04-19)** — same session:

6. **Orders overlay — navigable with timestamps + lastPx.** `Overlay::Orders` gained a `cursor: usize`; `overlay_intercept` routes `j`/`k`, `Ctrl+D`/`Ctrl+U`, `g`/`G` to move the cursor, and `Enter` calls `jump_to_orders_selection()` which closes the overlay and moves the main cursor to `events[cursor].ordinal` (warns without closing if that ordinal is filtered out of `visible`). The event table column layout changed from `ord | type | exec | status | cum-qty` to `time | type | exec | status | cum-qty | last-px`: the original ordinal column carried no user value, so we replaced it with `SystemTime` formatted as `YYYY-MM-DD HH:MM:SS` (UTC) via a local `civil_from_days` helper (inverse of Hinnant's civil-from-days) — no new date dep. Selected row highlight: `▶ ` prefix + dark-gray background. 2 new unit tests in `view/orders.rs`.
7. **Marks overlay — same columns as the list.** `view/marks.rs` was `letter | ordinal | preview`; now it renders the full list schema (`time | message | client order id | status | detail`) prefixed by a short `mark` column. Achieved by promoting `COL_TIME`, `COL_MESSAGE`, `COL_CLORDID`, `COL_STATUS`, `COL_SEP`, `header_line`, and a new ordinal-driven `build_line_for_ord(state, ord)` (extracted from the old `build_line`) to `pub(crate)` in `view/list.rs`. `Overlay::Marks { cursor }` gained the same navigation surface as Orders with `jump_to_marks_selection()` applied on `Enter`. Help overlay updated to read `m<0-9>` / `'<0-9>`.

All gates green: `cargo test --all` (229 tests), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all`. Bench: `analysis/histogram_build_1M: [30.838 ms 32.310 ms 32.986 ms] change: [-94.819% -94.215% -93.555%]`.

**Feature drop (2026-05-15)** — **consolidated order summaries across rotated logs** (4 phases, builder + CLI + TUI overlay).

1. **`fixlog-analysis` — `orders_consolidated` module.** New `ConsolidatedBuilder` with `push_source<R: Read>(R, &LogFormat) -> Result<SourceStats, AnalysisError>` and `finish() -> Vec<OrderConsolidated>`. **Streaming, not index-driven**: parses messages off an `impl Read` in 1 MiB chunks, carries the trailing partial across `read()` calls, and aggregates per-order state in a `HashMap<root_clordid, OrderAcc>`. Cancel/replace chains (`11 → 41`) are resolved with a union-find on a `HashMap<Vec<u8>, Vec<u8>>` alias map (path compression on `resolve_root`). **Guard against placeholder anchors**: if `OrigClOrdID` was never observed as a `tag 11` in the stream (case `41=NONE` in real broker logs), the new ClOrdID becomes its own root rather than coalescing every order that shares the placeholder. `OrderConsolidated` fields: `root_clordid`, `family: SmallVec`, `symbol`, `side`, `order_qty`, `cum_qty`, `notional` (= `Σ LastQty·LastPx` over fills), `avg_px` (computed `notional / cum_qty`, **not** tag 6), `fills`, `final_ord_status`, `first_seen`, `last_seen`. **Fill dedup is by ExecID (tag 17)** with `HashSet<Vec<u8>>` per order; fills without tag 17 fail open with a `tracing::warn!` and are counted. New `AnalysisError::Io(std::io::Error)` for streaming failures. Adds `memchr = "2"` as a dep (used for `8=FIX` boundary scans). 5 unit tests covering dedup, replace chain coalescence, notional accuracy, fill-without-ExecID, multi-source ≡ single-source.

2. **`fixlog-cli` — `orders consolidate` subcommand + `.gz` ingestion.** New `io::InputSource::{File, Stdin}` + `open_source(&InputSource) -> Result<Box<dyn BufRead>>` with transparent `MultiGzDecoder` for `.gz` paths (matching `cat a.gz b.gz` concatenated members) and `BufReader<Stdin>` for `-`. Adds `flate2 = "1"`. The new subcommand: `fixlog orders consolidate <inputs...> [--format pretty|csv|json] [--sort notional|cumqty|fills|recent]`. Sniff strategy: take the first 64 KiB of the first input (descomprimido si `.gz`) to `fixlog_format::sniff`, then re-emit those bytes ahead of the rest via `Cursor::new(prefix).chain(reader)` so we don't lose the head; remaining inputs assume the same format. **Backward compatibility**: `fixlog orders <file> [--id ...]` keeps timeline mode — the existing `Orders` clap variant became `OrdersArgs` with `#[command(args_conflicts_with_subcommands = true)]` and an `Option<OrdersSub::Consolidate>`. Exit 1 when no orders are aggregated. 3 `io::tests` (plain/gz equivalence, truncated gz emits `io::Error`, `-` parses as Stdin) + 6 integration tests (`tests/orders_consolidate.rs`) including a `head_plain + tail.gz == whole` test that splits a fixture at a `8=FIX` boundary, gzips the tail, and verifies multi-input + descompresión preserves the consolidated CSV byte-for-byte. Smoke against the 540 MB orders fixture (`fixtures/orders/FIXT.1.1-…log`) reports 55 136 orders in <5 s.

3. **`fixlog-tui` — `Overlay::Consolidated` + `:consolidated` command.** New `Overlay::Consolidated { rows: Vec<OrderConsolidated>, cursor: usize }`. `:consolidated` (alias `:consolidate`) in `command::execute` calls `open_consolidated_overlay` which runs the same `ConsolidatedBuilder` over `Cursor::new(&state.mmap[..])` — blocking on the foreground thread; caching + invalidation on append are intentionally deferred. `view::consolidated::render` shows a `ratatui::Table` with `ClOrdID | Side | Symbol | OrderQty | CumQty | Notional | AvgPx | Status | Fills`, totals header (`orders / fills / notional`), and status-aware row colors (Filled green, PartFilled yellow, Cancelled/Rejected red, Expired dark gray, New/PendingNew blue, Replaced/PendingReplace cyan). Cursor nav (`j/k`, `Ctrl+D/U`, `g/G`) wired through `app::overlay_intercept`. **`Enter` drill-down**: `App::drill_into_consolidated_selection` reads the row's `root_clordid`, calls the existing `command::open_orders_overlay` with it, and the overlay transitions to `Overlay::Orders { timeline, cursor }` — the consolidated table is the natural entry point into the per-order timeline + Gantt that already existed. `:help` updated. `tempfile` added as dev-dep so the 3 new unit tests can bootstrap a real `AppState` from a synthetic on-disk log.

All gates green: `cargo test --all` (no regressions, +14 tests across the feature: 5 builder unit + 3 cli io unit + 6 cli integration + 3 tui overlay), `cargo clippy --all-targets --all-features -- -D warnings`. Manual: `cargo run --release -p fixlog-cli -- orders consolidate fixtures/orders/*.log` produces the aggregated table; `--format json | jq '.[0]'` and `--format csv | head` round-trip cleanly; `fixlog tui …` then `:consolidated` opens the overlay; `Enter` on a row drills into the existing `:orders` timeline.

**Consolidated overlay perf pass + UX fixes (2026-05-15)** — **3 fixes landed, uncommitted**.

1. **Overlay render is O(viewport), not O(rows).** `Overlay::Consolidated` now carries `view: Arc<ConsolidatedView>` + `cursor` + `viewport_top` (sticky, clamped per frame in the render — mirrors `AppState::ensure_cursor_visible` for the main list). `ConsolidatedView::from_rows` consumes the raw `Vec<OrderConsolidated>` once at open and produces `Vec<ConsolidatedDisplayRow>` where every cell is `Cow<'static, str>`: static placeholders (`"-"`), `side_label` constants, and `chain_enum_value_label` dictionary hits stay `&'static`; only `fmt_int` / `fmt_money` / `fmt_price` / `lossy` output is owned. `view::consolidated::render` takes `&mut AppState` and builds `Row<'_>` only for the visible slice `view.rows[viewport_top..viewport_top+h]`. The summary line is precomputed in `from_rows` (previously two full-vec iterations per frame). On 100k-order logs the per-frame cost went from ~15 MB of `Row` allocations + 9 string allocs per row to ~50 row allocations with zero per-frame formatting; mouse-wheel scroll backlog (terminal converts wheel-ticks to `Up`/`Down` events) is gone. The Arc keeps `app.state.overlay.clone()` in the draw loop cheap.
2. **Order timeline shows per-fill *and* cumulative columns.** `OrderEvent` (in `fixlog-analysis::orders`) gained `last_qty: Option<SmallVec<[u8; 16]>>` (tag 32) and `avg_px: Option<SmallVec<[u8; 16]>>` (tag 6, **raw bytes from the wire** — distinct from `OrderConsolidated.avg_px` which is the computed `notional / cum_qty`). `view::orders` table is now `time | type | exec | status | LastQty | LastPx | CumQty | AvgPx`; per-fill columns sit next to the cumulative ones so the distinction is visually obvious.
3. **`Esc` from a drilled-into overlay returns to the parent.** New `AppState.previous_overlay: Option<Overlay>` one-slot stack + helpers `open_overlay(new)` / `esc_overlay()` / `close_overlay()`. All user-initiated overlay opens go through `open_overlay` (clears `previous_overlay`); the drill-down site (`App::drill_into_consolidated_selection`) clones the parent, calls `open_orders_overlay`, and re-parks the parent only if the drill succeeded. `Esc` (`Action::OverlayClose`) routes to `esc_overlay()` which pops the parent if any. So: `:consolidate` → `Enter` on a row → Orders timeline → `Esc` returns to the consolidated list (not all the way to the main message view); `:orders ABC` from the main list keeps the old behaviour (no parked parent → `Esc` closes normally).

All gates green: `cargo test --all` (no regressions across the whole workspace), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all`. No new benches — the win is structural (O(rows) → O(viewport) + zero-formatting render) and visible by inspection. See `crates/tui.md` §"Consolidated overlay" + §"Overlay stack" + §"Orders overlay" and `crates/analysis.md` §"`OrderEvent` fields" for details.

**Feature drop (2026-04-21)** — **3 features landed in 2 commits** (plus this docs sync).

1. **TUI — sort by tag 34 / 52 / 60.** New `SortKey::{Natural, MsgSeqNum, TransactTime, SendingTime}` in `crates/fixlog-tui/src/state.rs`; `sort_visible()` uses `sort_by_cached_key` over a tuple of `(present_flag, extracted_value, ordinal)` so the extractor runs once per row and the sort is stable (ties fall back to file order — what you want for resend duplicates). Byte-lexicographic compare on tags 52/60 is chronologically correct thanks to the fixed-width FIX UTC timestamp format — no `SystemTime` parse on the hot path. `CycleSortKey` action on `o` (Normal mode) rotates through the four keys, preserves the cursor on the message it pointed at, and emits a status-bar toast. Statusline shows `[sort:…]` when non-natural; `:help` overlay documents the rotation. CLI: `--sort {natural|seq|transact|sending}` on `fixlog tui`. `TuiConfig` gains a `Default` impl so existing integration tests migrate to `..Default::default()`. `TAG_TRANSACT_TIME = 60` added to `fixlog-parser` alongside the existing tag constants. One new integration test (`sort_visible_respects_sort_key`) with a synthetic log that duplicates tag 34 + tag 60 (resend-request shape) to verify stability.
2. **CLI — pipe input via stdin.** `fixlog tui` now accepts the file arg as `Option<PathBuf>`. When stdin is a pipe (not a TTY), `drain_stdin_to_tmpfile` copies it to a `tempfile::NamedTempFile`, mmaps that, and calls `libc::dup2(/dev/tty, STDIN_FILENO)` so crossterm's raw-mode reader still gets keystrokes after the pipe is consumed. Unix-only (`libc` dep behind `[target.'cfg(unix)'.dependencies]`); Windows keeps the file-required path. Rejects `--follow` (pipes don't grow) and empty stdin with clear error messages.
3. **Dict — repeating-group rendering.** New `fixlog-dict::groups::group_members(counter) -> Option<&'static [u32]>` returns the union of member tags that may appear inside a known `NumInGroup` group. Currently registers **only `268 NoMDEntries`** — covers `MarketDataSnapshotFullRefresh (W)` and `MarketDataIncrementalRefresh (X)`, which is the most common group shape in real logs. Resolver tracks an `active_group: Option<&'static [u32]>` and sets `ResolvedField.depth = 1` on members; a non-member tag closes the run so a stray field bounds the group cleanly (no full-schema awareness required). TUI detail panel indents `depth = 1` rows with `  └ ` in dark-gray and paints the counter itself yellow + bold. Auto-deriving the full counter table from `dictionaries/*.xml` at build time is tracked as P5-T10 follow-up work.

All gates green: `cargo test --all` (253 tests, +24 new from the sort test + marks/orders carried over), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all`. Manual: `cargo run -p fixlog-cli -- tui fixtures/real/fixt11-md.log` shows MDEntry members indented; `cargo run -p fixlog-cli -- tui fixtures/real/fix44-om.log --sort seq` reorders by MsgSeqNum; `rg "35=D" fixtures/real/fix44-om.log | cargo run -p fixlog-cli -- tui` opens the pipe input.

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
- `crates/fixlog-render` — shared rendering helpers (`write_pretty`, `write_jsonl`, `write_fix`, `write_csv_header` + `write_csv_row`). Depends on `fixlog-dict` + `fixlog-parser` only. Re-exported as `fixlog_core::render`; consumed by `fixlog-cli` (parse/grep) and `fixlog-tui` (`:export`). Added in Fase 5 partial (2026-04-18).

## Completed — Phase 1

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

## Completed — Phase 2

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

## Completed — Phase 3

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

## Completed — Phase 4

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
| `analysis/histogram_build_1M` | ~32 ms | <500 ms | **2026-04-19**: rewritten with rayon + narrow tag extract; was ~573 ms pre-rewrite |

## What's next, ordered by value

1. **P2-T10 `index` subcommand** (optional / Phase 5) — serialize an index to `<file>.fixlog-idx` with content hash. Unlocks <1s reopen for 100 MiB+ logs; probably lives alongside config persistence.
2. **`--strict` flag on parse** — treat checksum mismatches as errors (user-requested feature flag from earlier session).
3. **Persistent TUI config** (Phase 5) — `~/.config/fixlog/config.toml` for theme, hot tags, saved filters, keybindings, persisted bookmarks.

Items previously in this list and since done in Fase 5 partial (2026-04-18): Arc-wrapped `QueryExpr: Clone`, `fixlog-render` crate promotion, FIX 5.0 / 5.0 SP1 dictionaries, analysis/CLI `parse_one_with_format` migration. Symbolic query names (P4-T15) removed from the roadmap per the 2026-04-18 decision log.

## Known gaps / decisions deferred

- **TUI live filter uses `:filter <expr>` (not `/`)**: Phase 3 plan suggested `/` for the live filter bar; we reserved `/` for search (vim convention, matches ROADMAP) and routed live preview through command mode (`:filter` / `:f`). Every keystroke inside a `filter` command triggers a re-evaluation; `Esc` rolls back via `FilterSnapshot`.
- **TUI filter full-scan, no hot-tag pre-filter yet**: `state::evaluate_visible` scans all messages for every filter. `SecondaryIndex` is populated but unused by the TUI — a future optimisation wins ~5× on hot-tag filters (`35=D`, `49=BROKER1`). Acceptable for <1M messages.
- **TUI follow watcher is stat-based, not `notify`-based**: the event loop already polls at 250 ms via `crossterm::event::poll`; piggy-backing there beats adding a `notify` thread + channel. Up to 250 ms visual latency vs ~instant for `grep --follow`. If a TUI user complains about lag, wire `notify` the way `fixlog-cli/src/commands/grep.rs` does.
- **TUI `io.rs` duplicates `fixlog-cli/src/io.rs`**: two tiny copies of `mmap_file` / `head`. If a third consumer appears, promote to `fixlog-core`.
- ~~`QueryExpr` is not `Clone`~~: **resolved in Fase 5 partial (2026-04-18)** by wrapping `Op::Re` in `Arc<Regex>`. `FilterSnapshot` now stores the compiled expression; `iterate_search` clones it instead of re-parsing.
- **ResolvedMessageOwned duplicates ResolvedMessage<'a>**: we materialise because `Arc<Mmap>` gets swapped under `--follow`, but the duplication is a smell. Could be folded into `fixlog-dict` if the pattern repeats.
- **Parser ignores `LogFormat.line_prefix`**: the tokenizer scans for `8=FIX` via memmem, so variable-length prefixes "just work". `LinePrefix::Fixed(n)` is kept in the sniffer output for display only.
- **Chain selection is per-message, not per-session**: the resolver reads `BeginString` + `ApplVerID` from each message. For FIXT sessions where `ApplVerID` only appears on Logon, non-Logon messages fall back to the default `CHAIN_FIXT11_FIX50SP2` (SP2 covers the common-case wire format). A session-aware cache could improve accuracy but is out of scope. FIX 5.0 and 5.0 SP1 are now valid routing targets when `ApplVerID=7` / `ApplVerID=8` appears explicitly on the message.
- **Custom tags beyond dictionary range**: show as `?` in pretty output, `"name": null` in JSON.
- **`LogIndex.consumed` semantics**: points at the byte immediately past the last *successfully* indexed message, not at the end of the buffer. This is intentional — trailing partial messages (producer flushed half) are re-scanned by `append_from_offset` instead of being claimed-and-lost. Callers that expected `file_size == buf.len()` need to track that separately.
- **Query DSL is tag-number only**: no `MsgType=NewOrderSingle` yet — the parser stays dict-agnostic and `fixlog-query` doesn't depend on `fixlog-dict`. Symbolic names belong in a future thin adapter if we need them, probably in Phase 3 alongside the TUI.
- **Query `!=` and repeating groups**: `N!=X` is true iff *no* occurrence of tag N equals X. For repeating groups with multiple instances of the same tag, this is stricter than "some instance is different" — see module docs in `fixlog-query/src/eval.rs`. Change if real usage complains.
- **Secondary index representation**: `HashMap<(tag, SmallVec<[u8;16]>), Vec<u32>>` — not the `RoaringBitmap` that the original design doc aspired to. Roaring is denser for huge files but adds a dep and is slower to iterate; default is faster unless the memory budget becomes tight.
- **`--follow` event handling**: we watch the parent directory non-recursively and accept `Modify(Data|Name|Any)` / `Create` / `Remove` as triggers. `Access(Read)` events are ignored. Polling fallback on 500 ms timeout catches coalesced-away writes on macOS.
- **SIGINT in follow mode**: the follow loop never returns normally. The process relies on the default SIGINT handler to terminate cleanly; we do not install our own handler (no shared state to flush — stdout is flushed after every match).
