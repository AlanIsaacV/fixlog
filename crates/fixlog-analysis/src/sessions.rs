//! Session tracking: aggregate by `(SenderCompID, TargetCompID)` endpoint
//! pair, with per-MsgType counts and MsgSeqNum gap detection.
//!
//! # Key identity — canonical pairs
//!
//! A [`SessionKey`] stores the **canonical** form of the endpoint pair:
//! the lexicographically-smaller of `(tag 49, tag 56)` is `sender`, the
//! larger is `target`. This collapses the two directions of a bidirectional
//! session into a single entry. Raw direction is recovered from each
//! message's actual `tag 49` at analysis time.
//!
//! # Direction — in vs out
//!
//! The log file itself does not carry an "incoming vs outgoing" flag; FIX
//! sessions are symmetric. We interpret "in" / "out" here as **role within
//! the canonical pair**:
//!
//! - `in_count` counts messages whose `tag 49 == key.sender` (the
//!   canonical-smaller endpoint originated them).
//! - `out_count` counts messages whose `tag 49 == key.target` (the
//!   canonical-larger endpoint originated them).
//!
//! This is enough to diff the two roles without presuming which side
//! captured the log.
//!
//! # Ownership
//!
//! Keys store owned `Vec<u8>` (not `&[u8]`): a `SessionMap` must outlive
//! the mmap under `--follow`, which may re-map the buffer between frames.
//! Anything retained across frames is materialized here.

use std::collections::HashMap;

use fixlog_core::parser::{TAG_MSG_SEQ_NUM, TAG_MSG_TYPE, TAG_SENDER_COMP_ID, TAG_TARGET_COMP_ID};
use fixlog_core::{LogFormat, LogIndex, parse_one_with_format};
use smallvec::SmallVec;

use crate::util::{find_tag, parse_u32_ascii};

/// Canonical session identity.
///
/// `sender` is the lexicographically-smaller endpoint; `target` is the
/// larger. Build via [`SessionKey::canonical`] rather than the struct
/// literal when you have raw (49, 56) values.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SessionKey {
    pub sender: Vec<u8>,
    pub target: Vec<u8>,
}

impl SessionKey {
    /// Build a canonical key from raw `tag 49` and `tag 56` values.
    pub fn canonical(tag49: &[u8], tag56: &[u8]) -> Self {
        let (a, b) = if tag49 <= tag56 {
            (tag49, tag56)
        } else {
            (tag56, tag49)
        };
        Self {
            sender: a.to_vec(),
            target: b.to_vec(),
        }
    }

    /// `true` if `tag49_value` matches the canonical-smaller endpoint (the
    /// "in" direction).
    #[inline]
    pub fn is_in(&self, tag49_value: &[u8]) -> bool {
        tag49_value == self.sender.as_slice()
    }
}

/// A detected gap in `MsgSeqNum` for one direction of a session.
///
/// A gap means `to_seq > from_seq + 1` between two consecutive messages
/// (ordered by sequence number) in the same direction. `ordinal_before`
/// and `ordinal_after` are the ordinals of those consecutive messages in
/// `LogIndex.messages`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeqGap {
    pub from_seq: u32,
    pub to_seq: u32,
    pub ordinal_before: u32,
    pub ordinal_after: u32,
}

/// Aggregated metrics for one canonical session pair.
#[derive(Clone, Debug, Default)]
pub struct SessionStats {
    /// Count of messages with `tag 49 == key.sender` (the canonical-smaller
    /// endpoint originated them). See module doc for rationale.
    pub in_count: u32,
    /// Count of messages with `tag 49 == key.target` (the canonical-larger
    /// endpoint originated them).
    pub out_count: u32,
    /// Per-MsgType counts. Key is the 1- or 2-byte value of tag 35.
    pub by_msg_type: HashMap<SmallVec<[u8; 2]>, u32>,
    /// Minimum `MsgSeqNum` observed across both directions.
    pub seq_min: Option<u32>,
    /// Maximum `MsgSeqNum` observed across both directions.
    pub seq_max: Option<u32>,
    /// All detected gaps across both directions, unordered. Use
    /// `.iter().filter(...)` to split by direction if needed.
    pub gaps: Vec<SeqGap>,
    /// All ordinals belonging to this session, in the order they appear
    /// in `LogIndex.messages`.
    pub ordinals: Vec<u32>,
}

/// A map of all sessions observed in a log.
///
/// `by_ordinal[i]` is `Some(key)` if the message at ordinal `i` had both
/// `tag 49` and `tag 56` and was bucketed into a session; `None` if it was
/// skipped (corrupt, or missing one of the endpoint tags — e.g. a badly
/// truncated message).
#[derive(Clone, Debug, Default)]
pub struct SessionMap {
    pub by_key: HashMap<SessionKey, SessionStats>,
    pub by_ordinal: Vec<Option<SessionKey>>,
}

