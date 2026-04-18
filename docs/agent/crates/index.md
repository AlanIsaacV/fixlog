# `fixlog-index`

Offset-based index over a FIX log buffer, with append-friendly growth and a secondary
lookup map over hot tags. Lives at `crates/fixlog-index/`.

## Public surface

- `pub struct MessageOffset { start: u64, len: u32 }` — absolute offset + byte length.
- `pub struct LogIndex { messages: Vec<MessageOffset>, consumed: u64, secondary: SecondaryIndex }`.
- `pub fn build_from_bytes(buf, fmt) -> LogIndex` — single-thread full build, default hot tags.
- `pub fn build_from_bytes_with_hot_tags(buf, fmt, tags) -> LogIndex` — same, custom tags.
- `pub fn build_from_bytes_parallel(buf, fmt) -> LogIndex` — rayon-chunked build.
- `pub fn build_from_bytes_parallel_with_hot_tags(...)` — parallel + custom tags.
- `LogIndex::append_from_offset(buf, from, fmt) -> Result<usize, IndexError>` — delta ingest.
- `LogIndex::message_bytes(buf, idx) -> Option<&[u8]>` — borrow the bytes of message `idx`.
- `HotTags { default_set(), empty(), with(tag) }` + `SecondaryIndex::lookup(tag, value) -> &[u32]`.

## INVARIANTs

- `LogIndex.consumed` is the absolute byte offset **immediately past the last successfully
  indexed message**. It is *not* `buf.len()`. Trailing partial messages are intentionally
  left unclaimed so the next `append_from_offset` re-scans them from their true start.
- `append_from_offset(buf, from)` requires `from == self.consumed`. Violations return
  `IndexError::NonContiguousAppend` without mutating the index.
- If `buf.len() < self.consumed`, the call returns `IndexError::BufferShrank` — the caller
  must rebuild from scratch (this is the logrotate/truncation path).
- Secondary ordinals are indices into `messages`, monotonic, deduplicated within one
  message (a tag appearing twice with the same value records the ordinal once).

## REALITY (vs `ARCHITECTURE.md`)

- `secondary` is `HashMap<(u32, SmallVec<[u8;16]>), Vec<u32>>`, *not* `RoaringBitmap`. The
  bitmap gives denser storage for huge files but adds a dep and is slower to iterate.
  We'll switch only when memory measurements justify it.
- The parallel builder uses a per-chunk `SecondaryIndex` with local ordinals, then
  `merge_rebased` stitches them. Same final shape as the single-thread path.

## Performance (from `cargo bench -p fixlog-index --bench index`, Darwin 25.3.0)

| Bench | Throughput |
|-------|-----------|
| `index_real/single_thread/fix44_om` | ~203 MiB/s |
| `index_real/parallel/fix44_om`       | ~492 MiB/s (**2.4×**) |
| `index_real/single_thread/fixt11_md` | ~730 MiB/s |
| `index_real/parallel/fixt11_md`      | ~2.2 GiB/s (**3.0×**) |
| `index_amplified/single_thread_40MiB`| ~211 MiB/s |
| `index_amplified/parallel_40MiB`     | ~1.08 GiB/s (**5.1×**) |

Buffers below 1 MiB auto-fall-back to the single-thread path. Chunk size floor is 512 KiB.

## Tests

- 24 unit (builder, secondary, parallel).
- 4 integration in `tests/real_fixtures.rs` — must agree with the parser's count on the
  real fixtures and back-match every secondary ordinal to a primary message.

## When to modify this crate

- **Add a new hot-tag default**: change `HotTags::default_set()` in `secondary.rs`.
- **Change the secondary representation**: start in `secondary.rs`; keep the `lookup/record/
  merge_rebased` trio intact so the parallel builder keeps working.
- **Tune parallel chunk size**: constants at the top of `parallel.rs`
  (`MIN_BUF_SIZE_FOR_PARALLEL`, `MIN_CHUNK_SIZE`). Re-run the bench after any change.
