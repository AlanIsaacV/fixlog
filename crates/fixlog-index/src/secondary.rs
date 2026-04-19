//! Secondary index over "hot" tags.
//!
//! The primary index only records message offsets. For lookups like "every message where
//! `35=D`" we'd otherwise scan-and-evaluate every tag — fine at ~300 MiB/s but wasteful
//! when the same file is queried repeatedly.
//!
//! The secondary index keeps a map keyed on `(tag, value)` whose values are the ordinals
//! (indices into `LogIndex.messages`) of every message where that pair appears. Only the
//! tags named in [`HotTags`] are indexed — tagging everything would balloon memory with
//! tag values we almost never filter on (like `52` / SendingTime).
//!
//! # Storage
//!
//! - Key: `(u32 tag, SmallVec<[u8; 16]>)`. Most enum values and CompID strings fit in 16
//!   bytes so the key stays stack-only and cheap to hash.
//! - Value: `Vec<u32>` ordinals, *already sorted* because we insert as we iterate.
//!
//! `Vec<u32>` vs `roaring::RoaringBitmap`: the roaring representation is more compact for
//! very dense sets, but `Vec<u32>` is ~2× faster to iterate and doesn't pull in a new
//! dependency. We can swap it later if memory measurements justify it (see
//! `docs/PHASE2_PLAN.md` P2-T03 for the decision log).

use smallvec::SmallVec;
use std::collections::HashMap;

use fixlog_parser::{
    RawMessage, TAG_MSG_SEQ_NUM, TAG_MSG_TYPE, TAG_SENDER_COMP_ID, TAG_TARGET_COMP_ID,
};

/// Tag `11` — ClOrdID. Client order identifier.
pub const TAG_CL_ORD_ID: u32 = 11;
/// Tag `37` — OrderID. Exchange-assigned order identifier.
pub const TAG_ORDER_ID: u32 = 37;
/// Tag `41` — OrigClOrdID. Cancel/replace requests point here at the
/// original ClOrdID they target, which is how order-lifecycle queries
/// follow the chain once a new ClOrdID is issued.
pub const TAG_ORIG_CL_ORD_ID: u32 = 41;

/// Stack-allocated byte value for the secondary key. 16 bytes cover most enum/CompID
/// values while still hashing cheaply. Longer values still work — they just heap-allocate.
pub type Value = SmallVec<[u8; 16]>;

/// The set of tags the secondary index will materialize entries for.
///
/// The default set is a compromise between usefulness and cost: MsgType (`35`) is
/// filter-worthy in every log; the CompID pair identifies sessions; `34`/`11`/`37`
/// support sequence-number and order-lifecycle queries. Use `empty()` + `with(...)` for
/// custom sets.
#[derive(Debug, Clone)]
pub struct HotTags(SmallVec<[u32; 8]>);

impl HotTags {
    /// Standard set: `35, 49, 56, 11, 34, 37, 41`.
    pub fn default_set() -> Self {
        Self(SmallVec::from_slice(&[
            TAG_MSG_TYPE,
            TAG_SENDER_COMP_ID,
            TAG_TARGET_COMP_ID,
            TAG_CL_ORD_ID,
            TAG_MSG_SEQ_NUM,
            TAG_ORDER_ID,
            TAG_ORIG_CL_ORD_ID,
        ]))
    }

    /// An empty set — no secondary indexing will happen. Equivalent to skipping the
    /// secondary index entirely but useful as a starting point for `with(...)` calls.
    pub fn empty() -> Self {
        Self(SmallVec::new())
    }

    /// Add `tag` to the set. Duplicates are deduplicated.
    pub fn with(mut self, tag: u32) -> Self {
        if !self.0.contains(&tag) {
            self.0.push(tag);
        }
        self
    }

    /// Does this set contain `tag`?
    #[inline]
    pub fn contains(&self, tag: u32) -> bool {
        self.0.contains(&tag)
    }

