# `fixlog-format`

Detects the layout of a FIX log file (separator, line ending, line prefix) from a byte sample.

## Files

- `crates/fixlog-format/src/lib.rs` — types.
- `crates/fixlog-format/src/sniffer.rs` — `sniff()` + heuristics.

## Public API

```rust
pub struct LogFormat {
    pub separator: Separator,
    pub line_prefix: LinePrefix,
    pub encoding: Encoding,           // Utf8 only, for now
    pub line_ending: LineEnding,      // Lf | CrLf
    pub message_boundary: MessageBoundary,  // Line | Checksum — always Line currently
}

pub enum Separator { Soh, Pipe, Caret, Semicolon, Custom(u8) }
pub enum LinePrefix { None, Fixed(usize) }
pub enum LineEnding { Lf, CrLf }

pub fn sniff(sample: &[u8]) -> Result<LogFormat, SniffError>;

pub enum SniffError { EmptySample, NoBeginString, NoSeparator }
```

`Separator::as_byte() -> u8` maps the enum to its wire byte.

## Heuristics

### Line ending
Count `\r\n` occurrences. If >0 → `CrLf`, else `Lf`.

### Separator
For each candidate (`0x01`, `|`, `^`, `;`), count `<sep><digit>+=` fingerprints in the sample. Highest count wins. On a tie, first candidate wins (SOH is first in the list).

`count_field_boundaries` advances one byte at a time to avoid overlapping matches; a `<sep>` followed by no digits is ignored.

### Line prefix
Find `b"8=FIX"` on each sampled line. If all lines agree on the same offset, that's `LinePrefix::Fixed(offset)` (or `None` if offset is 0). Inconsistent offsets → `LinePrefix::None` fallback.

## REALITY: `LinePrefix` is advisory only

The parser does not consume `LinePrefix` — it does its own `memmem`-based scan for `8=FIX` (see `crates/parser.md`). `LinePrefix::Fixed(n)` is still useful for display (the `sniff` CLI command prints it) and as a hint, but it's not in the hot path.

Consequence: variable-length prefixes (logback, `<ts, sess, dir>` wrappers) that return `LinePrefix::None` from the sniffer still parse correctly.

## Typical sample size

The CLI feeds the first 64 KB of the file via `head(&mmap, SNIFF_WINDOW)`. That's enough for hundreds of messages even on long lines.

## Known limitations

- **Custom separator detection**: `Separator::Custom(byte)` is reachable via API but the sniffer only proposes the four standard candidates.
- **Encoding**: fixed to `Utf8`. No BOM handling, no Latin-1 fallback. Non-ASCII bytes in values render via `String::from_utf8_lossy` at presentation time.
- **Message boundary**: always `Line`. The distinction between `Line` and `Checksum` (message ends at `10=xxx<SOH>`) is not currently exercised.

## Tests

`src/sniffer.rs` has unit tests for each heuristic:
- Empty sample → `SniffError::EmptySample`.
- SOH / pipe detection.
- Fixed-timestamp prefix (`LinePrefix::Fixed(24)`).
- CRLF line ending.
- `no_begin_string_fails` (must return `NoBeginString` or `NoSeparator`).
- `count_field_boundaries_ignores_separator_inside_value` — regression guard.

## Do not

- Do not tie sniffer heuristics to specific FIX versions. Detection must work for any `BeginString` that starts with `8=FIX`.
- Do not return an `Err` for ambiguous samples; default to sensible fallbacks (e.g., inconsistent prefixes → `LinePrefix::None`).
