# `fixlog-cli`

Binary `fixlog` with subcommands `sniff`, `parse`, `stats`. Depends only on `fixlog-core` at the workspace-internal boundary.

## Files

- `crates/fixlog-cli/Cargo.toml` — deps: `fixlog-core`, `fixlog-analysis`, `fixlog-tui`, `anyhow`, `clap` (derive), `flate2`, `memmap2`, `notify`, `tempfile`, `tracing`, `tracing-subscriber` (env-filter), `libc` (unix only).
- `crates/fixlog-cli/src/main.rs` — `Cli` struct (clap derive), tracing init, `OrdersArgs` + `OrdersSub` subcommand wiring.
- `crates/fixlog-cli/src/io.rs` — `mmap_file` + `head`, `InputSource::{File, Stdin}`, `open_source` (transparent `.gz` via `MultiGzDecoder`, stdin via `BufReader<Stdin>`).
- `crates/fixlog-cli/src/commands/sniff.rs`
- `crates/fixlog-cli/src/commands/parse.rs`
- `crates/fixlog-cli/src/commands/stats.rs`
- `crates/fixlog-cli/src/commands/orders.rs` — timeline mode (legacy default).
- `crates/fixlog-cli/src/commands/orders_consolidate.rs` — consolidated summary across multiple inputs.
- `crates/fixlog-cli/src/commands/mod.rs` — re-exports the command modules.

## Command surface

```
fixlog sniff <file>
fixlog parse <file> [--first N] [--format pretty|json]
fixlog stats <file>
fixlog grep <file> --filter "<expr>" [--format ...] [--follow|-F]
fixlog tui [<file>] [--filter "<expr>"] [--follow|-F] [--sort natural|seq|transact|sending]
fixlog sessions <file> [--format pretty|json]
fixlog orders <file> [--id CLORDID] [--limit N] [--format pretty|json]
fixlog orders consolidate <inputs...> [--format pretty|csv|json] [--sort notional|cumqty|fills|recent]
fixlog histogram <file> [--bucket DUR] [--width N] [--peaks K]
fixlog -v … / -vv …   # tracing level: warn/info/debug
```

`ParseFormat` is a `ValueEnum` (`pretty` default, `json` for JSONL).
Fase 4 subcommands (`sessions`, `orders`, `histogram`) depend on
`fixlog-analysis`; they build a parallel index once and reuse the same
byte buffer for the analysis pass. `sessions` and `orders` (no-id list
mode) exit with code 1 when the result set is empty, matching grep(1)
conventions.

`orders consolidate` is the only subcommand that **doesn't** mmap/index.
It streams each input through `fixlog_analysis::ConsolidatedBuilder` via
`io::open_source` so `.gz` archives and rotated logs work without
pre-decompressing to disk. See §"`orders consolidate`" below.

## Shared conventions

- **File access**: `io::mmap_file(&Path) -> Result<Mmap>` via `memmap2`. Single `unsafe` block, documented with a `// SAFETY:` comment. Do not move the mmap into anything `'static`.
- **Sniff window**: commands feed `head(&mmap, 64 * 1024)` to the sniffer.
- **Stdout**: buffer with `BufWriter::new(stdout.lock())` and flush at the end. One message per entity; never mix diagnostic output with payload.
- **Stderr**: `tracing` writes here. `init_tracing` uses `EnvFilter::try_from_default_env()`, falling back to `warn`/`info`/`debug` depending on `-v` count.
- **Errors**: use `anyhow::Result` + `.with_context(|| format!("sniffing {}", path.display()))`. Let `main` return the error; clap/anyhow handle formatting.

## `sniff` output

```
File:           <path>
Separator:      SOH (0x01) | | (0x7C) | ^ (0x5E) | ; (0x3B) | custom
Line prefix:    none | fixed N bytes
Line ending:    LF (\n) | CRLF (\r\n)
Encoding:       UTF-8 / ASCII
Msg boundary:   line | checksum
```

## `parse` output

Pretty example:

```
Message @ offset 60 (300 bytes) NewOrderSingle
      8  BeginString      = FIX.4.4
     35  MsgType          = D (NewOrderSingle)
     54  Side             = 1 (BUY)
     40  OrdType          = 2 (LIMIT)
     ...
```

Rules:
- Header line carries offset, raw byte count, and (if resolved) MsgType name.
- Column widths: tag right-aligned in 5 chars; name left-aligned to the longest known name in the message.
- Enum labels appear in `(PARENS)` after the value. Missing name → `?`. Missing label → omitted.

JSONL example (one object per line):

```json
{"offset":0,"raw_len":95,"msg_type_name":"Logon","tags":[{"tag":8,"name":"BeginString","value":"FIX.4.4"},{"tag":35,"name":"MsgType","value":"A","value_label":"Logon"},…]}
```

- `msg_type_name` is omitted when the message has no MsgType or it's unknown.
- `name` is `null` for unknown tags.
- `value_label` is omitted (not `null`) when the field is not an enum or the label is unknown.
- Values are JSON-escaped via `write_json_string` (quotes, backslashes, control chars, lossy UTF-8).

## `stats` output

```
File:            <path>
Messages parsed: N
Parse errors:    N
Time range:      <min sendingTime> .. <max sendingTime>   (if any)
Sessions:        N
  <count>  <sender> → <target>
  ...
Message types:   <total> (top 10 shown)
  <count>  <wire>  <label>
  ...
```

