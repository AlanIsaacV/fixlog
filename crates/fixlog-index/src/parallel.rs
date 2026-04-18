//! Parallel index construction using rayon.
//!
//! # Algorithm
//!
//! The buffer is split into roughly equal chunks, one per worker. Each worker owns the
//! messages whose **start offset** falls inside its chunk range `[chunk_start, chunk_end)`,
//! even if the trailing bytes of its last message spill past `chunk_end`. The next
//! worker's range begins exactly at that `chunk_end`, so it will naturally scan forward
//! to the first `8=FIX` marker at or after that boundary — which is always *after* the
//! end of the previous worker's last message (since that message started before
//! `chunk_end`). No double counting, no lost messages.
//!
//! # Why it scales
//!
//! The parser is throughput-bound on memchr scans. Splitting the buffer lets every core
//! run its own memchr, and since `RawMessage::raw` borrows from the source buffer
//! there's no cross-thread contention — each worker only mutates its local state and
//! the results are stitched afterward.
//!
//! # When to call
//!
//! Small buffers (< a few MiB) don't benefit: thread-pool dispatch and merge overhead
//! dominate. [`build_from_bytes_parallel`] falls back to the single-threaded path below
//! that threshold. Everything above behaves identically to [`crate::build_from_bytes`]
//! modulo small heap-allocation differences.

use crate::{HotTags, LogIndex, MessageOffset, SecondaryIndex, build_from_bytes_with_hot_tags};
use fixlog_format::LogFormat;
use fixlog_parser::parse_all_with_format;
use rayon::prelude::*;

/// Minimum buffer size to bother with rayon. Below this, the single-thread path is
/// faster because of dispatch overhead.
const MIN_BUF_SIZE_FOR_PARALLEL: usize = 1_024 * 1_024;

/// Minimum chunk size: below this, too many short tasks hurt cache and dispatch.
const MIN_CHUNK_SIZE: usize = 512 * 1_024;

/// Parallel build using rayon + the default hot-tag set.
pub fn build_from_bytes_parallel(buf: &[u8], format: &LogFormat) -> LogIndex {
    build_from_bytes_parallel_with_hot_tags(buf, format, HotTags::default_set())
}

/// Parallel build using rayon with a caller-supplied hot-tag set.
pub fn build_from_bytes_parallel_with_hot_tags(
    buf: &[u8],
    format: &LogFormat,
    tags: HotTags,
) -> LogIndex {
    let threads = rayon::current_num_threads().max(1);
    if threads == 1 || buf.len() < MIN_BUF_SIZE_FOR_PARALLEL {
        return build_from_bytes_with_hot_tags(buf, format, tags);
    }
    let ranges = chunk_ranges(buf.len(), threads);
    let partials: Vec<ChunkResult> = ranges
        .par_iter()
        .map(|&(start, end)| scan_chunk(buf, start, end, format, tags.clone()))
        .collect();
    stitch(partials, tags)
}

/// Output of a single worker: absolute-offset messages that started in its chunk, plus
/// a secondary index with ordinals local to the chunk (`0..offsets.len()`).
struct ChunkResult {
    offsets: Vec<MessageOffset>,
    secondary: SecondaryIndex,
}

/// Scan the region of `buf` owned by a worker: messages whose start is in `[start, end)`.
///
/// The parser is fed `&buf[start..]` because it scans for `8=FIX` from the *start* of the
/// slice it receives. We translate its message offsets back into absolute terms and stop
/// as soon as a message would start at or past `end`, since that message belongs to the
/// next worker.
fn scan_chunk(
    buf: &[u8],
    start: usize,
    end: usize,
    format: &LogFormat,
    tags: HotTags,
) -> ChunkResult {
    let mut offsets = Vec::new();
    let mut secondary = SecondaryIndex::with_tags(tags);
    if start >= buf.len() {
        return ChunkResult { offsets, secondary };
    }
    let tail = &buf[start..];
    for msg in parse_all_with_format(tail, format).flatten() {
        let abs_start = start as u64 + msg.offset;
        if abs_start >= end as u64 {
            break;
        }
        let len = u32::try_from(msg.raw.len()).unwrap_or(u32::MAX);
        let ordinal = u32::try_from(offsets.len()).unwrap_or(u32::MAX);
        secondary.record(&msg, ordinal);
        offsets.push(MessageOffset {
            start: abs_start,
            len,
        });
    }
    ChunkResult { offsets, secondary }
}

/// Merge chunk results in order. Per-chunk ordinals are rebased by the running total.
fn stitch(partials: Vec<ChunkResult>, tags: HotTags) -> LogIndex {
    let total: usize = partials.iter().map(|p| p.offsets.len()).sum();
    let mut messages = Vec::with_capacity(total);
    let mut secondary = SecondaryIndex::with_tags(tags);
    let mut base: u32 = 0;
    for part in partials {
        let added = part.offsets.len();
        secondary.merge_rebased(part.secondary, base);
        messages.extend(part.offsets);
        base = base.saturating_add(u32::try_from(added).unwrap_or(u32::MAX));
    }
    let consumed = messages.last().map(MessageOffset::end).unwrap_or(0);
    LogIndex {
        messages,
        consumed,
        secondary,
    }
}

