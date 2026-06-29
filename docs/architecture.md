# Architecture

`fixlog` is a Rust Cargo workspace that parses FIX (Financial Information eXchange) logs and
exposes them through a CLI and an interactive TUI. The design goal is to process millions of
messages efficiently: zero-copy from the mmapped file all the way to the rendered output, with
parallel indexing on top.

> This file is the human-facing map: layout, crate graph, data flow, libraries, gotchas.
> For dense per-crate internals (state machines, invariants, algorithms) see `docs/agent/crates/*.md`.
> For the authoritative current status (phases, task table, benchmarks) see `docs/agent/state.md`.

## Code layout

```
crates/
├── fixlog-format/    # Format sniffer: separator (SOH/|/^/;), line prefix, line ending, encoding
├── fixlog-parser/    # Zero-copy tokenizer over &[u8] → RawMessage (tag/value slices)
├── fixlog-dict/      # FIX dictionaries (build.rs generates from dictionaries/*.xml) + resolver
├── fixlog-index/     # Offset-based index, append-only growth, hot-tag secondary map, rayon builder
├── fixlog-query/     # Filter DSL: AST + parser + zero-alloc evaluator (=, !=, ~, AND/OR/NOT)
├── fixlog-render/    # Shared output helpers: pretty / JSONL / raw FIX / CSV
├── fixlog-core/      # Facade: re-exports format, parser, dict, index, query, render
├── fixlog-analysis/  # Sessions, order lifecycle, temporal histogram, consolidated orders
├── fixlog-cli/       # Binary `fixlog` (clap): sniff/parse/stats/grep/tui/sessions/orders/histogram
└── fixlog-tui/       # Interactive ratatui + crossterm frontend (virtual list, overlays, follow)
dictionaries/         # Vendored QuickFIX XML schemas (FIX44, FIXT11, FIX50, FIX50SP1, FIX50SP2)
fixtures/             # synthetic/ (versioned, golden) + real/ & orders/ (gitignored, real data)
docs/                 # this directory
```

Workspace: `edition = "2024"`, `rust-version = "1.95"`, `license = "MIT OR Apache-2.0"`.
Release profile is tuned for throughput: `lto = true`, `codegen-units = 1`.

## Crate dependency graph

```
fixlog-format ─┐
fixlog-parser ─┼─► fixlog-dict ─┐
   │           │                ├─► fixlog-render ─┐
   │           ├─► fixlog-index │                  │
   │           └─► fixlog-query │                  │
   └───────────────────────────┴──────────────────┴─► fixlog-core
                                                          │
                                  fixlog-analysis ◄───────┘ (depends on core)
                                       │
            fixlog-cli ──► fixlog-tui ─┘   (cli also depends on core + analysis)
```

Notes:
- `fixlog-parser` depends only on `fixlog-format`. `fixlog-dict` depends only on `fixlog-parser` (raw tags in, names out) — the parser never knows about dictionaries.
- `fixlog-analysis` is **not** re-exported from `fixlog-core` by design: it composes on top of core primitives and stays a distinct layer. `fixlog-cli` and `fixlog-tui` depend on it directly.
- `fixlog-tui` is a library crate driven by the `fixlog tui` CLI subcommand.

## Data flow

1. **Open & sniff.** A file is `mmap`ed read-only (`memmap2`); the first chunk is fed to
   `fixlog_format::sniff`, which returns a `LogFormat` (separator, line prefix, line ending).
2. **Parse.** `fixlog-parser` scans the bytes for `8=FIX` boundaries (via `memchr`/memmem) and
   tokenizes each message into a `RawMessage` of `(tag, &[u8])` slices — zero allocation, zero copy.
   Checksum/BodyLength mismatches are **non-fatal**: the message is still emitted, logged at `debug`.
3. **Resolve (on demand).** `fixlog-dict` maps tags → field names and enum values → labels, picking
   a dictionary *chain* from `BeginString` + `ApplVerID` per message.
4. **Index.** `fixlog-index` builds offsets for every message (rayon-parallel above ~1 MiB) plus a
   secondary hot-tag map (`(tag, value) → [ordinals]`) used to short-circuit equality filters.
5. **Query / render.** `fixlog-query` evaluates a filter AST against `RawMessage`; `fixlog-render`
   writes the result as pretty / JSONL / raw FIX / CSV.
6. **Analyze.** `fixlog-analysis` builds session maps, order timelines (following cancel/replace
   chains), temporal histograms, and consolidated order summaries (streaming, multi-source).
