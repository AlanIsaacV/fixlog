# `fixlog-cli`

Binary `fixlog` with subcommands `sniff`, `parse`, `stats`. Depends only on `fixlog-core` at the workspace-internal boundary.

## Files

- `crates/fixlog-cli/Cargo.toml` — deps: `fixlog-core`, `anyhow`, `clap` (derive), `memmap2`, `tracing`, `tracing-subscriber` (env-filter).
- `crates/fixlog-cli/src/main.rs` — `Cli` struct (clap derive), tracing init.
- `crates/fixlog-cli/src/io.rs` — `mmap_file` + `head` helpers.
- `crates/fixlog-cli/src/commands/sniff.rs`
- `crates/fixlog-cli/src/commands/parse.rs`
- `crates/fixlog-cli/src/commands/stats.rs`
- `crates/fixlog-cli/src/commands/mod.rs` — re-exports the three command modules.

## Command surface

```
fixlog sniff <file>
fixlog parse <file> [--first N] [--format pretty|json]
fixlog stats <file>
fixlog -v … / -vv …   # tracing level: warn/info/debug
```

`ParseFormat` is a `ValueEnum` (`pretty` default, `json` for JSONL).

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
