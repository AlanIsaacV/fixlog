//! Zero-copy SOH tokenizer.
//!
//! Public surface:
//! - [`parse_one`]: parse a single FIX message from a buffer that starts at `8=`.
//! - [`parse_all`]: iterator over all messages in a buffer, skipping malformed ones with `tracing::warn!`.

use crate::{ParseError, RawMessage, TAG_BEGIN_STRING, TAG_BODY_LENGTH, TAG_CHECKSUM, validator};
use fixlog_format::LogFormat;
use smallvec::SmallVec;

/// Standard FIX field separator.
pub const SOH: u8 = 0x01;

/// Magic marker that identifies the start of a FIX message regardless of what surrounds it.
/// This is tag 8 (BeginString) followed by the mandatory `FIX` or `FIXT` version prefix.
const BEGIN_STRING_MARKER: &[u8] = b"8=FIX";

/// Parse a single FIX message from `buf`, assuming SOH (`\x01`) separators and no line prefix.
///
/// On success returns the parsed [`RawMessage`] (with `offset = 0`) and the number of bytes
/// consumed from `buf` so the caller can advance its cursor.
pub fn parse_one(buf: &[u8]) -> Result<(RawMessage<'_>, usize), ParseError> {
    parse_one_inner(buf, 0, SOH)
}

/// Parse a single FIX message from `buf` using the separator declared by `format`.
///
/// Equivalent to [`parse_one`] for SOH-separated logs, but also correct for pipe-
/// (`|`), caret-, semicolon-, or custom-separated logs produced by human-readable
/// loggers (QuickFIX-J `|`, some legacy C++ wrappers, etc.).
///
/// Only `format.separator` is consulted — the line prefix is ignored because the
/// caller is expected to pass a slice that already starts at `8=`.
pub fn parse_one_with_format<'a>(
    buf: &'a [u8],
    format: &LogFormat,
) -> Result<(RawMessage<'a>, usize), ParseError> {
    parse_one_inner(buf, 0, format.separator.as_byte())
}

/// Iterate over all messages in `buf`, assuming SOH separators.
///
/// The iterator is prefix-agnostic: it scans forward for the `8=FIX` BeginString marker and
/// ignores anything between messages, so application log noise, timestamp prefixes and
/// direction markers are all transparently skipped.
pub fn parse_all(buf: &[u8]) -> Parser<'_> {
    Parser {
        buf,
        cursor: 0,
        sep: SOH,
    }
}

/// Iterate over all messages in `buf` using the layout described by `format`.
///
/// Only `format.separator` is used here; the line prefix is ignored because the parser
/// locates messages via the `8=FIX` marker rather than by stripping bytes from each line.
/// This makes the parser tolerant of logs with variable-length prefixes (Java logback,
/// QuickFIX-C++ wrappers, direction markers that change per session, …).
pub fn parse_all_with_format<'a>(buf: &'a [u8], format: &LogFormat) -> Parser<'a> {
    Parser {
        buf,
        cursor: 0,
        sep: format.separator.as_byte(),
    }
}

/// Iterator returned by [`parse_all`] and [`parse_all_with_format`]. Public because
/// `impl Trait` in return position would hide the type from documentation; users
/// typically don't name it explicitly.
pub struct Parser<'a> {
    buf: &'a [u8],
    cursor: usize,
    sep: u8,
}

impl<'a> Iterator for Parser<'a> {
    type Item = Result<RawMessage<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        let start = find_begin_string(self.buf, self.cursor)?;
        let offset = start as u64;
        match parse_one_inner(&self.buf[start..], offset, self.sep) {
            Ok((msg, consumed)) => {
                self.cursor = start + consumed;
                Some(Ok(msg))
            }
            Err(err) => {
                tracing::warn!(offset = start, error = %err, "skipping malformed message");
                // Skip past the current marker so the next scan can find the *next* message.
                // Advancing by one byte handles the case where a false `8=FIX` match is embedded
                // inside a longer string and we want to resume from just after it.
                self.cursor = start + 1;
                Some(Err(err))
            }
        }
    }
}

/// Find the next occurrence of the `8=FIX` BeginString marker at or after `from`.
fn find_begin_string(buf: &[u8], from: usize) -> Option<usize> {
    if from >= buf.len() {
        return None;
    }
    memchr::memmem::find(&buf[from..], BEGIN_STRING_MARKER).map(|rel| from + rel)
}

