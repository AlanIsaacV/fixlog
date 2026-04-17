# Project state

> Snapshot of what exists and what's next. Authoritative over `PHASE1_PLAN.md` when they disagree.

## Phase

**Fase 1 — Core Parser (CLI, sin TUI)** — effectively complete except T16 (benchmarks) and T17 (manual e2e). The library and CLI parse/resolve FIX 4.4 and FIXT.1.1/FIX 5.0SP2 logs end to end.

## Crates in the workspace (Cargo.toml members)

- `crates/fixlog-parser` — zero-copy tokenizer.
- `crates/fixlog-format` — format sniffer.
- `crates/fixlog-dict` — multi-version dictionary + resolver.
- `crates/fixlog-core` — facade that re-exports the three above.
- `crates/fixlog-cli` — binary `fixlog` with `sniff` / `parse` / `stats`.

Not yet created (future phases): `fixlog-index`, `fixlog-query`, `fixlog-tui`.

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
| T16 Criterion benchmarks | **pending** | `crates/fixlog-parser/benches/parse.rs` does not exist yet. |
| T17 Manual E2E validation | **partial** | CLI has been smoke-tested against real fixtures; no formal sign-off. |

## Test & quality gates (status)

All green as of last run:

- `cargo test --all` — 35 tests (12 parser unit + 7 format unit + 4 dict unit + 7 dict integration + 3 synthetic + 2 with_format).
- `cargo clippy --all-targets --all-features -- -D warnings` — clean.
- `cargo fmt --all --check` — clean.

## Real-fixture parse metrics (last measured)

| Fixture | Size | Messages parsed | Parse errors |
|---------|------|-----------------|--------------|
| `fixtures/real/fix44-om.log` | 2.1 MB | 5419 | 0 |
| `fixtures/real/fixt11-md.log` | 8.7 MB | 8229 | 0 |

Checksum mismatches are **non-fatal** (see `crates/parser.md`); they are emitted as valid messages and logged at `debug` level.

## What's next, ordered by value

1. **T16 Criterion benchmarks** — establish baseline throughput before Phase 2 indexing work.
2. **Phase 2 — Indexing + tailing** — new crates `fixlog-index` (roaring bitmaps, append-friendly) and `fixlog-query` (filter DSL).
3. **FIX 5.0 / 5.0SP1 dictionaries** — add to `DICTIONARIES` list in `crates/fixlog-dict/build.rs` if a real fixture needs them.
4. **`--strict` flag on parse** — treat checksum mismatches as errors (user-requested feature flag from earlier session).

## Known gaps / decisions deferred

- **Parser ignores `LogFormat.line_prefix`**: the tokenizer scans for `8=FIX` via memmem, so variable-length prefixes "just work". `LinePrefix::Fixed(n)` is kept in the sniffer output for display only.
- **Chain selection is per-message, not per-session**: the resolver reads `BeginString` + `ApplVerID` from each message. For FIXT sessions where `ApplVerID` only appears on Logon, non-Logon messages fall back to the default `CHAIN_FIXT11_FIX50SP2`. A session-aware cache could improve accuracy but is out of scope.
- **Custom tags beyond dictionary range**: show as `?` in pretty output, `"name": null` in JSON.