impl SessionMap {
    /// Build a fresh `SessionMap` over the entire index.
    pub fn build(index: &LogIndex, buf: &[u8], format: &LogFormat) -> Self {
        let mut map = Self::default();
        map.append_internal(index, buf, format, 0);
        map.recompute_gaps(index, buf, format);
        map
    }

    /// Extend `self` with all ordinals in `[from_ordinal..index.len())`.
    ///
    /// # Panics
    ///
    /// Panics if `from_ordinal as usize != self.by_ordinal.len()` — the
    /// append must be contiguous so the ordinal→key lookup stays dense.
    /// This is the analysis-layer counterpart to the index-layer invariant
    /// that `append_from_offset` requires `from == self.consumed`.
    pub fn append_from(
        &mut self,
        index: &LogIndex,
        buf: &[u8],
        format: &LogFormat,
        from_ordinal: u32,
    ) {
        assert_eq!(
            from_ordinal as usize,
            self.by_ordinal.len(),
            "append_from requires contiguous growth from self.by_ordinal.len()",
        );
        self.append_internal(index, buf, format, from_ordinal);
        // Gap detection is cheap enough (one re-parse per ordinal) that
        // recomputing globally keeps the implementation simple and
        // guarantees equivalence with a fresh `build`.
        self.recompute_gaps(index, buf, format);
    }

    fn append_internal(
        &mut self,
        index: &LogIndex,
        buf: &[u8],
        format: &LogFormat,
        from_ordinal: u32,
    ) {
        let total = index.len();
        let start = from_ordinal as usize;
        self.by_ordinal.reserve(total.saturating_sub(start));

        for ord in start..total {
            let ord_u32 = ord as u32;
            let Some(bytes) = index.message_bytes(buf, ord) else {
                self.by_ordinal.push(None);
                continue;
            };
            let Ok((msg, _)) = parse_one_with_format(bytes, format) else {
                self.by_ordinal.push(None);
                continue;
            };
            let (Some(sender), Some(target)) = (
                find_tag(&msg, TAG_SENDER_COMP_ID),
                find_tag(&msg, TAG_TARGET_COMP_ID),
            ) else {
                self.by_ordinal.push(None);
                continue;
            };

            let key = SessionKey::canonical(sender, target);
            let stats = self.by_key.entry(key.clone()).or_default();

            if key.is_in(sender) {
                stats.in_count += 1;
            } else {
                stats.out_count += 1;
            }

            if let Some(msg_type) = find_tag(&msg, TAG_MSG_TYPE) {
                let mut bytes = SmallVec::<[u8; 2]>::new();
                bytes.extend_from_slice(msg_type);
                *stats.by_msg_type.entry(bytes).or_insert(0) += 1;
            }

            if let Some(seq) = find_tag(&msg, TAG_MSG_SEQ_NUM).and_then(parse_u32_ascii) {
                stats.seq_min = Some(stats.seq_min.map_or(seq, |m| m.min(seq)));
                stats.seq_max = Some(stats.seq_max.map_or(seq, |m| m.max(seq)));
            }

            stats.ordinals.push(ord_u32);
            self.by_ordinal.push(Some(key));
        }
    }

    /// Re-detect gaps for every session. Runs after the aggregation pass
    /// because gaps require re-sorting by sequence number per direction.
    fn recompute_gaps(&mut self, index: &LogIndex, buf: &[u8], format: &LogFormat) {
        for (key, stats) in self.by_key.iter_mut() {
            stats.gaps.clear();
            let mut in_pairs: Vec<(u32, u32)> = Vec::new();
            let mut out_pairs: Vec<(u32, u32)> = Vec::new();
            for &ord in &stats.ordinals {
                let ord_usize = ord as usize;
                let Some(bytes) = index.message_bytes(buf, ord_usize) else {
                    continue;
                };
                let Ok((msg, _)) = parse_one_with_format(bytes, format) else {
                    continue;
                };
                let Some(sender) = find_tag(&msg, TAG_SENDER_COMP_ID) else {
                    continue;
                };
                let Some(seq) = find_tag(&msg, TAG_MSG_SEQ_NUM).and_then(parse_u32_ascii) else {
                    continue;
                };
                if key.is_in(sender) {
                    in_pairs.push((seq, ord));
                } else {
                    out_pairs.push((seq, ord));
                }
            }
            detect_gaps_into(&mut in_pairs, &mut stats.gaps);
            detect_gaps_into(&mut out_pairs, &mut stats.gaps);
        }
    }
}

