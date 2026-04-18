//! Index construction from a byte buffer.
//!
//! Single-threaded for now; the parallel variant lands in P2-T05. The builder reuses the
//! parser's iterator so its behavior tracks the parser exactly (prefix-agnostic scan,
//! non-fatal checksum mismatches, corrupt messages emitted as `Err` and skipped here).
//!
//! # Invariants
//!
//! - After `build_from_bytes`, `index.consumed` is the absolute offset immediately past
//!   the last message that was successfully indexed. If the buffer ended mid-message,
//!   those trailing bytes are *not* claimed — they will be re-scanned next time.
//! - `append_from_offset(buf, from)` requires `from == index.consumed`.

use crate::{HotTags, IndexError, LogIndex, MessageOffset};
use fixlog_format::LogFormat;
use fixlog_parser::parse_all_with_format;

/// Build a fresh index over `buf` using the layout from `format` and the default hot-tag
/// set.
///
/// Malformed messages are dropped silently (they already trigger `tracing::warn!` inside
/// the parser); only well-formed offsets end up in the index.
pub fn build_from_bytes(buf: &[u8], format: &LogFormat) -> LogIndex {
    build_from_bytes_with_hot_tags(buf, format, HotTags::default_set())
}

/// Like [`build_from_bytes`] but with a caller-supplied hot-tag set. Pass
/// `HotTags::empty()` to skip the secondary index entirely (slightly faster build, no
/// lookup support).
pub fn build_from_bytes_with_hot_tags(buf: &[u8], format: &LogFormat, tags: HotTags) -> LogIndex {
    let mut index = LogIndex::with_hot_tags(tags);
    extend_index(&mut index, buf, format, 0);
    index
}

impl LogIndex {
    /// Extend the index with messages starting at or after `from` in `buf`.
    ///
    /// `from` must equal the current `consumed` watermark; otherwise returns
    /// [`IndexError::NonContiguousAppend`] without mutating the index. This invariant
    /// prevents partial double-indexing when the caller miscomputes the delta.
    ///
    /// The caller is expected to pass `buf` whose length is the *new* total file size
    /// (e.g. the re-mmapped file). Only bytes at `from..` are scanned, so a message that
    /// straddled the previous boundary (the producer flushed only half of it last time)
    /// is rescanned from its original start.
    pub fn append_from_offset(
        &mut self,
        buf: &[u8],
        from: u64,
        format: &LogFormat,
    ) -> Result<usize, IndexError> {
        if from != self.consumed {
            return Err(IndexError::NonContiguousAppend {
                consumed: self.consumed,
                from,
            });
        }
        if (buf.len() as u64) < self.consumed {
            return Err(IndexError::BufferShrank {
                consumed: self.consumed,
                buf_len: buf.len() as u64,
            });
        }
        let before = self.messages.len();
        extend_index(self, buf, format, from as usize);
        Ok(self.messages.len() - before)
    }
}