    /// Number of tags in the set. A set of size zero disables the secondary index.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Is this set empty (→ no secondary indexing)?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterator over the tag numbers in the set.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().copied()
    }
}

impl Default for HotTags {
    fn default() -> Self {
        Self::default_set()
    }
}

/// Secondary lookup: `(tag, value) → Vec<ordinal>`.
///
/// Ordinals are indices into `LogIndex.messages`, always monotonic because entries are
/// appended as the builder walks the buffer.
#[derive(Debug, Default, Clone)]
pub struct SecondaryIndex {
    tags: HotTags,
    by_tag_value: HashMap<(u32, Value), Vec<u32>>,
}

impl SecondaryIndex {
    /// Empty index for a given `tags` set. Use [`Self::record`] per message to populate.
    pub fn with_tags(tags: HotTags) -> Self {
        Self {
            tags,
            by_tag_value: HashMap::new(),
        }
    }

    /// Which tags this index materializes entries for.
    pub fn hot_tags(&self) -> &HotTags {
        &self.tags
    }

    /// Record every hot-tag occurrence in `msg` against ordinal `ordinal`.
    ///
    /// If a hot tag appears multiple times in the same message (repeating groups, for
    /// instance), every occurrence is recorded. Lookups therefore answer "which messages
    /// mention this `(tag, value)` at least once" — which matches `fixlog grep` semantics.
    pub fn record(&mut self, msg: &RawMessage<'_>, ordinal: u32) {
        if self.tags.is_empty() {
            return;
        }
        for (tag, value) in &msg.tags {
            if !self.tags.contains(*tag) {
                continue;
            }
            let key = (*tag, Value::from_slice(value));
            let entry = self.by_tag_value.entry(key).or_default();
            // Append-only → ordinals are already monotonic. Skip duplicate within the same
            // message (happens for `35` on copy-paste-style logs but mostly for repeating
            // groups where both occurrences carry the same value).
            if entry.last().copied() != Some(ordinal) {
                entry.push(ordinal);
            }
        }
    }

    /// All ordinals for `(tag, value)`. Returns an empty slice if the tag is not hot or
    /// the value has never been seen.
    /// Whether `tag` is in the configured hot-tag set (and therefore
    /// indexed). Equivalent to `self.hot_tags().contains(tag)` — exposed
    /// as a convenience because consumers checking pushdown-eligibility
    /// shouldn't need to know the underlying `HotTags` API shape.
    #[inline]
    pub fn has_tag(&self, tag: u32) -> bool {
        self.tags.contains(tag)
    }