`SendingTime` (tag 52) is compared lexicographically — the `YYYYMMDD-HH:MM:SS[.sss]` format sorts identically to real time order. Session tuples are `(SenderCompID, TargetCompID)`. Top-10 MsgTypes sorted by count desc, then by wire value.

The chain used to resolve MsgType labels comes from `chain_for(last_begin_string, last_appl_ver_id)` — the session's last-seen values. This works because real logs typically carry one session per file.

## `orders consolidate`

Multi-input consolidated summary across plain logs, `.gz` archives, and
stdin. Backward-compatible with `fixlog orders <file> [--id …]` —
clap routes the nested subcommand via
`Orders(OrdersArgs)` + `OrdersSub::Consolidate { … }` with
`#[command(args_conflicts_with_subcommands = true)]`, so a bare
`fixlog orders FILE` still hits timeline mode.

```
fixlog orders consolidate today.log today.1.log.gz today.2.log.gz
fixlog orders consolidate today.log --format csv > orders.csv
fixlog orders consolidate today.log --format json | jq '.[0]'
zcat today.log.2.gz today.log.1.gz today.log | fixlog orders consolidate -
fixlog orders consolidate today.log --sort fills
```

Flags:

- `--format pretty|csv|json` (default `pretty`). Pretty: aligned table
  with comma thousand-separators. CSV: header
  `root_clordid,family,side,symbol,order_qty,cum_qty,notional,avg_px,fills,final_ord_status`,
  family joined with `|`. JSON: a top-level array (so `jq '.[0]'`
  works) with one object per order; `notional` and `avg_px` use 4
  decimal places.
- `--sort notional|cumqty|fills|recent` (default `notional`). All
  descending with `root_clordid` as the deterministic tie-break.

### Inputs and sniff

`InputSource::from_arg(token)` maps `-` to `Stdin` and anything else to
`File(PathBuf)`. `io::open_source` returns a `Box<dyn BufRead>`:
`MultiGzDecoder<File>` when the path ends in `.gz` (case-insensitive),
`BufReader<File>` otherwise, `BufReader<Stdin>` for `-`. `MultiGzDecoder`
handles concatenated gzip members so `cat a.gz b.gz > combined.gz`
works.

The runner reads the first 64 KiB of the first input (descomprimido si
`.gz`), runs `fixlog_format::sniff` over those bytes, and re-emits them
ahead of the rest via `Cursor::new(prefix).chain(reader)` so the
builder doesn't lose the head. Remaining inputs are assumed to share
the same format — pipe-vs-SOH-vs-prefix shifts mid-stream are not
detected today.

### Errors and exit codes

- Missing inputs: clap rejects (no `<inputs>`).
- Empty input: `anyhow!("<src> is empty")`.
- Sniff failure: `anyhow!("could not infer log format from <src>")`.
- Truncated `.gz`: `flate2` surfaces an `io::Error` that bubbles
  through `ConsolidatedBuilder::push_source` and is reported with the
  source label via `.with_context`.
- Empty result set (no orders parsed): exit 1, like `grep`.

### Tests

- `crates/fixlog-cli/src/io.rs` — 3 unit tests: plain/gz equivalence,
  truncated gz surfaces `io::Error` (no panic), `from_arg("-")` parses
  as Stdin.
- `crates/fixlog-cli/tests/orders_consolidate.rs` — 6 integration
  tests against the real orders fixture:
  pretty/csv/json shape, no-inputs clap error, backward-compat
  (`fixlog orders FILE --limit N` keeps working), and the canonical
  multi-input check: split the fixture at a `8=FIX` boundary near the
  midpoint, gzip the tail, feed `<head_plain> <tail.gz>` to
  consolidate, and assert the CSV is byte-identical to the
  single-file run.

## Importing from `fixlog-core`

CLI code should use `fixlog_core::` for re-exports, not the underlying crates directly:

```rust
use fixlog_core::{ResolvedMessage, parse_all_with_format, resolve};        // top-level re-exports
use fixlog_core::dict::{chain_for, chain_msg_type_label};                  // dict sub-module
use fixlog_core::parser::{TAG_MSG_TYPE, TAG_SENDER_COMP_ID /*, …*/};       // parser sub-module
use fixlog_core::format::{Separator, LinePrefix, …};                       // format sub-module
```

Do not add direct dependencies on `fixlog-parser`/`fixlog-format`/`fixlog-dict` to `fixlog-cli/Cargo.toml`. That's what the facade is for.

## Adding a new subcommand

1. New file `crates/fixlog-cli/src/commands/<name>.rs` with `pub fn run(…) -> Result<()>`.
2. Register in `commands/mod.rs`.
3. Add a variant to `Command` in `main.rs` and route in the `match cli.command`.
4. Follow the BufWriter stdout + tracing stderr patterns.

## Do not

- Do not read the whole file into a `Vec<u8>` — use `mmap_file`.
- Do not `println!` from library code; that's for `main`/command handlers only.
- Do not `unwrap()` / `expect()` on user-reachable paths. Use `anyhow::Context`.
- Do not bypass `tracing-subscriber`; respect the `-v` flag and `RUST_LOG`.
