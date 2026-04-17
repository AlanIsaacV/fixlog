//! Heuristics that infer a [`LogFormat`] from the first bytes of a log file.

use crate::{Encoding, LineEnding, LinePrefix, LogFormat, MessageBoundary, Separator, SniffError};

/// Inspect `sample` and return the best-guess [`LogFormat`].
///
/// `sample` should be the first ~64KB of the file. The algorithm:
///
/// 1. Detect the line ending (CRLF vs LF) by counting `\r\n` occurrences.
/// 2. Detect the field separator by counting `<sep><digits>+=` patterns for each candidate
///    byte (`SOH`, `|`, `^`, `;`); the one with the most matches wins.
/// 3. Detect the line prefix by finding the byte offset of `8=FIX` on each sampled line and
///    requiring it to be consistent. If consistent and non-zero, that offset is the prefix length.
pub fn sniff(sample: &[u8]) -> Result<LogFormat, SniffError> {
    if sample.is_empty() {
        return Err(SniffError::EmptySample);
    }

    let line_ending = detect_line_ending(sample);
    let separator = detect_separator(sample)?;
    let line_prefix = detect_line_prefix(sample, line_ending)?;

    Ok(LogFormat {
        separator,
        line_prefix,
        encoding: Encoding::Utf8,
        line_ending,
        message_boundary: MessageBoundary::Line,
    })
}

fn detect_line_ending(sample: &[u8]) -> LineEnding {
    let mut crlf = 0usize;
    let mut i = 0;
    while let Some(pos) = memchr::memchr(b'\r', &sample[i..]) {
        let abs = i + pos;
        if sample.get(abs + 1) == Some(&b'\n') {
            crlf += 1;
        }
        i = abs + 1;
    }
    if crlf > 0 {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

fn detect_separator(sample: &[u8]) -> Result<Separator, SniffError> {
    const CANDIDATES: &[(u8, Separator)] = &[
        (0x01, Separator::Soh),
        (b'|', Separator::Pipe),
        (b'^', Separator::Caret),
        (b';', Separator::Semicolon),
    ];

    let mut best: Option<(usize, Separator)> = None;
    for &(byte, sep) in CANDIDATES {
        let count = count_field_boundaries(sample, byte);
        if count == 0 {
            continue;
        }
        match best {
            Some((c, _)) if count <= c => {}
            _ => best = Some((count, sep)),
        }
    }
    best.map(|(_, s)| s).ok_or(SniffError::NoSeparator)
}

/// Count occurrences of `<sep><digit>+=` in `sample`. This is the unambiguous fingerprint of
/// a FIX field boundary regardless of the rest of the message contents.
fn count_field_boundaries(sample: &[u8], sep: u8) -> usize {
    let mut count = 0;
    let mut i = 0;
    while let Some(pos) = memchr::memchr(sep, &sample[i..]) {
        let abs = i + pos;
        let mut j = abs + 1;
        while j < sample.len() && sample[j].is_ascii_digit() {
            j += 1;
        }
        if j > abs + 1 && sample.get(j) == Some(&b'=') {
            count += 1;
        }
        i = abs + 1;
    }
    count
}

fn detect_line_prefix(sample: &[u8], line_ending: LineEnding) -> Result<LinePrefix, SniffError> {
    let needle = b"8=FIX";
    let mut prefix_lens: Vec<usize> = Vec::new();

    let lines = sample_lines(sample, line_ending);
    for line in lines {
        if let Some(pos) = find_subslice(line, needle) {
            prefix_lens.push(pos);
        }
    }

    if prefix_lens.is_empty() {
        return Err(SniffError::NoBeginString);
    }

    let first = prefix_lens[0];
    if prefix_lens.iter().all(|&l| l == first) {
        Ok(if first == 0 {
            LinePrefix::None
        } else {
            LinePrefix::Fixed(first)
        })
    } else {
        // Inconsistent prefixes — best to assume none and let the parser deal with stray bytes.
        Ok(LinePrefix::None)
    }
}

fn sample_lines(sample: &[u8], line_ending: LineEnding) -> Vec<&[u8]> {
    let term = match line_ending {
        LineEnding::Lf => b'\n',
        LineEnding::CrLf => b'\n',
    };
    let mut lines = Vec::new();
    let mut start = 0;
    while start < sample.len() {
        let end = memchr::memchr(term, &sample[start..])
            .map(|i| start + i)
            .unwrap_or(sample.len());
        let mut line = &sample[start..end];
        if matches!(line_ending, LineEnding::CrLf) && line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        if !line.is_empty() {
            lines.push(line);
        }
        start = end + 1;
    }
    lines
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sample_fails() {
        assert_eq!(sniff(&[]), Err(SniffError::EmptySample));
    }

    #[test]
    fn detects_soh_separator() {
        let sample = b"8=FIX.4.4\x019=12\x0135=A\x0110=001\x01\n";
        let fmt = sniff(sample).unwrap();
        assert_eq!(fmt.separator, Separator::Soh);
        assert_eq!(fmt.line_prefix, LinePrefix::None);
        assert_eq!(fmt.line_ending, LineEnding::Lf);
    }

    #[test]
    fn detects_pipe_separator() {
        let sample = b"8=FIX.4.4|9=12|35=A|10=001|\n";
        let fmt = sniff(sample).unwrap();
        assert_eq!(fmt.separator, Separator::Pipe);
        assert_eq!(fmt.line_prefix, LinePrefix::None);
    }

    #[test]
    fn detects_fixed_timestamp_prefix() {
        let sample = b"20260416-13:30:00.000 : 8=FIX.4.4\x019=12\x0135=A\x0110=001\x01\n\
                       20260416-13:30:30.100 : 8=FIX.4.4\x019=12\x0135=0\x0110=001\x01\n";
        let fmt = sniff(sample).unwrap();
        assert_eq!(fmt.separator, Separator::Soh);
        assert_eq!(fmt.line_prefix, LinePrefix::Fixed(24));
    }

    #[test]
    fn detects_crlf() {
        let sample = b"8=FIX.4.4\x019=12\x0135=A\x0110=001\x01\r\n";
        let fmt = sniff(sample).unwrap();
        assert_eq!(fmt.line_ending, LineEnding::CrLf);
    }

    #[test]
    fn errors_when_no_begin_string() {
        let sample = b"hello world\nthis is not fix\n";
        assert!(matches!(
            sniff(sample),
            Err(SniffError::NoBeginString | SniffError::NoSeparator)
        ));
    }

    #[test]
    fn count_field_boundaries_ignores_separator_inside_value() {
        // SOH appears 3 times here but only 2 are field boundaries
        // (the trailing one isn't followed by digits=).
        let sample = b"8=FIX.4.4\x019=12\x0135=A\x01";
        assert_eq!(count_field_boundaries(sample, 0x01), 2);
    }
}