    pub fn lookup(&self, tag: u32, value: &[u8]) -> &[u32] {
        let key = (tag, Value::from_slice(value));
        self.by_tag_value
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Total number of `(tag, value)` keys materialized. Informational only.
    pub fn key_count(&self) -> usize {
        self.by_tag_value.len()
    }

    /// Total number of (ordinal) entries stored across all keys. Informational only.
    pub fn entry_count(&self) -> usize {
        self.by_tag_value.values().map(Vec::len).sum()
    }

    /// Clear the map but preserve the configured hot tags. Used on rebuilds after file
    /// truncation.
    pub fn clear(&mut self) {
        self.by_tag_value.clear();
    }

    /// Merge `other` into `self`, adding `base` to every ordinal from `other`.
    ///
    /// This is the primitive the parallel builder uses to stitch per-chunk partial
    /// secondaries into a single map. Callers must ensure the hot-tag sets match; if they
    /// don't, keys from `other` under a non-shared tag still merge verbatim, but
    /// consistency of the final secondary is the caller's problem.
    pub fn merge_rebased(&mut self, other: SecondaryIndex, base: u32) {
        for (key, ordinals) in other.by_tag_value {
            let entry = self.by_tag_value.entry(key).or_default();
            entry.reserve(ordinals.len());
            for ord in ordinals {
                let rebased = ord.saturating_add(base);
                if entry.last().copied() != Some(rebased) {
                    entry.push(rebased);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixlog_parser::{parse_one, validator};

    fn make_msg(msg_type: &str, extra: &[u8]) -> Vec<u8> {
        let mut body = format!("35={msg_type}\x0149=S\x0156=T\x01").into_bytes();
        body.extend_from_slice(extra);
        let prefix = format!("8=FIX.4.4\x019={}\x01", body.len());
        let mut msg: Vec<u8> = prefix.into_bytes();
        msg.extend_from_slice(&body);
        let cs = validator::compute_checksum(&msg);
        msg.extend_from_slice(format!("10={cs:03}\x01").as_bytes());
        msg
    }

    #[test]
    fn default_set_includes_msg_type_and_compids() {
        let tags = HotTags::default_set();
        assert!(tags.contains(TAG_MSG_TYPE));
        assert!(tags.contains(TAG_SENDER_COMP_ID));
        assert!(tags.contains(TAG_TARGET_COMP_ID));
    }

    #[test]
    fn empty_set_skips_recording() {
        let bytes = make_msg("D", b"");
        let (msg, _) = parse_one(&bytes).unwrap();
        let mut idx = SecondaryIndex::with_tags(HotTags::empty());
        idx.record(&msg, 0);
        assert_eq!(idx.key_count(), 0);
    }

    #[test]
    fn lookup_returns_ordinals_for_hot_tag() {
        let bytes = make_msg("D", b"");
        let (msg, _) = parse_one(&bytes).unwrap();
        let mut idx = SecondaryIndex::with_tags(HotTags::default_set());
        idx.record(&msg, 7);
        assert_eq!(idx.lookup(TAG_MSG_TYPE, b"D"), &[7]);
        assert_eq!(idx.lookup(TAG_SENDER_COMP_ID, b"S"), &[7]);
        assert_eq!(idx.lookup(TAG_MSG_TYPE, b"A"), &[] as &[u32]);
    }

    #[test]
    fn ordinals_are_appended_monotonically_across_messages() {
        let m1 = make_msg("D", b"");
        let m2 = make_msg("D", b"");
        let m3 = make_msg("8", b"");
        let mut idx = SecondaryIndex::with_tags(HotTags::default_set());
        idx.record(&parse_one(&m1).unwrap().0, 0);
        idx.record(&parse_one(&m2).unwrap().0, 1);
        idx.record(&parse_one(&m3).unwrap().0, 2);
        assert_eq!(idx.lookup(TAG_MSG_TYPE, b"D"), &[0, 1]);
        assert_eq!(idx.lookup(TAG_MSG_TYPE, b"8"), &[2]);
    }

    #[test]
    fn repeating_same_value_within_a_message_records_once() {
        // 448=BROKER1 twice — the secondary should record the ordinal once, not twice.
        let bytes = make_msg("D", b"11=ORD1\x0111=ORD1\x01");
        let (msg, _) = parse_one(&bytes).unwrap();
        let mut idx = SecondaryIndex::with_tags(HotTags::default_set());
        idx.record(&msg, 5);
        assert_eq!(idx.lookup(TAG_CL_ORD_ID, b"ORD1"), &[5]);
    }

    #[test]
    fn non_hot_tags_are_ignored() {
        let bytes = make_msg("D", b"55=AAPL\x01");
        let (msg, _) = parse_one(&bytes).unwrap();
        let mut idx = SecondaryIndex::with_tags(HotTags::default_set());
        idx.record(&msg, 0);
        // 55 (Symbol) is not in the default hot set.
        assert_eq!(idx.lookup(55, b"AAPL"), &[] as &[u32]);
    }

    #[test]
    fn with_adds_and_deduplicates_tags() {
        let tags = HotTags::empty().with(55).with(55).with(1128);
        assert_eq!(tags.len(), 2);
        assert!(tags.contains(55));
        assert!(tags.contains(1128));
    }
}
