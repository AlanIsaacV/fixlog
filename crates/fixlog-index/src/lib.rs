#![forbid(unsafe_code)]

//! Offset-based index over a FIX log buffer.
//!
//! The index records `(start, len)` for every well-formed message so that callers can:
//! - jump to any message by ordinal without re-scanning the prefix,
//! - count messages without re-parsing,
//! - grow the index incrementally as a tailed file extends past `file_size`.
//!
//! The primary index is deliberately **content-free**: it stores offsets only. Lookups
//! resolve to `&buf[start..start+len]` and re-parse lazily with `fixlog_parser`. This
//! keeps memory cost bounded at ~12 bytes/message regardless of message size.
//!
//! A secondary lookup map over a configurable set of "hot" tags (MsgType, CompIDs, order
//! IDs) rides alongside the primary index. See [`secondary`] for details; the default
//! set is enabled automatically in [`build_from_bytes`].

pub mod builder;
pub mod parallel;
pub mod secondary;

pub use builder::{build_from_bytes, build_from_bytes_with_hot_tags};
pub use parallel::{build_from_bytes_parallel, build_from_bytes_parallel_with_hot_tags};
pub use secondary::{HotTags, SecondaryIndex};

/// Position of a single message within the source buffer.
///
/// `start` is the absolute byte offset of the `8=` BeginString; `len` is the number of bytes
/// up to and including the trailing separator after the CheckSum field, matching
/// `RawMessage::raw.len()` from the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageOffset {
    pub start: u64,
    pub len: u32,
}

impl MessageOffset {
    /// Byte range `start..start+len` suitable for `&buf[range]`.
    #[inline]
    pub fn range(&self) -> std::ops::Range<usize> {
        let s = self.start as usize;
        s..s + self.len as usize
    }

    /// Absolute offset of the byte *after* this message. Useful as the seed for
    /// [`LogIndex::append_from_offset`] when processing a delta from a tailed file.
    #[inline]
    pub fn end(&self) -> u64 {
        self.start + self.len as u64
    }
}

/// Primary index over a FIX log, with an optional secondary lookup map.
///
/// Shape: one `MessageOffset` per parsed message, plus the high-water mark of the source
/// buffer (`consumed`), plus a [`SecondaryIndex`] materialized over a configurable set
/// of hot tags.
#[derive(Debug, Default, Clone)]
pub struct LogIndex {
    /// Ordered offsets, one per successfully parsed message. Monotonic in `start`.
    pub messages: Vec<MessageOffset>,
    /// Absolute byte offset immediately past the last *successfully indexed* message.
    ///
    /// Any trailing bytes between `consumed` and the real EOF (partial message, padding,
    /// logrotate marker) are intentionally not claimed — they will be re-scanned on the
    /// next call to [`Self::append_from_offset`]. That is what makes the append path
    /// robust to partial writes from a live producer.
    pub consumed: u64,
    /// Reverse map `(tag, value) → ordinals`. Populated only for hot tags; see
    /// [`SecondaryIndex`] and [`HotTags`]. Empty if the caller requested no secondary
    /// indexing (e.g. `HotTags::empty()`).
    pub secondary: SecondaryIndex,
}

impl LogIndex {
    /// Create an empty index with no hot tags. Prefer [`build_from_bytes`] for the common
    /// case; use this only when you will populate it yourself (tests, fuzzing).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty index with a pre-configured hot-tag set for the secondary index.
    /// Use this when you want to populate the index incrementally (e.g. for a tailing
    /// reader) and still get lookup support.
    pub fn with_hot_tags(tags: HotTags) -> Self {
        Self {
            messages: Vec::new(),
            consumed: 0,
            secondary: SecondaryIndex::with_tags(tags),
        }
    }

    /// Number of indexed messages.
    #[inline]
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the index has no messages recorded yet.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Borrow the raw bytes of the message at ordinal `idx` from `buf`.
    ///
    /// Returns `None` if `idx` is out of range or the offset escapes the buffer
    /// (which indicates the caller is using a shorter buffer than the one used to
    /// build the index — usually a bug).
    pub fn message_bytes<'a>(&self, buf: &'a [u8], idx: usize) -> Option<&'a [u8]> {
        let off = self.messages.get(idx)?;
        let range = off.range();
        buf.get(range)
    }
}

/// Errors produced by the index builder.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IndexError {
    /// `append_from_offset` got a `from` that does not line up with `consumed`.
    #[error("non-contiguous append: consumed={consumed} but got from={from}")]
    NonContiguousAppend { consumed: u64, from: u64 },
    /// Caller passed a buffer shorter than what had already been indexed. This is almost
    /// always a logrotate/truncation that requires rebuilding from scratch.
    #[error("buffer shrank: consumed={consumed} but buf.len()={buf_len}")]
    BufferShrank { consumed: u64, buf_len: u64 },
}
