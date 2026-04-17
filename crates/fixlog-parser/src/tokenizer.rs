//! SOH tokenizer. The MVP implementation lands in T04.

use crate::{ParseError, RawMessage};

/// Parse a single FIX message from `buf`, assuming SOH (`\x01`) separators and no line prefix.
///
/// On success returns the parsed [`RawMessage`] and the number of bytes consumed from `buf`
/// (so the caller can advance its cursor). On failure returns a [`ParseError`] describing
/// what went wrong; the caller decides whether to skip the byte range or abort.
pub fn parse_one(buf: &[u8]) -> Result<(RawMessage<'_>, usize), ParseError> {
    let _ = buf;
    todo!("implemented in T04")
}
