//! FIX checksum helpers.
//!
//! The CheckSum (tag 10) is the sum of all bytes from the start of the message up to and
//! including the separator byte that precedes the `10=` field, taken modulo 256 and rendered
//! as a 3-digit zero-padded decimal.

/// Sum every byte in `bytes` modulo 256.
#[inline]
pub fn compute_checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b))
}

/// Parse a 3-digit ASCII checksum (e.g. `b"161"`) into a `u8`. Returns `None` if the bytes
/// are not exactly three ASCII digits or if the resulting value would not fit in `u8`.
pub fn parse_checksum(bytes: &[u8]) -> Option<u8> {
    if bytes.len() != 3 {
        return None;
    }
    let mut n: u16 = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (b - b'0') as u16;
    }
    u8::try_from(n).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_checksum_wraps_around() {
        // 0xFF + 0x01 = 0x00.
        assert_eq!(compute_checksum(&[0xFF, 0x01]), 0);
    }

    #[test]
    fn parse_checksum_accepts_three_digits() {
        assert_eq!(parse_checksum(b"000"), Some(0));
        assert_eq!(parse_checksum(b"161"), Some(161));
        assert_eq!(parse_checksum(b"255"), Some(255));
    }

    #[test]
    fn parse_checksum_rejects_overflow_and_garbage() {
        assert_eq!(parse_checksum(b"256"), None);
        assert_eq!(parse_checksum(b"99"), None);
        assert_eq!(parse_checksum(b"1234"), None);
        assert_eq!(parse_checksum(b"a23"), None);
    }
}
