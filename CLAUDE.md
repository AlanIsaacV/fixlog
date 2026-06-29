# fixlog

> Zero-copy parser and interactive TUI viewer for FIX (Financial Information eXchange) logs, in Rust. Built to chew through millions of FIX messages efficiently, with a local TUI in the spirit of fixparser.targetcompid.com.

## What it is

A Cargo workspace (10 crates) producing one binary, `fixlog`, with a CLI (`sniff`, `parse`, `stats`, `grep`, `sessions`, `orders` [+ `orders consolidate`], `histogram`) and an interactive `tui`. Layout (separator, line prefix, encoding) is auto-detected; dictionaries cover FIX 4.4, FIXT.1.1, FIX 5.0, 5.0 SP1 and 5.0 SP2.

## Run / test / format / lint

```bash
cargo build --all
cargo run -p fixlog-cli -- tui fixtures/synthetic/minimal_4.4.log   # run the TUI
cargo run -p fixlog-cli -- parse fixtures/synthetic/minimal_4.4.log # run the CLI
cargo test --all
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

## Critical rules

- **Never `panic!`/`unwrap()` in production code** (tests may). `unsafe` requires a `// SAFETY:` comment justifying the invariant.
- **All three gates must pass before any commit**: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all`.
- **The sniffer decides the format** — never assume a single separator/version. The parser emits raw tags; the dictionary resolves them. Keep parser and dict decoupled.
- **Never commit `fixtures/real/` or `fixtures/orders/`** — real, gitignored data. Anonymize before adding any fixture.
- **`avg_px` is overloaded**: `OrderConsolidated.avg_px` is *computed* (`notional / cum_qty`); `OrderEvent.avg_px` is the *raw* wire tag 6. Do not conflate them.
- Conventional commits, one logical unit per commit. Never commit without explicit instruction.

## Docs

- `docs/architecture.md` — layout, crate graph, data flow, key libraries, critical gotchas. **Start here.**
- `docs/conventions.md` — code style, errors, logging, testing, commits.
- `docs/runbook.md` — build, install, run, env vars, fixtures, troubleshooting.
- `docs/features/` — capability docs (parsing & sniffing, query/grep/tailing, TUI, analysis & consolidation).
- `docs/agent/` — dense LLM-facing internals, one file per crate. `docs/agent/state.md` is **authoritative** on current phase/status.
- `docs/decisions/`, `docs/postmortems/` — ADRs and incident write-ups (seeded, empty).
