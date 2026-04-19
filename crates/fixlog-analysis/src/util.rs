//! Small helpers shared across analysis modules.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fixlog_core::RawMessage;
use fixlog_core::format::LogFormat;

/// Linear scan over `msg.tags` for `tag`. For ≤ 32 tags this beats any hash-map
/// lookup. Returns the raw `&[u8]` borrowed from the mmap.
#[inline]
pub fn find_tag<'a>(msg: &RawMessage<'a>, tag: u32) -> Option<&'a [u8]> {
    msg.tags.iter().find(|(t, _)| *t == tag).map(|(_, v)| *v)
}

/// Scan a raw FIX message body for the first occurrence of `tag` and
/// return its value slice. Stops on the first match so it's O(tag_pos)
/// rather than O(n_tags) and — crucially — avoids constructing a full
/// [`RawMessage`].
///
/// This is the hot-path helper for analyses that only need a single tag
/// (e.g. histogram over tag 52 / SendingTime) and whose cost is dominated
/// by per-message tokenization when done via [`parse_one_with_format`].
///
/// `bytes` must be a message slice starting at `8=FIX` (or the first
/// field post-line-prefix — i.e. what [`LogIndex::message_bytes`] returns).
pub fn extract_tag_raw<'a>(bytes: &'a [u8], format: &LogFormat, tag: u32) -> Option<&'a [u8]> {
    let sep = format.separator.as_byte();
    let mut i = 0;
    while i < bytes.len() {
        // Parse the tag up to '='.
        let eq = memchr_byte(b'=', &bytes[i..])?;
        let tag_bytes = &bytes[i..i + eq];
        let value_start = i + eq + 1;
        // Find the separator ending this value.
        let end_rel = match memchr_byte(sep, &bytes[value_start..]) {
            Some(n) => n,
            // No trailing separator: treat the remainder as the final value.
            None => bytes.len() - value_start,
        };
        let value = &bytes[value_start..value_start + end_rel];
        if let Some(parsed) = parse_u32_ascii(tag_bytes)
            && parsed == tag
        {
            return Some(value);
        }
        // Advance past the separator. If there was none, we're at EOF.
        i = value_start + end_rel + 1;
    }
    None
}

#[inline]
fn memchr_byte(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
}

/// Parse an ASCII u32 without allocating. Returns `None` on empty input, a
/// non-digit byte, or overflow.
#[inline]
pub fn parse_u32_ascii(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    let mut out: u32 = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(out)
}

/// Parse a FIX `SendingTime` (tag 52) value into a [`SystemTime`].
///
/// Accepted formats (per the FIX spec):
/// - `YYYYMMDD-HH:MM:SS`            (second precision)
/// - `YYYYMMDD-HH:MM:SS.sss`        (millisecond precision)
/// - `YYYYMMDD-HH:MM:SS.ssssss`     (microsecond precision)
/// - `YYYYMMDD-HH:MM:SS.sssssssss`  (nanosecond precision)
///
/// Missing fractional digits are interpreted as zeros. Returns `None` on any
/// parse failure; callers should treat a missing timestamp as "unknown" rather
/// than fatal.
pub fn parse_sending_time(bytes: &[u8]) -> Option<SystemTime> {
    // Minimum length: "YYYYMMDD-HH:MM:SS" = 17 bytes.
    if bytes.len() < 17 {
        return None;
    }
    if bytes[8] != b'-' || bytes[11] != b':' || bytes[14] != b':' {
        return None;
    }
    let year = parse_u32_ascii(&bytes[0..4])? as i32;
    let month = parse_u32_ascii(&bytes[4..6])?;
    let day = parse_u32_ascii(&bytes[6..8])?;
    let hour = parse_u32_ascii(&bytes[9..11])?;
    let minute = parse_u32_ascii(&bytes[12..14])?;
    let second = parse_u32_ascii(&bytes[15..17])?;
    let nanos = if bytes.len() > 17 {
        if bytes[17] != b'.' {
            return None;
        }
        let frac = &bytes[18..];
        if frac.is_empty() || frac.len() > 9 {
            return None;
        }
        let mut n = parse_u32_ascii(frac)? as u64;
        for _ in frac.len()..9 {
            n = n.checked_mul(10)?;
        }
        n
    } else {
        0
    };
    let days = days_from_civil(year, month, day)?;
    let secs_of_day = (hour as u64).checked_mul(3600)? + (minute as u64) * 60 + (second as u64);
    let total_secs = days.checked_mul(86_400)?.checked_add(secs_of_day)?;
    UNIX_EPOCH.checked_add(Duration::new(total_secs, nanos as u32))
}

