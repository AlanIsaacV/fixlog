#![forbid(unsafe_code)]

//! Zero-copy FIX tokenizer and raw message types.
//!
//! This crate is intentionally agnostic to FIX version and dictionaries.
//! It emits [`RawMessage`]s that borrow from the original buffer; resolution
//! of tag names and enum values lives in `fixlog-dict`.

use smallvec::SmallVec;

pub mod tokenizer;
pub mod validator;

pub use tokenizer::{parse_all, parse_all_with_format, parse_one};

/// Tag `8` — BeginString (e.g. `FIX.4.4`, `FIXT.1.1`).
pub const TAG_BEGIN_STRING: u32 = 8;
/// Tag `9` — BodyLength (bytes between tag 9 and tag 10, exclusive).
pub const TAG_BODY_LENGTH: u32 = 9;
/// Tag `10` — CheckSum (sum of bytes up to and including the SOH before tag 10, mod 256).
pub const TAG_CHECKSUM: u32 = 10;
/// Tag `34` — MsgSeqNum.
pub const TAG_MSG_SEQ_NUM: u32 = 34;
/// Tag `35` — MsgType (e.g. `D` = NewOrderSingle, `8` = ExecutionReport).
pub const TAG_MSG_TYPE: u32 = 35;
/// Tag `49` — SenderCompID.
pub const TAG_SENDER_COMP_ID: u32 = 49;
/// Tag `52` — SendingTime.
pub const TAG_SENDING_TIME: u32 = 52;
/// Tag `56` — TargetCompID.
pub const TAG_TARGET_COMP_ID: u32 = 56;

/// A single FIX message, still unresolved against any dictionary.
///
/// `RawMessage` is the fundamental unit produced by the parser. It borrows
/// from the original buffer (`raw`) and exposes its tag/value pairs without
/// copying them. Use `fixlog-dict` to resolve tags into field names and
/// enum values when you need presentation data.
#[derive(Debug, Clone)]
pub struct RawMessage<'a> {
    /// Byte offset of this message within the original source buffer/file.
    pub offset: u64,
    /// Raw bytes of the full message, including `BeginString` through `CheckSum<SOH>`.
    pub raw: &'a [u8],
    /// Tag/value pairs in the order they appear in `raw`. Stack-allocated for the
    /// common case of ≤32 tags per message.
    pub tags: SmallVec<[(u32, &'a [u8]); 32]>,
}

/// Errors produced by the tokenizer for a single message.
///
/// The parser does not propagate these as fatal errors at the stream level;
/// corrupted individual messages are logged via `tracing` and skipped. These
/// variants exist so callers that want per-message diagnostics can pattern
/// match on the cause.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// Buffer ended before a complete message could be read.
    #[error("unexpected end of input while parsing message")]
    UnexpectedEof,
    /// A tag component was not a valid ASCII unsigned integer.
    #[error("invalid tag number at offset {offset}")]
    InvalidTag { offset: u64 },
    /// Found a `tag=` without a value segment before the next separator.
    #[error("tag {tag} has no value at offset {offset}")]
    MissingValue { tag: u32, offset: u64 },
    /// First field was not `8=...` or BeginString value was not recognized.
    #[error("invalid BeginString at offset {offset}")]
    InvalidBeginString { offset: u64 },
    /// Tag `9` (BodyLength) missing, non-numeric, or inconsistent with message size.
    #[error("invalid BodyLength at offset {offset}")]
    InvalidBodyLength { offset: u64 },
    /// Tag `10` (CheckSum) did not match the computed value.
    #[error("checksum mismatch at offset {offset}: expected {expected}, got {got}")]
    BadChecksum { offset: u64, expected: u8, got: u8 },
}
