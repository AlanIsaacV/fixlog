# Parsing & format sniffing

## What it does

Turns an arbitrary FIX log file into parsed messages without assuming a single layout. A format
sniffer detects the separator (`SOH`, `|`, `^`, `;`), line prefix, and line ending; a zero-copy
tokenizer then scans the bytes for `8=FIX` boundaries and emits `RawMessage`s of `(tag, &[u8])`
slices. Dictionaries resolve tags → field names and enum values → labels across multiple FIX
versions, selecting a chain automatically from `BeginString` + `ApplVerID`.

## Surface (CLI)

| Command | What it shows |
|---------|---------------|
| `fixlog sniff FILE` | Detected separator, line prefix, line ending, encoding, message boundary. |
| `fixlog parse FILE [--first N] [--format pretty\|json]` | Parsed + resolved messages (pretty table or JSONL). |
| `fixlog stats FILE` | Totals, parse errors, time range, sessions, top-10 MsgTypes. |

Resolution also powers the TUI detail panel, `grep`/`tui` rendering, and all analysis commands.

## Files involved

- `crates/fixlog-format/` — sniffer (`sniff`, `LogFormat`). See `docs/agent/crates/format.md`.
- `crates/fixlog-parser/` — tokenizer (`RawMessage`, `parse_one_with_format`,
  `parse_all_with_format`), checksum/BodyLength tolerance. See `docs/agent/crates/parser.md`.
- `crates/fixlog-dict/` — generated field/enum tables (`build.rs` ← `dictionaries/*.xml`), resolver,
  `FixVersion`, chains. See `docs/agent/crates/dict.md`.
- `crates/fixlog-render/` — pretty / JSONL output. See `docs/agent/crates/cli.md`.
- `crates/fixlog-cli/src/commands/{sniff,parse,stats}.rs` — command wiring.

## Data flow

`mmap` the file → `format::sniff` on the head → parser scans for `8=FIX` and tokenizes each message
(zero-copy slices into the mmap) → on demand, `dict` resolves tags/values using the chain chosen from
`BeginString`/`ApplVerID` → `render` writes pretty or JSONL.

## Supported versions

FIX 4.4, FIXT.1.1, FIX 5.0, FIX 5.0 SP1, FIX 5.0 SP2 (XMLs vendored from QuickFIX in
`dictionaries/`). SP2 is the default routing target when `ApplVerID` is unknown/absent.

## Edge cases

- **Checksum / BodyLength mismatches are non-fatal** — the message is emitted and logged at `debug`.
- **Variable-length line prefixes** (timestamps, logback) are handled because the parser scans for
  `8=FIX` itself; `LogFormat.line_prefix` is informational only.
- **Empty/blank lines and truncated trailing messages** are tolerated, not fatal.
- **Custom tags** outside the dictionary range render as `?` (pretty) / `null` (JSON).
- **Repeating groups**: only `268 NoMDEntries` is currently registered for indented rendering
  (covers Market Data `W`/`X`); other groups render flat. See `docs/agent/state.md`.