7. **TUI.** `fixlog-tui` keeps the mmap + index in `AppState`, renders a virtual list + resolved
   detail panel, and layers overlays (sessions/orders/diff/marks/histogram/consolidated) on top.

## Key libraries (and why)

| Library | Why |
|---------|-----|
| `ratatui` + `crossterm` | Terminal UI framework + backend for the interactive viewer. |
| `rayon` | Data-parallel index build and histogram extraction (multi-core throughput). |
| `memmap2` | mmap large logs so the OS pages bytes in lazily — no full load into RAM. |
| `memchr` | SIMD-accelerated byte scans for `8=FIX` boundaries and narrow tag extraction. |
| `regex` | Backs the `~` (regex match) operator in the filter DSL; compiled once, `Arc`-shared. |
| `clap` (derive) | CLI argument parsing for the `fixlog` binary. |
| `flate2` | Transparent `.gz` decompression for `orders consolidate` over rotated archives. |
| `arboard` | Clipboard access for the TUI `yank` commands. |
| `thiserror` / `anyhow` | Typed library errors / contextual binary errors. |
| `tracing` | Structured logging (level via `-v`/`-vv` or `RUST_LOG`). |
| `criterion` | Benchmarks (parser, index, analysis, TUI frame budget). |

## Entities and data model

There is **no database**. The "schema" is the FIX protocol itself, encoded as dictionaries:

- `dictionaries/*.xml` — QuickFIX schemas (FIX44, FIXT11, FIX50, FIX50SP1, FIX50SP2). At build
  time `fixlog-dict/build.rs` generates Rust field/enum tables from these. To add a FIX version,
  drop the XML here and wire the chain (see `docs/agent/crates/dict.md`).
- Core in-memory types: `RawMessage` (parser), `LogIndex` + `MessageOffset` (index),
  `QueryExpr` (query), `ResolvedMessage` (dict), and the analysis aggregates
  `SessionMap`, `OrderTimeline`/`OrderEvent`, `Histogram`, `OrderConsolidated`.

## Critical notes for collaborators

- **Never assume one format/version.** The sniffer decides separator/prefix/version. The parser
  scans for `8=FIX` itself, so `LogFormat.line_prefix` is informational only — variable-length
  prefixes "just work". Never hardcode SOH or a single FIX version.
- **Parser ⟂ dictionary.** The parser emits raw tag numbers; the dictionary resolves them. Adding
  a dict version must not touch the parser, and vice versa.
- **Checksum mismatches are non-fatal.** Real logs contain truncated messages, blank lines, and
  broken checksums. The parser emits them and logs at `debug` — it never panics on bad input.
- **`avg_px` is overloaded — this bites people.** `OrderConsolidated.avg_px` is *computed*
  (`notional / cum_qty`, where `notional = Σ LastQty·LastPx` over fills). `OrderEvent.avg_px` is
  the *raw* wire value (tag 6). They are different numbers; pick deliberately.
- **Consolidation dedups fills by ExecID (tag 17).** Fills without tag 17 fail open (counted, with
  a `tracing::warn!`). Cancel/replace chains are coalesced via union-find on `11 → 41`, with a
  guard so a placeholder anchor (`41=NONE` in real broker logs) does not merge unrelated orders.
- **TUI `:consolidated` runs synchronously on the foreground thread** (blocking, no caching /
  invalidation on append yet). Fine for one-shot inspection; a known follow-up for live files.
- **TUI follow is stat-based** (250 ms poll piggy-backed on the event loop), not `notify` — up to
  250 ms visual latency vs. `grep --follow`. Intentional; revisit only if a user complains.
- **`LogIndex.consumed` ≠ buffer end.** It points just past the last *successfully* indexed message
  so trailing partial writes are re-scanned by `append_from_offset`. Don't assume `file_size == buf.len()`.
- **Query DSL is tag-number only** (`35=D`, not `MsgType=NewOrderSingle`). `fixlog-query` stays
  dict-agnostic on purpose.
- **`docs/ARCHITECTURE.md` (uppercase) is the older, partly-aspirational design doc** (e.g. it
  describes a RoaringBitmap secondary index that was not adopted). This file + `docs/agent/*` are
  the current reality. The filesystem here is case-sensitive, so both files coexist.

## See also

- `docs/agent/INDEX.md` — routing table: task type → which internals file to read.
- `docs/agent/state.md` — authoritative phase/status, benchmarks, known gaps.
- `docs/features/` — user-facing capability docs.
- `README.md` — end-user usage with command examples (Spanish).
