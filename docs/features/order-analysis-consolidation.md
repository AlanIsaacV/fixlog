# Order & session analysis, and consolidation

## What it does

Higher-level analytics over a parsed log, all in the `fixlog-analysis` crate:

- **Sessions** — group messages by the canonical `(SenderCompID, TargetCompID)` pair, split in/out
  counts, report `MsgSeqNum` ranges, and detect sequence gaps.
- **Order lifecycle** — reconstruct one order's full timeline by `ClOrdID` (tag 11), following
  cancel (`F`) / replace (`G`) chains via `OrigClOrdID` (tag 41) to a fixed point, with an ASCII
  Gantt bar and a per-event table (LastQty/LastPx vs CumQty/AvgPx).
- **Temporal histogram** — distribution of `SendingTime` (tag 52) into buckets, with an ASCII
  sparkline (percentile-scaled) and top-k traffic peaks.
- **Consolidated orders** — aggregate fills per order across **one or more** logs (including rotated
  `.gz` archives and stdin), streaming, deduplicating fills by ExecID.

## Surface (CLI)

| Command | Notes |
|---------|-------|
| `fixlog sessions FILE [--format pretty\|json]` | Session table or JSONL. |
| `fixlog orders FILE [--id CLORDID] [--limit N] [--format …]` | With `--id`: timeline + Gantt. Without: top-N ClOrdIDs by event count. |
| `fixlog orders consolidate INPUTS... [--format pretty\|csv\|json] [--sort notional\|cumqty\|fills\|recent]` | Consolidated summary across inputs; `.gz` transparent, `-` = stdin. |
| `fixlog histogram FILE [--bucket 1s] [--width 80] [--peaks 5]` | Sparkline + peaks. |

All four are also TUI overlays (`:sessions`, `:orders`/`O`, `:histogram`, `:consolidated`).

## Files involved

- `crates/fixlog-analysis/src/` — `sessions.rs`, `orders.rs` (`OrderTimeline`, `OrderEvent`),
  `histogram.rs`, `orders_consolidated.rs` (`ConsolidatedBuilder`, `OrderConsolidated`), `util.rs`
  (`extract_tag_raw`), `AnalysisError`. See `docs/agent/crates/analysis.md`.
- `crates/fixlog-cli/src/commands/{sessions,orders,orders_consolidate,histogram}.rs`.
- `crates/fixlog-cli/src/io.rs` — `InputSource`, `open_source` (`.gz` via `flate2::MultiGzDecoder`,
  stdin via `-`).
- TUI overlays: `crates/fixlog-tui/src/view/{sessions,orders,histogram,consolidated}.rs`.

## Data flow

- **Sessions / orders / histogram** read the built `LogIndex` (orders use the secondary hot-tag
  lookup on tags 11/37/41) and materialize aggregates.
- **Consolidation is streaming, not index-driven**: `ConsolidatedBuilder::push_source` reads each
  input in 1 MiB chunks (carrying the trailing partial across reads), resolves cancel/replace chains
  with union-find on `11 → 41`, and accumulates per-order state keyed by root ClOrdID. `finish()`
  returns `Vec<OrderConsolidated>`.

## Key fields & invariants

- **`OrderConsolidated.avg_px` is computed** (`notional / cum_qty`, `notional = Σ LastQty·LastPx`),
  **not** tag 6. `OrderEvent.avg_px` *is* raw tag 6. Different numbers — choose deliberately.
- **Fill dedup is by ExecID (tag 17)**; fills lacking tag 17 fail open (counted, `tracing::warn!`).
- **Placeholder-anchor guard**: if an `OrigClOrdID` was never seen as a tag-11 in the stream
  (e.g. `41=NONE` in real broker logs), the order becomes its own root instead of merging unrelated
  orders that share the placeholder.
- **Multi-source ≡ single-source**: splitting a log at a `8=FIX` boundary and feeding the halves
  (one plain, one `.gz`) produces byte-identical consolidated output (regression-tested).

## Performance

- `analysis/histogram_build_1M` ~32 ms (rayon-parallel timestamp extraction + narrow tag scan).
- `analysis/order_lookup_1M` ~5 µs (secondary lookup, O(events in chain)).
- `orders consolidate` over the ~540 MB orders fixture: ~55k orders in under 5 s.

## Edge cases

- **Truncated `.gz`** surfaces as `AnalysisError::Io` / `io::Error`, never a panic.
- **Sessions gap detection** re-parses ordinals per session and sorts by `MsgSeqNum` per direction.
- **FIXT chain selection** for non-Logon messages without `ApplVerID` falls back to FIX 5.0 SP2.
