# Runbook

`fixlog` is a **local CLI/TUI binary**, not a network service: there is no deployment, no server, no
listening port. "Operations" here means build, install, run, and troubleshoot against log files.

## Build

```bash
cargo build --all                          # debug build, all crates
cargo build --release -p fixlog-cli        # optimized binary → target/release/fixlog
```

The release profile enables `lto = true` + `codegen-units = 1` for throughput; expect a slower
compile in exchange. Requires the toolchain pinned in `rust-toolchain.toml` (stable, edition 2024,
rust-version 1.95) with `rustfmt` + `clippy` components.

## Install / distribute

```bash
cargo install --path crates/fixlog-cli     # installs `fixlog` into ~/.cargo/bin
# …or copy the standalone binary:
cp target/release/fixlog /usr/local/bin/
```

The binary is self-contained (FIX dictionaries are generated into it at build time from
`dictionaries/*.xml`). No runtime config files or external services are required.

## Run

```bash
fixlog sniff      FILE                                   # detect separator / prefix / encoding
fixlog parse      FILE [--first N] [--format pretty|json]
fixlog stats      FILE                                   # totals, time range, sessions, top MsgTypes
fixlog grep       FILE --filter "EXPR" [--format …] [-F] # grep-style; -F/--follow tails like tail -f
fixlog sessions   FILE [--format pretty|json]
fixlog orders     FILE [--id CLORDID] [--limit N] [--format …]      # order lifecycle / timeline
fixlog orders consolidate INPUTS... [--format pretty|csv|json] [--sort notional|cumqty|fills|recent]
fixlog histogram  FILE [--bucket 1s] [--width 80] [--peaks 5]
fixlog tui        [FILE] [--filter EXPR] [-F] [--sort natural|seq|transact|sending]
```

- `orders consolidate` accepts multiple inputs, transparently decompresses `.gz` archives, and reads
  stdin when an input is `-`. Fills are deduplicated by ExecID so overlap between a live log and its
  rotated `.gz` does not double-count.
- `fixlog tui` reads from **stdin** when `FILE` is omitted and stdin is a pipe
  (e.g. `rg "35=D" *.log | fixlog tui`). `--follow` is rejected with piped input (pipes don't grow).
- Global `-v` / `-vv` raise log verbosity; `grep`/`tui` exit codes follow `grep(1)` (0 = matched).

## Environment variables

| Variable | Description | Required |
|----------|-------------|----------|
| `RUST_LOG` | Overrides the `tracing` level filter (e.g. `RUST_LOG=fixlog_parser=debug`). Without it, `-v`/`-vv` set `info`/`debug`, default `warn`. Logs go to **stderr**. | No |

There are **no** service credentials, secrets, or connection strings — `fixlog` only reads local
files (and stdin).

## Fixtures & test data

- `fixtures/synthetic/` — committed, deterministic golden inputs.
- `fixtures/real/` and `fixtures/orders/` — **gitignored real data**, never committed. Used for
  end-to-end smoke tests (e.g. the ~540 MB orders log: `orders consolidate` reports ~55k orders in
  under 5 s). See `fixtures/README.md`.

## Quality gates (run before committing)

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
```

## Troubleshooting

- **Parse errors / "checksum mismatch" at `debug`.** Expected for truncated or corrupt messages;
  they are non-fatal. Run with `-vv` to see them; the message is still emitted.
- **Wrong separator or fields not resolving.** Run `fixlog sniff FILE` to see what the sniffer
  detected. Variable line prefixes are handled automatically (the parser scans for `8=FIX`).
- **Tags show as `?` (pretty) / `null` (JSON).** Custom/unknown tags outside the dictionary range.
- **FIXT sessions resolving to the wrong FIX version.** Chain selection reads `ApplVerID` per
  message; for non-Logon messages without it, the resolver falls back to `CHAIN_FIXT11_FIX50SP2`.
- **`truncated gz` error from `orders consolidate`.** A `.gz` input ended mid-stream; it surfaces as
  an `io::Error` (not a panic). Re-fetch the archive.
- **Piped TUI input then no keyboard.** Unix only: after draining the pipe, stdin is re-pointed at
  `/dev/tty` so crossterm still gets keystrokes. On Windows, pass a file path instead.

## Monitoring / alerts / on-call

> N/A — local developer tool. No production runtime, dashboards, or alerts.

## External dependencies

> None at runtime. Build-time only: the Rust toolchain and the vendored QuickFIX dictionary XMLs in
> `dictionaries/`.
