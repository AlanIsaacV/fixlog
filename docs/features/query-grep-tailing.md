# Query, grep & live tailing

## What it does

Filters FIX messages with a small grep-style DSL and, optionally, streams matches live as the file
grows (`tail -f` style). Equality filters over indexed "hot" tags are resolved via the secondary
index instead of a full scan, giving a large speedup on common queries.

## DSL grammar (summary)

- Comparators: `=` (equal), `!=` (not equal), `~` (regex match).
- Boolean: `AND`, `OR`, `NOT`, parentheses; keywords case-insensitive.
- Tag-number only: `35=D`, `35=8 AND 55=AAPL`, `55~^MS`, `NOT (35=0)`.
- `N!=X` is true iff *no* occurrence of tag N equals X (stricter than "some instance differs" for
  repeating groups — see `crates/fixlog-query/src/eval.rs`).

## Surface (CLI)

| Command | Notes |
|---------|-------|
| `fixlog grep FILE --filter "EXPR" [--format pretty\|json]` | grep-style filter; `grep(1)` exit codes (0 = matched, 1 = none). |
| `fixlog grep FILE --filter "EXPR" -F/--follow` | Tails the file via `notify` + re-mmap; detects rotation/truncation. |

The same DSL drives the TUI live filter, search (`/`, `n`/`N`), and filter-from-detail (`f`/`x`).

## Files involved

- `crates/fixlog-query/` — AST, parser, evaluator, `Expr::hot_equalities()`. See
  `docs/agent/crates/query.md`.
- `crates/fixlog-index/` — secondary hot-tag map (`(tag, value) → [ordinals]`), append-only growth.
  See `docs/agent/crates/index.md`.
- `crates/fixlog-cli/src/commands/grep.rs` — grep command + `--follow` loop (`notify`).

## Data flow

Parse the filter string into a `QueryExpr` (regexes compiled once, `Arc`-shared) → if the expression
is a pure AND of equalities over hot tags, intersect the sorted ordinal lists from the secondary
index (fast path); otherwise evaluate the AST against every `RawMessage` (zero-alloc, short-circuit)
→ render matches. With `--follow`, a `notify` watcher re-mmaps on growth and re-runs the filter over
the appended range.

## Performance

- **Hot-tag pre-filter**: a pure-AND-of-equalities query over indexed tags
  (default hot set `35,49,56,11,34,37,41`) drops `tui_filter/apply_35eqD_1M` from ~477 ms to
  ~156 µs (~3000×) on 1M messages. Non-hot or boolean-OR queries fall back to full scan.

## Edge cases

- **Partial / malformed expression** in the TUI live preview freezes the previous result rather than
  clearing the view.
- **`--follow` rotation/truncation**: a shrinking or replaced file triggers a full re-bootstrap.
- **No symbolic names**: `MsgType=NewOrderSingle` is not supported — use `35=D`. The query crate is
  intentionally dict-agnostic.
