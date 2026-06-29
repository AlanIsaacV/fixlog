# fixlog

Zero-copy parser and interactive TUI viewer for FIX (Financial Information eXchange) logs, in
Rust. Built to chew through millions of FIX messages efficiently — zero-copy from the mmapped file
all the way to the rendered output — with a fast local TUI in the spirit of
fixparser.targetcompid.com.

It auto-detects the log layout (separator, line prefix, encoding) instead of assuming one format, so
the same binary handles raw QuickFIX logs, pipe-rendered logs, and logs wrapped in timestamp/logback
prefixes. Dictionaries cover **FIX 4.4, FIXT.1.1, FIX 5.0, 5.0 SP1 and 5.0 SP2**.

```
file ──mmap──► sniff (separator/prefix) ──► parse (zero-copy 8=FIX scan) ──► resolve (tags→names)
                                                          │
                          index (rayon, hot-tag map) ◄────┤
                                  │                        ▼
                          query / grep / --follow     CLI render · TUI · analysis
```

## Install

Requires a recent stable Rust toolchain (tested with 1.95, edition 2024; pinned in
`rust-toolchain.toml`).

```sh
git clone <repo> fixlog && cd fixlog
cargo build --release -p fixlog-cli      # binary → target/release/fixlog
cargo install --path crates/fixlog-cli   # …or install `fixlog` into ~/.cargo/bin
```

## Quick start

Examples assume `alias fixlog=./target/release/fixlog` (or that you ran `cargo install`).

```sh
fixlog sniff fixtures/synthetic/minimal_4.4.log        # what layout is this log?
fixlog parse fixtures/synthetic/minimal_4.4.log --first 5
fixlog tui   fixtures/real/fix44-om.log                # interactive viewer (press : help)
fixlog grep  live.log --filter "35=8 AND 55=AAPL" -F   # tail -f, filtered
fixlog orders consolidate logs/*.log logs/*.log.gz     # aggregate fills across rotated logs
```

## Commands

| Command | What it does | Deep dive |
|---------|--------------|-----------|
| `sniff` / `parse` / `stats` | Detect layout, print resolved messages (pretty/JSON), summarize a file. | [parsing & format](docs/features/parsing-and-format.md) |
| `grep` (`-F`/`--follow`) | Filter with a grep-style DSL (`35=D AND 55=AAPL`, `55~^MS`); tail live files. | [query, grep & tailing](docs/features/query-grep-tailing.md) |
| `tui` | Interactive viewer: virtual list, resolved detail, live filter, search, overlays, export. | [interactive TUI](docs/features/interactive-tui.md) |
| `sessions` / `orders` (`+ consolidate`) / `histogram` | Session gaps, order lifecycle (Gantt), consolidated fills across `.gz`/stdin, temporal histogram. | [analysis & consolidation](docs/features/order-analysis-consolidation.md) |

`fixlog tui` also reads from a pipe (`rg "35=D" *.log | fixlog tui`) and takes `--sort
natural|seq|transact|sending`. Inside the TUI, press `?` or `:help` for the full keybinding
cheatsheet. Filter DSL: `=`, `!=`, `~` (regex), combined with `AND`/`OR`/`NOT` and parens
(precedence `NOT > AND > OR`); tag numbers only. JSON output is JSONL, pipe-friendly to `jq`.

## Supported formats

| Aspect | Support |
|--------|---------|
| FIX versions | 4.4, FIXT.1.1 (session) + 5.0 / 5.0 SP1 / 5.0 SP2 (application) |
| Separator | `SOH` (`\x01`), `\|`, `^`, `;` — auto-detected |
| Line prefix | any (timestamp, logback, PID…) — the parser scans for `8=FIX` |
| Line ending | LF, CRLF |
| Encoding | UTF-8 / ASCII (non-UTF-8 bytes shown lossy) |
| Invalid checksums | **non-fatal** — the message is still emitted, logged at `-vv` |

Adding a FIX version is ~10 lines: drop the QuickFIX XML into `dictionaries/` and register it in
`crates/fixlog-dict/build.rs`.

## Performance (highlights)

Parse ~1 GiB/s on long messages; parallel index build up to ~5× single-thread; TUI frame ~737 µs
on 1M messages (~22× under the 16 ms budget); hot-tag equality filters ~156 µs (~3000× vs full
scan). Full, per-phase benchmark tables live in
[`docs/agent/state.md`](docs/agent/state.md).

## Documentation

- [`docs/architecture.md`](docs/architecture.md) — code layout, crate graph, data flow, key libraries, gotchas.
- [`docs/runbook.md`](docs/runbook.md) — build, install, run, env vars, troubleshooting.
- [`docs/conventions.md`](docs/conventions.md) — code style, errors, testing, commits.
- [`docs/features/`](docs/features/) — per-capability docs (the table above links into these).
- [`docs/agent/`](docs/agent/) — dense, LLM-facing internals per crate; [`state.md`](docs/agent/state.md) is authoritative on current status and backlog.

## Development

```sh
cargo run -p fixlog-cli -- tui <file>                        # run without installing
cargo test --all                                            # full suite
cargo clippy --all-targets --all-features -- -D warnings    # lint (must be clean)
cargo fmt --all
cargo bench -p fixlog-parser --bench parse                  # benches: parser / index / frame
```

Fixtures: `fixtures/synthetic/` is versioned for golden tests; `fixtures/real/` and
`fixtures/orders/` hold real logs and are **gitignored** (never commit FIX data). See
[`fixtures/README.md`](fixtures/README.md).

## License

MIT OR Apache-2.0.
