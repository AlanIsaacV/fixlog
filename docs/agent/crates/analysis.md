# fixlog-analysis

Pure library for semantic analysis over an already-indexed FIX log.
Consumers (CLI, TUI) pass in `&LogIndex`, `&[u8]`, `&LogFormat`;
builders return owned structures that survive an `Arc<Mmap>` swap under
`--follow`.

Four modules, one `AnalysisError` enum. `orders_consolidated` performs
streaming I/O over `impl Read`; everything else is pure (no I/O).
No async.

## Module layout

| Module                | Types                                                       | Purpose                                                                  |
|-----------------------|-------------------------------------------------------------|--------------------------------------------------------------------------|
| `sessions`            | `SessionKey`, `SessionStats`, `SeqGap`, `SessionMap`        | Aggregate by `(49, 56)` canonical pair; detect `MsgSeqNum` gaps.         |
| `orders`              | `OrderEvent`, `OrderTimeline`, `render_gantt`               | Reconstruct a ClOrdID chain across `11`/`37` crossrefs; Gantt renderer.  |
| `orders_consolidated` | `OrderConsolidated`, `ConsolidatedBuilder`, `SourceStats`   | Streaming per-order summary over one or more `impl Read` sources.        |
| `histogram`           | `Bin`, `Histogram`                                          | Uniform-width temporal histogram over tag 52; sparkline + peaks.         |
| `util`                | `find_tag`, `parse_u32_ascii`, `parse_sending_time`         | Internal helpers; `pub` for benches and future modules.                  |

## Invariants

1. **Owned across mmap boundary.** `SessionKey` stores `Vec<u8>`;
   `OrderEvent` / `OrderTimeline` store `SmallVec<[u8; N]>` / `Vec<u8>`.
   Never hold a `&[u8]` from the mmap across an `append_from` /
   follow-mode tick.
2. **Skip + log on corrupt msgs.** `parse_one` failures are silently
   skipped. The design mirrors the indexer's "warn + skip" contract.
3. **`append_from` is contiguous.** `SessionMap::append_from(from_ord, …)`
   panics if `from_ord != self.by_ordinal.len()`. Analog of the
   `LogIndex::append_from_offset(from == consumed)` invariant.
4. **No dict dep in parser.** `fixlog-analysis` may depend on dict via
   `fixlog-core` (for symbolic MsgType labels in CLI rendering), but
   the parser crate itself never does.
5. **`#![forbid(unsafe_code)]`** at the top of `lib.rs` — pure library.

## Sessions (`sessions.rs`)

### Key identity — canonical pair

`SessionKey::canonical(tag49, tag56)` puts the lex-smaller endpoint in
`sender` and the lex-larger in `target`. This collapses the two
directions of a bidirectional session into a single entry. Consumers
that care about direction read each message's actual `tag 49`.

### Direction — `in_count` / `out_count`