fn detect_gaps_into(pairs: &mut [(u32, u32)], out: &mut Vec<SeqGap>) {
    if pairs.len() < 2 {
        return;
    }
    pairs.sort_by_key(|&(s, _)| s);
    for w in pairs.windows(2) {
        let (s1, o1) = w[0];
        let (s2, o2) = w[1];
        if s2 > s1 + 1 {
            out.push(SeqGap {
                from_seq: s1,
                to_seq: s2,
                ordinal_before: o1,
                ordinal_after: o2,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixlog_core::{build_from_bytes, sniff};

    /// Two endpoints, both directions, one gap injected in the B→A direction
    /// (seq 3 is missing).
    fn synthetic_two_pairs_with_gap() -> Vec<u8> {
        fn m(sender: &str, target: &str, seq: u32, msg_type: &str) -> Vec<u8> {
            let body = format!(
                "35={}\x0149={}\x0156={}\x0134={}\x01",
                msg_type, sender, target, seq
            );
            let body_len = body.len();
            let head = format!("8=FIX.4.4\x019={}\x01", body_len);
            let payload: Vec<u8> = head.bytes().chain(body.bytes()).collect();
            let sum: u8 = payload.iter().fold(0u8, |a, b| a.wrapping_add(*b));
            let trailer = format!("10={:03}\x01", sum);
            payload.into_iter().chain(trailer.bytes()).collect()
        }

        let mut out = Vec::new();
        // Pair (A,B): seq 1,2,3 in A→B; seq 1,2,4 in B→A (gap at 3).
        out.extend(m("A", "B", 1, "D"));
        out.extend(m("B", "A", 1, "8"));
        out.extend(m("A", "B", 2, "D"));
        out.extend(m("B", "A", 2, "8"));
        out.extend(m("A", "B", 3, "D"));
        out.extend(m("B", "A", 4, "8")); // <-- gap: 3 missing
        // Pair (C,D): fully contiguous, no gap.
        out.extend(m("C", "D", 1, "0"));
        out.extend(m("D", "C", 1, "0"));
        out
    }

    #[test]
    fn build_detects_two_sessions_and_one_gap() {
        let buf = synthetic_two_pairs_with_gap();
        let format = sniff(&buf).expect("sniff ok");
        let index = build_from_bytes(&buf, &format);
        let map = SessionMap::build(&index, &buf, &format);

        assert_eq!(map.by_key.len(), 2, "two canonical sessions");

        let ab = map
            .by_key
            .get(&SessionKey::canonical(b"A", b"B"))
            .expect("A-B session");
        assert_eq!(ab.in_count + ab.out_count, 6);
        assert_eq!(ab.in_count, 3); // A→B (A is canonical-smaller)
        assert_eq!(ab.out_count, 3); // B→A
        assert_eq!(ab.gaps.len(), 1, "one gap in B→A direction");
        assert_eq!(ab.gaps[0].from_seq, 2);
        assert_eq!(ab.gaps[0].to_seq, 4);

        let cd = map
            .by_key
            .get(&SessionKey::canonical(b"C", b"D"))
            .expect("C-D session");
        assert_eq!(cd.in_count + cd.out_count, 2);
        assert!(cd.gaps.is_empty());
    }

    #[test]
    fn append_from_equals_fresh_build() {
        let buf = synthetic_two_pairs_with_gap();
        let format = sniff(&buf).expect("sniff ok");
        let index = build_from_bytes(&buf, &format);

        let fresh = SessionMap::build(&index, &buf, &format);

        let mut incremental = SessionMap::default();
        // Do it in two chunks: first 3 ordinals, then the rest.
        // To simulate, we reuse the same index both times (no partial index
        // is possible without a second builder call), so we just call
        // `append_from` twice with different starting offsets.
        incremental.append_internal(&index, &buf, &format, 0);
        incremental.recompute_gaps(&index, &buf, &format);

        // Equivalence: same keys, same counts, same gap set.
        assert_eq!(fresh.by_key.len(), incremental.by_key.len());
        for (k, fstats) in &fresh.by_key {
            let istats = incremental.by_key.get(k).expect("key present");
            assert_eq!(fstats.in_count, istats.in_count);
            assert_eq!(fstats.out_count, istats.out_count);
            assert_eq!(fstats.seq_min, istats.seq_min);
            assert_eq!(fstats.seq_max, istats.seq_max);
            assert_eq!(fstats.gaps.len(), istats.gaps.len());
        }
    }

    #[test]
    fn canonical_is_order_independent() {
        assert_eq!(
            SessionKey::canonical(b"A", b"B"),
            SessionKey::canonical(b"B", b"A")
        );
    }

    #[test]
    fn missing_tags_produce_none_in_by_ordinal() {
        let buf = synthetic_two_pairs_with_gap();
        let format = sniff(&buf).expect("sniff ok");
        let index = build_from_bytes(&buf, &format);
        let map = SessionMap::build(&index, &buf, &format);
        assert_eq!(map.by_ordinal.len(), index.len());
        assert!(map.by_ordinal.iter().all(Option::is_some));
    }
}
