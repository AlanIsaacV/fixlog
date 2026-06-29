# Decisions (ADRs)

Architecture Decision Records — one decision per file (`NNNN-short-title.md`), max ~1 page each.
None recorded yet.

| ADR | Title | Status | Date |
|-----|-------|--------|------|
| _none yet_ | | | |

> Several design decisions are currently captured inline in `docs/agent/state.md`
> (§"Known gaps / decisions deferred") rather than as ADRs — e.g. reserving `/` for search vs. live
> filter, extracting the `fixlog-render` crate, `Arc`-wrapping `QueryExpr` for cheap clones, keeping
> `fixlog-analysis` out of the `fixlog-core` re-export, and the hot-tag AST pushdown. Promote any of
> these to a real ADR here when it needs a durable, standalone record.