"In" / "out" are **roles within the canonical pair**, not real-world
directionality (which the log doesn't encode):

- `in_count` = messages where `tag 49 == key.sender` (canonical-smaller).
- `out_count` = messages where `tag 49 == key.target` (canonical-larger).

### Gap detection

After the aggregation pass, `recompute_gaps` re-parses every ordinal
belonging to the session and splits `(seq, ordinal)` pairs by direction.
Each direction is sorted ascending by `seq`; any adjacent pair with
`seq[i+1] > seq[i] + 1` yields a `SeqGap`. Both directions' gaps are
combined in `SessionStats.gaps` (unordered).

### Algorithm complexity

`SessionMap::build` is O(n + n log k) where n is messages, k the number
of distinct sessions. The re-parse-for-gaps pass doubles the constant.
Bench: ~1.16 s over 1M messages (target <1 s; within 20% at --quick;
acceptable).

## Orders (`orders.rs`)

### Algorithm

1. `index.secondary.lookup(11, clordid)` → initial ordinals.
2. Re-parse each, collect observed `tag 37` values.
3. For each `37` value, expand via `index.secondary.lookup(37, v)`.
4. Dedup, sort ascending, materialize `OrderEvent`s.

The cross-lookup is critical: `F` (cancel) and `G` (replace) messages
change the ClOrdID but keep the OrderID, so a plain `lookup(11)` misses
the execution reports that follow.

### `OrderEvent` fields

Each materialized event carries per-fill *and* cumulative bytes so the
TUI timeline can render both columns side-by-side and the reader can
tell "this fill" apart from "running total":

| Field         | Tag | Per-fill / cumulative | Notes                                    |
|---------------|-----|-----------------------|------------------------------------------|
| `msg_type`    | 35  | —                     | `SmallVec<[u8; 2]>`                      |
| `sending_time`| 52  | —                     | Parsed to `SystemTime` via `util`        |
| `exec_type`   | 150 | per-fill              | A/0/5/F/4/8 → color in the TUI overlay   |
| `ord_status`  | 39  | running               | Final state at this event                |
| `last_qty`    | 32  | per-fill              | Added 2026-05-15; raw bytes              |
| `last_px`     | 31  | per-fill              | Raw bytes                                |
| `cum_qty`     | 14  | cumulative            | Raw bytes                                |
| `avg_px`      |  6  | cumulative            | Added 2026-05-15; raw bytes from tag 6 — **not** the computed `notional / cum_qty` exposed by `OrderConsolidated.avg_px` |

All `last_*` / `cum_qty` / `avg_px` values are stored as
`Option<SmallVec<[u8; 16]>>` — we never parse the numeric value at
build time; downstream renderers do their own formatting (the TUI
overlay just calls `String::from_utf8_lossy`, the CLI doesn't expose
these yet).

The distinction matters when reading both `OrderEvent.avg_px` (this
module) and `OrderConsolidated.avg_px` (next section): the former is
whatever the broker emitted in tag 6 of the ExecutionReport at that
point in time; the latter is a recomputed `Σ notional / Σ cum_qty`
over deduped fills. They will usually but not always agree —
brokers occasionally round tag 6 differently than the implicit
running average.

### `render_gantt(timeline, width)`

Width is clamped to `[10, 500]`. Character map:

| MsgType | Char | Meaning             |
|---------|------|---------------------|
| `D`     | `N`  | NewOrderSingle      |
| `8`     | `X`  | ExecutionReport     |
| `F`     | `C`  | OrderCancelRequest  |
| `G`     | `R`  | OrderCancelReplace  |
| `3`,`j` | `!`  | Reject              |
| other   | `?`  | unknown             |

Events at the same time-column collide; later ones win the slot. For
fine-grained inspection, the event table below the Gantt row is
authoritative.

### Performance

`order_lookup_1M`: ~5 µs per build (target <50 ms; 9000× under).
Secondary-index-first makes the lookup effectively O(events-in-chain).

## Consolidated orders (`orders_consolidated.rs`)

Streaming per-order summary builder. Unlike `orders::OrderTimeline`
(which works off a `LogIndex` over a single mmap), this builder operates
on **one or more `impl Read` sources** and never materialises an index.
The motivating use case is consolidating across **rotated logs** — today's
`.log` plus several `.gz` archives from previous days — without first
concatenating to disk and without paying the index-build cost when the
caller only wants the aggregate summary.

### Public surface

```rust
pub struct OrderConsolidated {
    pub root_clordid: Vec<u8>,
    pub family: SmallVec<[Vec<u8>; 2]>,
    pub symbol: Option<Vec<u8>>,
    pub side: Option<u8>,
    pub order_qty: Option<u64>,
    pub cum_qty: u64,
    pub notional: f64,         // Σ LastQty · LastPx over deduped fills
    pub avg_px: f64,           // notional / cum_qty — computed, NOT tag 6
    pub fills: u32,            // post-dedup
    pub final_ord_status: Option<SmallVec<[u8; 2]>>,
    pub first_seen: Option<SystemTime>,
    pub last_seen: Option<SystemTime>,
}

pub struct ConsolidatedBuilder { /* opaque */ }
impl ConsolidatedBuilder {
    pub fn new() -> Self;
    pub fn push_source<R: Read>(&mut self, src: R, format: &LogFormat)
        -> Result<SourceStats, AnalysisError>;
    pub fn finish(self) -> Vec<OrderConsolidated>;
}

pub struct SourceStats {
    pub messages: u64,
    pub fills_seen: u64,
    pub fills_deduped: u64,
}
```

`finish` returns rows sorted by notional descending then by
`root_clordid` for tie-break determinism.

### Algorithm

For each message off the stream:

1. **Family binding.** Resolve the message's `tag 11` (ClOrdID) to a
   family root.
   - If the message carries `tag 41` (OrigClOrdID) **and** that ClOrdID
     was previously seen as a `tag 11` somewhere in the stream:
     `union(new_cid, orig)` — the new ClOrdID inherits orig's root.
   - Otherwise: ensure an acc exists for `new_cid`'s own root. This
     guards against placeholder anchors (`41=NONE`, `41=0`, `41=""`)
     that real broker logs use when the field is mandatory but no
     predecessor exists — without the guard, every order that shared
     the placeholder would coalesce into a single bogus family.
2. **OrderID binding.** When both `tag 11` and `tag 37` are present,
   record `order_id_to_root[37] = root(11)`. Later ERs that carry only
   `tag 37` still resolve to the right family.
3. **Descriptive fields.** First-seen `symbol` (55), `side` (54),
   `order_qty` (38, taken from `D`/`F`/`G` requests — Replace can
   change OrderQty, so the most recent one wins).
4. **Timestamps.** `first_seen` / `last_seen` track tag 52 across all
   messages.
5. **ExecutionReport-specific** (`35=8`):
   - Update `final_ord_status` from tag 39 (last value wins —
     chronological order is preserved by streaming).
   - Maximize `cum_qty` from tag 14.
   - If `ExecType` (tag 150) is in `{F, 1}`: this is a fill.
     `HashSet<ExecID>` per acc dedups; on a new ExecID, add
     `LastQty · LastPx` to `notional` and bump `fills`. Fills without
     ExecID fail open (counted, `tracing::warn!` emitted) so logs from
     misconfigured counterparts don't drop data silently.

`avg_px` is **never read from tag 6**; the implicit `notional / cum_qty`
is the only source of truth so a counterpart that rounds AvgPx in the
ER can't corrupt the summary.

### Streaming I/O

`push_source` buffers 1 MiB chunks (`READ_CHUNK`), parses complete
messages via `parse_one_with_format`, and keeps any trailing partial
across `read()` calls. Watchdog: if the buffer ever exceeds 8 MiB
(`MAX_BUFFER`) with no `8=FIX` boundary recoverable, the first
`MAX_BUFFER/2` bytes are dropped with a `tracing::warn!` — keeps the
parser alive on corrupt gz / binary noise.

A `.gz` archive is consumed by handing `push_source` a
`MultiGzDecoder<File>` from the caller's side; the builder itself only
sees `impl Read`. Sniff handling is the caller's responsibility (CLI
reads the first 64 KiB of the first input, sniffs, then re-emits via
`Cursor::new(prefix).chain(rest)`).