/// Days since 1970-01-01 for a proleptic Gregorian calendar date. Returns
/// `None` on an invalid date. Algorithm: Howard Hinnant, "chrono-compatible"
/// — public domain.
fn days_from_civil(y: i32, m: u32, d: u32) -> Option<u64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = (era as i64) * 146_097 + doe as i64 - 719_468;
    if days < 0 { None } else { Some(days as u64) }
}

/// Nanoseconds since `UNIX_EPOCH` for a [`SystemTime`]. Returns `None` for
/// pre-epoch instants (impossible in FIX logs in practice).
#[inline]
pub fn system_time_to_nanos(t: SystemTime) -> Option<u128> {
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_u32_ascii_basics() {
        assert_eq!(parse_u32_ascii(b"0"), Some(0));
        assert_eq!(parse_u32_ascii(b"1"), Some(1));
        assert_eq!(parse_u32_ascii(b"12345"), Some(12345));
        assert_eq!(parse_u32_ascii(b""), None);
        assert_eq!(parse_u32_ascii(b"12a"), None);
        assert_eq!(parse_u32_ascii(b"4294967295"), Some(u32::MAX));
        assert_eq!(parse_u32_ascii(b"4294967296"), None);
    }

    #[test]
    fn sending_time_second_precision() {
        let t = parse_sending_time(b"20260417-12:34:56").unwrap();
        let nanos = system_time_to_nanos(t).unwrap();
        // 2026-04-17 = 20560 days since 1970-01-01 (Hinnant's chrono table);
        // 12:34:56 = 45296 seconds of day.
        let expected_secs: u64 = 20_560 * 86_400 + 45_296;
        assert_eq!(nanos, (expected_secs as u128) * 1_000_000_000);
    }

    #[test]
    fn sending_time_ms_precision() {
        let t = parse_sending_time(b"20260417-12:34:56.123").unwrap();
        let nanos = system_time_to_nanos(t).unwrap();
        let base = parse_sending_time(b"20260417-12:34:56").unwrap();
        let base_nanos = system_time_to_nanos(base).unwrap();
        assert_eq!(nanos - base_nanos, 123_000_000);
    }

    #[test]
    fn sending_time_ns_precision() {
        let t = parse_sending_time(b"20260417-12:34:56.123456789").unwrap();
        let base = parse_sending_time(b"20260417-12:34:56").unwrap();
        let d = t.duration_since(base).unwrap();
        assert_eq!(d.as_nanos(), 123_456_789);
    }

    #[test]
    fn extract_tag_raw_returns_first_match() {
        use fixlog_core::sniff;
        let msg = b"8=FIX.4.4\x019=40\x0135=D\x0134=7\x0149=A\x0156=B\x0152=20260417-12:34:56\x0155=AAPL\x0110=000\x01";
        let fmt = sniff(msg).expect("sniff");
        assert_eq!(extract_tag_raw(msg, &fmt, 35), Some(b"D".as_slice()));
        assert_eq!(
            extract_tag_raw(msg, &fmt, 52),
            Some(b"20260417-12:34:56".as_slice())
        );
        assert_eq!(extract_tag_raw(msg, &fmt, 999), None);
    }

    #[test]
    fn extract_tag_raw_handles_missing_trailing_separator() {
        // Synthesize a buffer without a trailing separator on the last
        // field — still recoverable.
        let msg = b"8=FIX.4.4\x0152=2026";
        use fixlog_core::sniff;
        // Use the preceding buffer with a proper sep for sniffing, then
        // pass the stripped one in.
        let full = b"8=FIX.4.4\x019=5\x0152=2026\x0110=000\x01";
        let fmt = sniff(full).expect("sniff");
        assert_eq!(extract_tag_raw(msg, &fmt, 52), Some(b"2026".as_slice()));
    }

    #[test]
    fn sending_time_rejects_malformed() {
        assert!(parse_sending_time(b"too-short").is_none());
        assert!(parse_sending_time(b"20260417T12:34:56").is_none()); // wrong sep
        assert!(parse_sending_time(b"20260417-12:34:56.").is_none()); // empty frac
        assert!(parse_sending_time(b"20260217-12:34:56.1234567890").is_none()); // >9 frac
    }
}
