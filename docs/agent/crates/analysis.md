# fixlog-analysis

Pure library for semantic analysis over an already-indexed FIX log.
Consumers (CLI, TUI) pass in `&LogIndex`, `&[u8]`, `&LogFormat`;
builders return owned structures that survive an `Arc<Mmap>` swap under
`--follow`.

Three modules, one `AnalysisError` enum, no I/O, no async.

## Module layout

| Module       | Types                                    | Purpose                              |
|--------------|------------------------------------------|--------------------------------------|
| `sessions`   | `SessionKey`, `SessionStats`, `SeqGap`, `SessionMap` | Aggregate by `(49, 56)` canonical pair; detect `MsgSeqNum` gaps. |
| `orders`     | `OrderEvent`, `OrderTimeline`, `render_gantt` | Reconstruct a ClOrdID chain across `11`/`37` crossrefs; Gantt renderer. |
| `histogram`  | `Bin`, `Histogram`                       | Uniform-width temporal histogram over tag 52; sparkline + peaks. |
| `util`       | `find_tag`, `parse_u32_ascii`, `parse_sending_time` | Internal helpers; `pub` for benches and future modules. |

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
    Parse(fixlog_core::ParseError),  // delegated via `#[from]`
    MissingTag { tag: u32, ordinal: u32 },
}
```

Currently unused by the public builders (they silently skip corrupt
messages; no result is ever `Err(AnalysisError)`). Reserved for future
strict-mode builders where a missing required tag should abort rather
than skip.

## Dependencies

| From `Cargo.toml` | Reason                                 |
|-------------------|----------------------------------------|
| `fixlog-core`     | `LogIndex`, `LogFormat`, `parse_one`, dict |
| `smallvec`        | Stack-allocated byte values in `Order*` |
| `thiserror`       | `AnalysisError` derive                 |
| `tracing`         | Reserved for future corrupt-msg warnings |

**Not depended on**: `serde`, `tokio`, `fixlog-parser`, `fixlog-index`
directly (all via `fixlog-core`).

## Consumed by

- `fixlog-cli`: `sessions` / `orders` / `histogram` subcommands.
- `fixlog-tui`: sessions overlay, order-lifecycle overlay (`O` key),
  histogram overlay (`:histogram`), bookmarks index, diff view.

## Not re-exported from `fixlog-core`

Intentional — `fixlog-core` stays a domain facade (parser + dict +
index + query + format). Analysis is a composition *on top of* those
primitives and lives in its own tree. Downstream consumers depend on
`fixlog-analysis` directly.