### Union-find on the alias map

`alias: HashMap<Vec<u8>, Vec<u8>>` maps a child ClOrdID to its parent.
`resolve_root` walks parents to fixed point and applies path compression
on the way back, so subsequent lookups stay O(1) amortised.
`union(child, parent)` merges accs and rewires any OrderID bindings
that pointed at the absorbed root.

### Invariants specific to this module

1. **Streaming order is chronological.** Real FIX logs are written in
   send/receive order. The builder relies on this: a fill arriving
   after its NewOrderSingle / Replace must already see the family
   binding established. If a caller feeds out-of-order sources, fills
   that precede their `D`/`G` in the stream become their own root.
   Callers concatenating multiple files should pass them oldest-first.
2. **No memory of "fake" parents.** A `41` value that never appears as
   a `tag 11` in any source is **never** treated as a root. This is
   the placeholder-anchor guard from §Algorithm step 1.
3. **`f64` rounding on huge notional.** Over 142M+ fills the running
   `notional` sum can drift in the last decimal — acceptable because
   the CLI prints two decimals and a `OrderConsolidated` consumer that
   wants exact decimal totals should run on a small subset.

### Tests

Five unit tests in the module:

- `dedup_repeated_execid_counts_once` — resend of the same fill stays at 1.
- `replace_chain_unifies_family` — `D(A) → G(B,41=A) → 8(B, fill)`
  collapses to one row with `family = [A, B]`, `root = A`.