/// Core loop: walk the parser from `start` and push every `Ok` message into `index`.
///
/// `parse_all_with_format` only scans the slice you hand it and reports offsets relative
/// to that slice, so we re-base into absolute terms before pushing. `consumed` is
/// updated to the end of the last successfully indexed message (see the invariants in
/// the module doc). Hot-tag occurrences on each message are handed to
/// `index.secondary.record` with the message's final ordinal.
fn extend_index(index: &mut LogIndex, buf: &[u8], format: &LogFormat, start: usize) {
    if start >= buf.len() {
        return;
    }
    let tail = &buf[start..];
    for msg in parse_all_with_format(tail, format).flatten() {
        let absolute_start = start as u64 + msg.offset;
        let len = u32::try_from(msg.raw.len()).unwrap_or(u32::MAX);
        let offset = MessageOffset {
            start: absolute_start,
            len,
        };
        index.consumed = offset.end();
        let ordinal = u32::try_from(index.messages.len()).unwrap_or(u32::MAX);
        index.secondary.record(&msg, ordinal);
        index.messages.push(offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixlog_format::sniff;
    use fixlog_parser::parse_all_with_format as parse;

    const MINIMAL: &[u8] = include_bytes!("../../../fixtures/synthetic/minimal_4.4.log");
    const PIPED: &[u8] = include_bytes!("../../../fixtures/synthetic/pipe_separated.log");
    const PREFIXED: &[u8] = include_bytes!("../../../fixtures/synthetic/with_timestamp_prefix.log");

    fn parser_count(buf: &[u8], fmt: &LogFormat) -> usize {
        parse(buf, fmt).filter_map(Result::ok).count()
    }

    #[test]
    fn build_matches_parser_count_for_soh_fixture() {
        let fmt = sniff(MINIMAL).expect("sniffable");
        let expected = parser_count(MINIMAL, &fmt);
        let index = build_from_bytes(MINIMAL, &fmt);
        assert_eq!(index.len(), expected);
        assert!(!index.is_empty());
        // `consumed` must equal the end of the last message.
        assert_eq!(index.consumed, index.messages.last().unwrap().end());
    }

    #[test]
    fn build_matches_parser_count_for_pipe_fixture() {
        let fmt = sniff(PIPED).expect("sniffable");
        let expected = parser_count(PIPED, &fmt);
        let index = build_from_bytes(PIPED, &fmt);
        assert_eq!(index.len(), expected);
    }

    #[test]
    fn build_matches_parser_count_for_prefixed_fixture() {
        let fmt = sniff(PREFIXED).expect("sniffable");
        let expected = parser_count(PREFIXED, &fmt);
        let index = build_from_bytes(PREFIXED, &fmt);
        assert_eq!(index.len(), expected);
    }

    #[test]
    fn offsets_are_monotonic_and_in_bounds() {
        let fmt = sniff(MINIMAL).expect("sniffable");
        let index = build_from_bytes(MINIMAL, &fmt);
        let mut last = 0u64;
        for off in &index.messages {
            assert!(off.start >= last, "offsets must be monotonic");
            assert!(off.end() <= MINIMAL.len() as u64, "offset escapes buffer");
            last = off.end();
        }
    }

    #[test]
    fn message_bytes_round_trips_through_parser() {
        let fmt = sniff(MINIMAL).expect("sniffable");
        let index = build_from_bytes(MINIMAL, &fmt);
        for i in 0..index.len() {
            let slice = index.message_bytes(MINIMAL, i).expect("in range");
            assert!(slice.starts_with(b"8=FIX"), "slice must start at 8=FIX");
            let reparsed: Vec<_> = parse(slice, &fmt).filter_map(Result::ok).collect();
            assert_eq!(reparsed.len(), 1, "isolated slice must parse as 1 message");
        }
    }

    #[test]
    fn append_from_offset_rejects_non_contiguous_from() {
        let fmt = sniff(MINIMAL).expect("sniffable");
        let mut index = build_from_bytes(MINIMAL, &fmt);
        let err = index
            .append_from_offset(MINIMAL, index.consumed + 1, &fmt)
            .unwrap_err();
        assert_eq!(
            err,
            IndexError::NonContiguousAppend {
                consumed: index.consumed,
                from: index.consumed + 1
            }
        );
    }

    #[test]
    fn append_from_offset_rejects_shrunken_buffer() {
        let fmt = sniff(MINIMAL).expect("sniffable");
        let mut index = build_from_bytes(MINIMAL, &fmt);
        let consumed = index.consumed;
        // Buffer that's shorter than what we already consumed. Caller must rebuild.
        let shorter = &MINIMAL[..(consumed as usize / 2)];
        // Fix `from` so the NonContiguous check doesn't fire first.
        let err = index
            .append_from_offset(shorter, consumed, &fmt)
            .unwrap_err();
        assert_eq!(
            err,
            IndexError::BufferShrank {
                consumed,
                buf_len: shorter.len() as u64
            }
        );
    }

    #[test]
    fn secondary_index_materializes_hot_tags() {
        let fmt = sniff(MINIMAL).expect("sniffable");
        let index = build_from_bytes(MINIMAL, &fmt);
        // MINIMAL contains at least one NewOrderSingle (35=D) and Logon (35=A).
        let d_ords = index.secondary.lookup(fixlog_parser::TAG_MSG_TYPE, b"D");
        let a_ords = index.secondary.lookup(fixlog_parser::TAG_MSG_TYPE, b"A");
        assert!(!d_ords.is_empty(), "must have 35=D entries");
        assert!(!a_ords.is_empty(), "must have 35=A entries");
        // Ordinals must be valid indices into messages.
        for ord in d_ords.iter().chain(a_ords.iter()) {
            assert!((*ord as usize) < index.len(), "ordinal in range");
        }
    }

    #[test]
    fn secondary_index_disabled_when_tags_empty() {
        let fmt = sniff(MINIMAL).expect("sniffable");
        let index =
            build_from_bytes_with_hot_tags(MINIMAL, &fmt, fixlog_index_crate::HotTags::empty());
        assert_eq!(index.secondary.key_count(), 0);
        // Primary still works.
        assert!(!index.is_empty());
    }

    // Workaround: `use crate as fixlog_index_crate` so the HotTags path in the test above
    // stays explicit even when tests are moved around.
    use crate as fixlog_index_crate;

    #[test]
    fn secondary_ordinals_match_primary_scan() {
        // Brute-force verification: every ordinal returned for a `(tag, value)` lookup
        // must point at a primary message whose tags include that pair.
        let fmt = sniff(MINIMAL).expect("sniffable");
        let index = build_from_bytes(MINIMAL, &fmt);
        for tag in [
            fixlog_parser::TAG_MSG_TYPE,
            fixlog_parser::TAG_SENDER_COMP_ID,
            fixlog_parser::TAG_TARGET_COMP_ID,
        ] {
            // Collect every distinct value observed for this tag.
            let mut seen_values = std::collections::BTreeSet::new();
            for i in 0..index.len() {
                let bytes = index.message_bytes(MINIMAL, i).unwrap();
                for (t, v) in parse(bytes, &fmt)
                    .filter_map(Result::ok)
                    .flat_map(|m| m.tags)
                {
                    if t == tag {
                        seen_values.insert(v.to_vec());
                    }
                }
            }
            for v in &seen_values {
                let ords = index.secondary.lookup(tag, v);
                assert!(
                    !ords.is_empty(),
                    "expected at least one ordinal for tag={tag} value={v:?}"
                );
                for &ord in ords {
                    let bytes = index.message_bytes(MINIMAL, ord as usize).unwrap();
                    let m = parse(bytes, &fmt)
                        .filter_map(Result::ok)
                        .next()
                        .expect("reparses");
                    assert!(
                        m.tags.iter().any(|(t, val)| *t == tag && val == v),
                        "ordinal {ord} for tag={tag} value={v:?} did not back-match"
                    );
                }
            }
        }
    }

    #[test]
    fn append_from_offset_produces_same_index_as_single_build() {
        let fmt = sniff(MINIMAL).expect("sniffable");
        // Build a full index first to know where message boundaries lie.
        let full = build_from_bytes(MINIMAL, &fmt);
        // Pick a prefix that ends mid-buffer (between two messages ~halfway through the file).
        // The prefix itself may end *inside* a message — that trailing partial message is
        // expected to be re-scanned by the appender, which is the whole point of the test.
        let half_idx = full.len() / 2;
        let split = full.messages[half_idx].start as usize + 5; // +5 bytes into the header
        let prefix = &MINIMAL[..split];

        let mut incremental = build_from_bytes(prefix, &fmt);
        // `consumed` should be at the previous message boundary, before `split`.
        assert!(incremental.consumed < split as u64);

        let appended = incremental
            .append_from_offset(MINIMAL, incremental.consumed, &fmt)
            .unwrap();
        assert_eq!(incremental.messages, full.messages);
        assert_eq!(incremental.consumed, full.consumed);
        assert_eq!(incremental.len(), full.len());
        assert_eq!(appended, full.len() - parser_count(prefix, &fmt));
        // Secondary index must match too — ordinals and key set should be identical.
        for tag in [
            fixlog_parser::TAG_MSG_TYPE,
            fixlog_parser::TAG_SENDER_COMP_ID,
            fixlog_parser::TAG_TARGET_COMP_ID,
        ] {
            for value in [b"D" as &[u8], b"A", b"8", b"SENDER", b"TARGET"] {
                assert_eq!(
                    incremental.secondary.lookup(tag, value),
                    full.secondary.lookup(tag, value),
                    "secondary mismatch at tag={tag} value={value:?}"
                );
            }
        }
    }
}
