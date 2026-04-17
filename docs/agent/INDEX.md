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
| Add/modify a CLI subcommand | `crates/cli.md`, `crates/core.md` |
| Work with test fixtures (add new, debug expected counts) | `reference/fixtures.md` |
| Cross-crate change, API boundary, lifetime issue | `patterns.md`, plus the relevant `crates/*.md` |
| Plan next phase / next task | `state.md`, `../ROADMAP.md`, `../PHASE1_PLAN.md` |
| Debug a parse failure on a real log | `crates/parser.md`, `reference/fixtures.md`, `reference/fix-protocol.md` |
| Understand crate deps, re-exports, versioning | `crates/core.md`, `../ARCHITECTURE.md` §Dependencias |

## Files in this directory

- `INDEX.md` — this file. Always loaded.
- `state.md` — current phase + task status snapshot. Reflects reality (may diverge from `PHASE1_PLAN.md` which was the original plan).
- `patterns.md` — cross-cutting code patterns (zero-copy, error handling, test helpers, tracing).
- `crates/parser.md` — `fixlog-parser` internals: tokenizer state machine, BeginString scan, checksum tolerance.
- `crates/format.md` — `fixlog-format` internals: sniffer heuristics, current limitations.
- `crates/dict.md` — `fixlog-dict` + build.rs codegen: generated module layout, `FixVersion`, chains.
- `crates/cli.md` — `fixlog-cli` commands: sniff / parse / stats. stdin/stdout conventions, tracing.
- `crates/core.md` — `fixlog-core` facade: what is re-exported and why.
- `reference/fix-protocol.md` — FIX wire format essentials: tags 8/9/10/35/1128, BodyLength semantics, checksum algorithm, FIXT.1.1 vs FIX.4.4 split.
- `reference/fixtures.md` — fixture catalog with expected message counts.

## Related canonical docs (outside agent/)

- `CLAUDE.md` — user preferences, stack, conventions, anti-patterns. **Always loaded** by the system.
- `docs/ARCHITECTURE.md` — original design doc. Some parts are **aspirational** (e.g. `LogIndex`, `fixlog-index`, `fixlog-query`, TUI data flow). Use `docs/agent/` for current reality; use `ARCHITECTURE.md` for long-term intent.
- `docs/ROADMAP.md` — 5-phase roadmap.
- `docs/PHASE1_PLAN.md` — detailed task list for Phase 1 (status may be stale; see `state.md` for truth).

## Style guide for these docs

- Dense. Signatures over prose. Code blocks for types.
- One concept per section. Cross-link rather than repeat.
- Call out invariants explicitly with "INVARIANT:".
- Call out divergences from original design with "REALITY:".
- Absolute paths for file references so agents can Read directly.