fn parse_one_inner(
    buf: &[u8],
    offset: u64,
    sep: u8,
) -> Result<(RawMessage<'_>, usize), ParseError> {
    // Field 1: BeginString (tag 8).
    let (tag, _begin_string, after_begin) = read_field(buf, sep, 0, offset)?;
    if tag != TAG_BEGIN_STRING {
        return Err(ParseError::InvalidBeginString { offset });
    }

    // Field 2: BodyLength (tag 9).
    let (tag, body_len_bytes, after_body_len) = read_field(buf, sep, after_begin, offset)?;
    if tag != TAG_BODY_LENGTH {
        return Err(ParseError::InvalidBodyLength { offset });
    }
    let body_len =
        parse_uint(body_len_bytes).ok_or(ParseError::InvalidBodyLength { offset })? as usize;

    // BodyLength counts bytes from the byte AFTER the BodyLength field's separator
    // up to and including the separator before the CheckSum tag. So the CheckSum
    // tag begins exactly `body_len` bytes after `after_body_len`.
    let body_end = after_body_len
        .checked_add(body_len)
        .ok_or(ParseError::InvalidBodyLength { offset })?;
    if body_end > buf.len() {
        return Err(ParseError::UnexpectedEof);
    }

    // Field N: CheckSum (tag 10) at exactly `body_end`.
    let (tag, checksum_bytes, after_checksum) = read_field(buf, sep, body_end, offset)?;
    if tag != TAG_CHECKSUM {
        return Err(ParseError::InvalidBodyLength { offset });
    }
    // Checksum is verified but a mismatch is non-fatal: many human-readable FIX logs are
    // renderings of the wire format (e.g. SOH replaced with `|`) whose checksum was computed
    // over the original bytes and no longer matches the rendered ones. We still emit the
    // message so downstream consumers can see it; strict callers can recompute via
    // `validator::compute_checksum(raw)` themselves.
    if let Some(declared) = validator::parse_checksum(checksum_bytes) {
        let actual = validator::compute_checksum(&buf[..body_end]);
        if actual != declared {
            tracing::debug!(
                offset,
                expected = declared,
                got = actual,
                "checksum mismatch (message emitted regardless)"
            );
        }
    } else {
        tracing::debug!(
            offset,
            "checksum field is not three ASCII digits (message emitted regardless)"
        );
    }

    // Re-walk the message to populate the tag list. Cheap second pass; keeps the validating
    // code above readable and avoids stuffing the SmallVec only to throw it away on error.
    let raw = &buf[..after_checksum];
    let mut tags: SmallVec<[(u32, &[u8]); 32]> = SmallVec::new();
    let mut cursor = 0;
    while cursor < after_checksum {
        let (tag, value, next) = read_field(raw, sep, cursor, offset)?;
        tags.push((tag, value));
        cursor = next;
    }

    Ok((RawMessage { offset, raw, tags }, after_checksum))
}

/// Read one `tag=value<sep>` triple starting at `start`. Returns `(tag, value, next_cursor)`
/// where `next_cursor` points to the byte after the trailing separator.
#[inline]
fn read_field(
    buf: &[u8],
    sep: u8,
    start: usize,
    msg_offset: u64,
) -> Result<(u32, &[u8], usize), ParseError> {
    let rel_eq = memchr::memchr(b'=', &buf[start..]).ok_or(ParseError::UnexpectedEof)?;
    let eq = start + rel_eq;
    let tag = parse_uint(&buf[start..eq]).ok_or(ParseError::InvalidTag {
        offset: msg_offset + start as u64,
    })?;
    let value_start = eq + 1;
    let rel_sep = memchr::memchr(sep, &buf[value_start..]).ok_or(ParseError::UnexpectedEof)?;
    let sep_idx = value_start + rel_sep;
    Ok((tag, &buf[value_start..sep_idx], sep_idx + 1))
}