- `notional_is_sum_lastqty_lastpx` — log with disagreeing tag 6 still
  reports `Σ LastQty · LastPx`.
- `fill_without_execid_still_counts` — open-failed dedup; warn but count.
- `multi_source_concatenated_streams` — splitting the same log at a
  message boundary and feeding both halves yields the same output as a
  single push of the concatenated whole.

### Consumers

- `fixlog-cli`: `orders consolidate` subcommand (`commands/orders_consolidate.rs`).
- `fixlog-tui`: `Overlay::Consolidated`, opened via `:consolidated`
  (`command::open_consolidated_overlay`). Runs the builder over
  `Cursor::new(&state.mmap[..])`; blocking, no caching today.

## Histogram (`histogram.rs`)

Single-pass over `index.messages`, parse tag 52, bucket at
`bucket_ns`. Empty input or zero valid timestamps → empty `bins`.

### Sparkline

`render_sparkline(width)` merges bins down to `width` columns by
summing counts, then maps each column to a Unicode block glyph
(`' ▁▂▃▄▅▆▇█'`) using the **p95 count as the "full" level**. Rationale:
a single peak bucket (e.g. a burst of market-open traffic) doesn't
flatten the rest of the chart, which would happen with a raw-max
scale. The outliers still visibly saturate at `█`.

### Peaks

`peaks(k)` returns the top-k bins by count (descending), ties broken by
earlier `start_ns`. Empty bins are filtered out.

### Performance

`histogram_build_1M`: ~500 ms (target <500 ms; on the edge). Biggest
cost is `parse_sending_time` on every message; a faster ASCII-date
parser would push this well under.

## `AnalysisError`

```rust
pub enum AnalysisError {
    Parse(fixlog_core::ParseError),    // delegated via `#[from]`
    MissingTag { tag: u32, ordinal: u32 },
    Io(std::io::Error),                // streaming sources (file, gz, stdin)
}
```

- `Parse` and `MissingTag` remain reserved for future strict-mode
  builders; the index-driven builders (`SessionMap`, `OrderTimeline`,
  `Histogram`) silently skip corrupt messages.
- `Io` is real and live: returned from
  `ConsolidatedBuilder::push_source` when the underlying `Read` errors
  (truncated `.gz`, unexpected EOF mid-chunk, etc.). The CLI surfaces
  it through `anyhow::Context` with the source label.

## Dependencies

| From `Cargo.toml` | Reason                                 |
|-------------------|----------------------------------------|
| `fixlog-core`     | `LogIndex`, `LogFormat`, `parse_one`, dict |
| `memchr`          | `8=FIX` boundary scan in `orders_consolidated::drain_complete` |
| `rayon`           | `Histogram::build` parallel timestamp extraction |
| `smallvec`        | Stack-allocated byte values in `Order*` |
| `thiserror`       | `AnalysisError` derive                 |
| `tracing`         | `warn!` on corrupt-msg skip, fill-without-ExecID, gz watchdog |

**Not depended on**: `serde`, `tokio`, `fixlog-parser`, `fixlog-index`
directly (all via `fixlog-core`).

## Consumed by

- `fixlog-cli`: `sessions` / `orders` (timeline) / `orders consolidate` /
  `histogram` subcommands.
- `fixlog-tui`: sessions overlay, order-lifecycle overlay (`O` key),
  consolidated overlay (`:consolidated`), histogram overlay
  (`:histogram`), bookmarks index, diff view.

## Not re-exported from `fixlog-core`

Intentional — `fixlog-core` stays a domain facade (parser + dict +
index + query + format). Analysis is a composition *on top of* those
primitives and lives in its own tree. Downstream consumers depend on
`fixlog-analysis` directly.
