# Agent docs — index

LLM-facing documentation for `fixlog`. Each file below is self-contained and can be loaded independently. Read this index first, then pull only the files you need for the current task.

## Load order

1. **Always**: `docs/agent/INDEX.md` (this file) + `CLAUDE.md` (conventions + stack).
2. **Before non-trivial work**: `docs/agent/state.md` — current phase, completed tasks, open items.
3. **Task-specific**: pick from the routing table below.

## Routing table — which file(s) to load for which task

| Task | Files to load |
|------|---------------|
| Touch parser / tokenizer / checksum / BeginString scan | `crates/parser.md`, `reference/fix-protocol.md` |
| Touch sniffer, separator, line-prefix detection | `crates/format.md`, `reference/fix-protocol.md` |
| Add a new FIX version dictionary, regenerate, debug build.rs | `crates/dict.md`, `reference/fix-protocol.md` |
| Change resolver, chain logic, MsgType resolution | `crates/dict.md`, `reference/fix-protocol.md` |
| Touch index builder / append semantics / offset invariants | `crates/index.md`, `crates/parser.md` |
| Touch query DSL grammar / evaluator semantics | `crates/query.md` |
| Add/modify a CLI subcommand | `crates/cli.md`, `crates/core.md` |
| Touch TUI internals (layout, keybindings, event loop, follow watcher) | `crates/tui.md`, `crates/index.md` (for append/consumed), `crates/query.md` (for filter/search) |
| Add a new key binding or input mode | `crates/tui.md` §Input modes / §Keybindings |
| Session tracking / order lifecycle / histogram analysis | `crates/analysis.md`, `crates/index.md` (for secondary lookup) |
| Hot-tag pre-filter / AST pushdown | `crates/query.md` §hot_equalities, `crates/index.md` §SecondaryIndex |
| Work with test fixtures (add new, debug expected counts) | `reference/fixtures.md` |
| Cross-crate change, API boundary, lifetime issue | `patterns.md`, plus the relevant `crates/*.md` |
| Plan next phase / next task | `state.md`, `../ROADMAP.md`, `../PHASE2_PLAN.md` |
| Debug a parse failure on a real log | `crates/parser.md`, `reference/fixtures.md`, `reference/fix-protocol.md` |
| Understand crate deps, re-exports, versioning | `crates/core.md`, `../ARCHITECTURE.md` §Dependencias |

## Files in this directory

- `INDEX.md` — this file. Always loaded.
- `state.md` — current phase + task status snapshot. Reflects reality (may diverge from `PHASE1_PLAN.md` which was the original plan).
- `patterns.md` — cross-cutting code patterns (zero-copy, error handling, test helpers, tracing).
- `crates/parser.md` — `fixlog-parser` internals: tokenizer state machine, BeginString scan, checksum tolerance.
- `crates/format.md` — `fixlog-format` internals: sniffer heuristics, current limitations.
- `crates/dict.md` — `fixlog-dict` + build.rs codegen: generated module layout, `FixVersion`, chains.
- `crates/index.md` — `fixlog-index` internals: offset layout, `consumed` invariant, append semantics, parallel builder.
- `crates/query.md` — `fixlog-query` internals: grammar, precedence, evaluator semantics.
- `crates/cli.md` — `fixlog-cli` commands: sniff / parse / stats / grep / tui. stdin/stdout conventions, tracing.
- `crates/core.md` — `fixlog-core` facade: what is re-exported and why.
- `crates/tui.md` — `fixlog-tui` internals: layout, `AppState`, event loop, input modes, follow watcher, theme, bench numbers.
- `crates/analysis.md` — `fixlog-analysis` internals: session map, order lifecycle, temporal histogram, Gantt/sparkline renderers, `AnalysisError`.
- `reference/fix-protocol.md` — FIX wire format essentials: tags 8/9/10/35/1128, BodyLength semantics, checksum algorithm, FIXT.1.1 vs FIX.4.4 split.
- `reference/fixtures.md` — fixture catalog with expected message counts.

## Related canonical docs (outside agent/)

- `CLAUDE.md` — user preferences, stack, conventions, anti-patterns. **Always loaded** by the system.
- `docs/ARCHITECTURE.md` — original design doc. Some parts are **aspirational** (e.g. TUI data flow, RoaringBitmap in the secondary index). Use `docs/agent/` for current reality; use `ARCHITECTURE.md` for long-term intent.
- `docs/ROADMAP.md` — 5-phase roadmap.
- `docs/PHASE1_PLAN.md` — detailed task list for Phase 1 (closed).
- `docs/PHASE2_PLAN.md` — detailed task list for Phase 2 (current phase; status may be stale; see `state.md` for truth).

## Style guide for these docs

- Dense. Signatures over prose. Code blocks for types.
- One concept per section. Cross-link rather than repeat.
- Call out invariants explicitly with "INVARIANT:".
- Call out divergences from original design with "REALITY:".
- Absolute paths for file references so agents can Read directly.