/// Divide `[0, buf_len)` into at most `threads` contiguous half-open ranges with at least
/// `MIN_CHUNK_SIZE` bytes each (except possibly the last one, which can be shorter).
fn chunk_ranges(buf_len: usize, threads: usize) -> Vec<(usize, usize)> {
    debug_assert!(threads >= 1);
    if buf_len == 0 {
        return Vec::new();
    }
    let target = (buf_len / threads).max(MIN_CHUNK_SIZE);
    let mut ranges = Vec::with_capacity(threads);
    let mut cursor = 0usize;
    while cursor < buf_len {
        let end = cursor.saturating_add(target).min(buf_len);
        ranges.push((cursor, end));
        cursor = end;
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_from_bytes, build_from_bytes_with_hot_tags};
    use fixlog_format::sniff;

    const MINIMAL: &[u8] = include_bytes!("../../../fixtures/synthetic/minimal_4.4.log");
    const FIX44_OM: &[u8] = include_bytes!("../../../fixtures/real/fix44-om.log");
    const FIXT11_MD: &[u8] = include_bytes!("../../../fixtures/real/fixt11-md.log");

    fn amplify(buf: &[u8], target: usize) -> Vec<u8> {
        // Concatenating the same log repeatedly is safe: the parser is prefix-agnostic
        // and messages already self-delimit via BodyLength + CheckSum.
        let copies = target.div_ceil(buf.len().max(1));
        let mut out = Vec::with_capacity(copies * buf.len());
        for _ in 0..copies {
            out.extend_from_slice(buf);
        }
        out
    }

    #[test]
    fn parallel_matches_single_thread_on_small_buffer() {
        // Below MIN_BUF_SIZE the parallel path falls back to single-thread; this test
        // guarantees the fallback is bit-identical.
        let fmt = sniff(MINIMAL).expect("sniffable");
        let single = build_from_bytes(MINIMAL, &fmt);
        let parallel = build_from_bytes_parallel(MINIMAL, &fmt);
        assert_eq!(single.messages, parallel.messages);
        assert_eq!(single.consumed, parallel.consumed);
    }

    #[test]
    fn parallel_matches_single_thread_on_real_fix44() {
        // Real fixture is 2 MiB; with default rayon threads we cross the parallel
        // threshold on most machines. The two outputs must be identical.
        let fmt = sniff(FIX44_OM).expect("sniffable");
        let single = build_from_bytes(FIX44_OM, &fmt);
        let parallel = build_from_bytes_parallel(FIX44_OM, &fmt);
        assert_eq!(single.messages, parallel.messages);
        assert_eq!(single.consumed, parallel.consumed);
        assert_eq!(
            single.secondary.key_count(),
            parallel.secondary.key_count(),
            "secondary key set must match"
        );
    }

    #[test]
    fn parallel_matches_single_thread_on_real_fixt11() {
        let fmt = sniff(FIXT11_MD).expect("sniffable");
        let single = build_from_bytes(FIXT11_MD, &fmt);
        let parallel = build_from_bytes_parallel(FIXT11_MD, &fmt);
        assert_eq!(single.messages, parallel.messages);
        assert_eq!(single.consumed, parallel.consumed);
        // Spot-check a few secondary lookups.
        for key in [(35u32, &b"W"[..]), (35, b"X"), (49, b"LMAX")] {
            assert_eq!(
                single.secondary.lookup(key.0, key.1),
                parallel.secondary.lookup(key.0, key.1),
                "secondary mismatch for key {key:?}"
            );
        }
    }

    #[test]
    fn parallel_matches_single_thread_on_amplified_buffer() {
        // 10× amplified fixture crosses MIN_CHUNK_SIZE for every reasonable thread count.
        // This is the scenario the parallel path is actually *for*, so the equivalence
        // test here has the highest real-world signal.
        let fmt = sniff(FIX44_OM).expect("sniffable");
        let amp = amplify(FIX44_OM, 20 * 1024 * 1024);
        let single = build_from_bytes(&amp, &fmt);
        let parallel = build_from_bytes_parallel(&amp, &fmt);
        assert_eq!(single.len(), parallel.len());
        assert_eq!(single.messages, parallel.messages);
        assert_eq!(single.consumed, parallel.consumed);
    }

    #[test]
    fn custom_hot_tags_pass_through_to_parallel_workers() {
        let fmt = sniff(FIX44_OM).expect("sniffable");
        let tags = HotTags::empty().with(fixlog_parser::TAG_MSG_TYPE);
        let par = build_from_bytes_parallel_with_hot_tags(FIX44_OM, &fmt, tags.clone());
        let seq = build_from_bytes_with_hot_tags(FIX44_OM, &fmt, tags);
        assert_eq!(par.secondary.key_count(), seq.secondary.key_count());
        // No other tag should be present: only MsgType was requested.
        assert_eq!(par.secondary.lookup(49, b"BVLORSG"), &[] as &[u32]);
    }

    #[test]
    fn chunk_ranges_cover_buffer_exactly() {
        let ranges = chunk_ranges(10_000_000, 4);
        assert!(!ranges.is_empty());
        assert_eq!(ranges[0].0, 0);
        assert_eq!(ranges.last().unwrap().1, 10_000_000);
        for w in ranges.windows(2) {
            assert_eq!(w[0].1, w[1].0, "ranges must be contiguous");
        }
    }
}