/// Parse an ASCII unsigned integer. Returns `None` on empty input, non-digit byte, or overflow.
#[inline]
fn parse_uint(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    let mut n: u32 = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TAG_MSG_TYPE;

    /// Build a Logon (35=A) message with correct BodyLength and CheckSum.
    fn logon_message() -> Vec<u8> {
        let body = b"35=A\x0149=SENDER\x0156=TARGET\x01";
        // BodyLength counts bytes after "9=NN<SOH>" up to and including the SOH before "10=".
        let body_len = body.len();
        let prefix = format!("8=FIX.4.4\x019={body_len}\x01");
        let mut msg: Vec<u8> = prefix.into_bytes();
        msg.extend_from_slice(body);
        let checksum = validator::compute_checksum(&msg);
        msg.extend_from_slice(format!("10={checksum:03}\x01").as_bytes());
        msg
    }

    #[test]
    fn parse_one_round_trips_a_well_formed_message() {
        let msg = logon_message();
        let (parsed, consumed) = parse_one(&msg).expect("must parse");
        assert_eq!(consumed, msg.len());
        assert_eq!(parsed.offset, 0);
        assert_eq!(parsed.raw, &msg[..]);
        // Tags: 8, 9, 35, 49, 56, 10
        assert_eq!(parsed.tags.len(), 6);
        assert_eq!(parsed.tags[0].0, TAG_BEGIN_STRING);
        assert_eq!(parsed.tags[0].1, b"FIX.4.4");
        assert_eq!(parsed.tags[2], (TAG_MSG_TYPE, &b"A"[..]));
    }

    #[test]
    fn parse_one_accepts_empty_value() {
        // 38=<SOH> (OrderQty empty) — structurally valid, parser keeps the field.
        let body = b"35=D\x0138=\x0155=AAPL\x01";
        let prefix = format!("8=FIX.4.4\x019={}\x01", body.len());
        let mut msg = prefix.into_bytes();
        msg.extend_from_slice(body);
        let cs = validator::compute_checksum(&msg);
        msg.extend_from_slice(format!("10={cs:03}\x01").as_bytes());

        let (parsed, _) = parse_one(&msg).expect("must parse");
        let qty = parsed
            .tags
            .iter()
            .find(|(t, _)| *t == 38)
            .expect("38 present");
        assert!(qty.1.is_empty(), "OrderQty value should be empty");
    }

    #[test]
    fn parse_one_emits_message_on_checksum_mismatch() {
        // Pipe-rendered logs and some archival formats keep the original (SOH-based) checksum
        // even after the transport bytes are altered, so a mismatch is non-fatal: we still
        // return the message.
        let mut msg = logon_message();
        let len = msg.len();
        msg[len - 4] = b'0';
        msg[len - 3] = b'0';
        msg[len - 2] = b'1';
        let (parsed, consumed) = parse_one(&msg).expect("must still emit the message");
        assert_eq!(consumed, msg.len());
        assert_eq!(parsed.tags.last().map(|(t, _)| *t), Some(TAG_CHECKSUM));
    }

    #[test]
    fn parse_one_rejects_inconsistent_body_length() {
        // Build a message but lie about BodyLength.
        let body = b"35=A\x0149=S\x0156=T\x01";
        let prefix = b"8=FIX.4.4\x019=999\x01"; // claims 999, real is body.len()
        let mut msg = prefix.to_vec();
        msg.extend_from_slice(body);
        let cs = validator::compute_checksum(&msg);
        msg.extend_from_slice(format!("10={cs:03}\x01").as_bytes());

        // body_end = after_body_len + 999 > buf.len() => UnexpectedEof.
        assert!(matches!(parse_one(&msg), Err(ParseError::UnexpectedEof)));
    }

    #[test]
    fn parse_one_rejects_missing_begin_string() {
        let buf = b"35=A\x019=10\x0110=001\x01";
        assert!(matches!(
            parse_one(buf),
            Err(ParseError::InvalidBeginString { .. })
        ));
    }

    #[test]
    fn parse_all_iterates_back_to_back_messages() {
        let mut combined = logon_message();
        combined.push(b'\n');
        combined.extend_from_slice(&logon_message());
        combined.push(b'\n');

        let results: Vec<_> = parse_all(&combined).collect();
        assert_eq!(results.len(), 2);
        for r in &results {
            assert!(r.is_ok());
        }
        // Second message has a non-zero offset.
        let second = results[1].as_ref().unwrap();
        assert!(second.offset > 0);
    }

    #[test]
    fn parse_all_skips_blank_lines_between_messages() {
        let mut combined = logon_message();
        combined.extend_from_slice(b"\n\n\n");
        combined.extend_from_slice(&logon_message());
        let results: Vec<_> = parse_all(&combined).filter_map(Result::ok).collect();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn parse_all_skips_variable_length_prefixes_and_noise() {
        // Simulate a Java-logback wrapped FIX log where each message is preceded by a
        // variable-length prefix and interleaved with application log noise.
        let m1 = logon_message();
        let m2 = logon_message();
        let mut combined = Vec::new();
        combined.extend_from_slice(b"2026-04-16 13:00:06.385  INFO 120615 [main] boot: starting\n");
        combined.extend_from_slice(b"2026-04-16 13:00:07.120  INFO 120615 [main] sess -> MME: ");
        combined.extend_from_slice(&m1);
        combined.extend_from_slice(b"\n2026-04-16 13:00:07.300  WARN 120615 [main] cron fired\n");
        combined.extend_from_slice(b"2026-04-16 13:00:08.001 DEBUG 120615 [io-1] sess <- MME: ");
        combined.extend_from_slice(&m2);
        combined.extend_from_slice(b"\n");

        let results: Vec<_> = parse_all(&combined).filter_map(Result::ok).collect();
        assert_eq!(results.len(), 2, "both embedded messages must be recovered");
        // Offsets point into the combined buffer, past the leading prefixes.
        assert!(results[0].offset > 0);
        assert!(results[1].offset > results[0].offset);
    }

    #[test]
    fn parse_all_recovers_after_a_corrupt_message() {
        let good1 = logon_message();
        // A corrupt block: starts with "8=" so it enters parse_one_inner but then fails.
        let corrupt = b"8=FIX.4.4\x019=99\x0135=A\x0110=000\x01\n";
        let good2 = logon_message();
        let mut combined = good1.clone();
        combined.push(b'\n');
        combined.extend_from_slice(corrupt);
        combined.extend_from_slice(&good2);
        combined.push(b'\n');

        let results: Vec<_> = parse_all(&combined).collect();
        // Expect: Ok, Err, Ok
        assert_eq!(results.len(), 3);
        assert!(results[0].is_ok());
        assert!(results[1].is_err());
        assert!(results[2].is_ok());
    }
}
