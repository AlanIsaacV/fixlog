# `fixlog-parser`

Zero-copy FIX tokenizer. Emits `RawMessage<'a>` that borrow from the source buffer.

## Files

- `crates/fixlog-parser/src/lib.rs` — public types, tag constants, `ParseError`.
- `crates/fixlog-parser/src/tokenizer.rs` — state machine + iterator.
- `crates/fixlog-parser/src/validator.rs` — `compute_checksum`, `parse_checksum`.
- `crates/fixlog-parser/tests/synthetic.rs` — fixtures/synthetic golden tests.
- `crates/fixlog-parser/tests/with_format.rs` — sniff+parse pipeline tests.

## Public API

```rust
pub struct RawMessage<'a> {
    pub offset: u64,                             // byte offset in the source buffer
    pub raw: &'a [u8],                           // full message bytes, 8=…10=xxx\x01
    pub tags: SmallVec<[(u32, &'a [u8]); 32]>,   // tag/value pairs in source order
}

pub fn parse_one(buf: &[u8]) -> Result<(RawMessage<'_>, usize), ParseError>;
pub fn parse_all(buf: &[u8]) -> Parser<'_>;
pub fn parse_all_with_format<'a>(buf: &'a [u8], fmt: &LogFormat) -> Parser<'a>;

pub const TAG_BEGIN_STRING: u32 = 8;
pub const TAG_BODY_LENGTH:  u32 = 9;
pub const TAG_CHECKSUM:     u32 = 10;
pub const TAG_MSG_SEQ_NUM:  u32 = 34;
pub const TAG_MSG_TYPE:     u32 = 35;
pub const TAG_SENDER_COMP_ID: u32 = 49;
pub const TAG_SENDING_TIME: u32 = 52;
pub const TAG_TARGET_COMP_ID: u32 = 56;
```

`Parser<'a>` implements `Iterator<Item = Result<RawMessage<'a>, ParseError>>`.

## Algorithm

1. **Find next message**: `memchr::memmem::find(buf[cursor..], b"8=FIX")` locates the start. Everything before that offset is noise (log prefix, previous bad message bytes) and is skipped. This makes the parser **prefix-agnostic**: it works for timestamp prefixes, logback prefixes, or no prefix, without any configuration.
2. **Parse BeginString** (`8=<value><SOH>`): the value runs until the separator. No specific format is required beyond ASCII.
3. **Parse BodyLength** (`9=<digits><SOH>`): must be valid ASCII unsigned integer. `body_start = cursor`, `body_end = body_start + body_length`.
4. **Tokenize the body**: starting at `body_start`, repeatedly read `<tag>=<value><SOH>` pairs until `cursor >= body_end`. Tag 35 (MsgType) is typically first; other tags follow in any order.
5. **Parse CheckSum** (`10=<3 digits><SOH>`): must appear immediately after the body.
6. **Validate checksum** (informational only — see below).

All tag/value slices in `RawMessage.tags` reference the original buffer; no copying.

## INVARIANT: prefix-agnostic scan

The parser **ignores `LogFormat.line_prefix`**. `parse_all_with_format` only uses `LogFormat.separator`. Timestamp prefixes, logback prefixes, PID strings — anything before `8=FIX` is skipped by `find_begin_string`. Do not re-introduce prefix-length handling in the parser.

## INVARIANT: checksum mismatch is non-fatal

Tag 10 is **validated but not enforced**. If the computed checksum differs from the declared one, the parser:

1. Logs at `tracing::debug!` level.
2. Emits the message anyway as `Ok(RawMessage)`.

Rationale: real logs are often re-rendered (SOH → `|`, added prefixes, trailing whitespace) and their checksums no longer match the rendered bytes. Refusing to emit would lose 100% of such logs (see `fixtures/real/fixt11-md.log` — 8229 messages, all checksum-mismatched). A `--strict` CLI flag can re-introduce fatal behavior later.

`ParseError::BadChecksum` remains in the enum for callers who want to validate explicitly via `validator::compute_checksum` but is not produced by `parse_one`/`parse_all`.

## Structural errors (returned as `Err`)

These are fatal per-message and cause the iterator to advance 1 byte and resume scanning:

- `UnexpectedEof` — declared `BodyLength` walks past the buffer.
- `InvalidTag { offset }` — tag number isn't a valid `u32`.
- `MissingValue { tag, offset }` — `tag=` followed immediately by separator.
- `InvalidBeginString { offset }` — marker found but first field isn't `8=…`.
- `InvalidBodyLength { offset }` — tag 9 missing or non-numeric.

`BadChecksum` is defined but not emitted (see invariant above).

## Separator handling

The separator byte comes from `LogFormat.separator` (via `parse_all_with_format`) or defaults to `0x01` (SOH) for `parse_all`/`parse_one`. Common alternatives: `|`, `^`, `;`. See `reference/fix-protocol.md` for wire-format details.

## Performance notes

- `SmallVec<[_; 32]>` keeps tag arrays on the stack for messages with ≤32 tags. Hot path has zero allocations.
- `memchr::memmem::find` for the BeginString scan is SIMD-accelerated.
- `memchr::memchr(sep, ...)` is used inside the tokenizer loop for finding field boundaries.

## Tests

Unit tests live in `tokenizer.rs` and `validator.rs` under `#[cfg(test)] mod tests`. Integration:

- `tests/synthetic.rs::minimal_4_4_parses_all_ten_messages` — 10 messages, 7 MsgTypes.
- `tests/synthetic.rs::malformed_emits_valid_messages_and_logs_errors` — asserts 5 emitted / 1 structural error.
- `tests/with_format.rs` — pipe separator + timestamp prefix fixtures.

## Do not

- Do not reintroduce prefix-length handling in the parser (see invariant).
- Do not make checksum mismatch fatal in the default path (see invariant). Add a `strict: bool` parameter or a new entry point if needed.
- Do not depend on `fixlog-dict` from here. Parser is version-agnostic.
- Do not allocate in the hot loop. Reviews block on this.
