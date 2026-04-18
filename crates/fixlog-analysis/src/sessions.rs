//! Session tracking: aggregate by (SenderCompID, TargetCompID) with
//! per-MsgType counts and MsgSeqNum gap detection.
//!
//! Keys store owned `Vec<u8>` (not `&[u8]`) because a `SessionMap`
//! outlives the mmap under `--follow`: the buffer may be re-mapped
//! between frames, so anything retained must not borrow from it.

use std::collections::HashMap;

use fixlog_core::{LogFormat, LogIndex};
use smallvec::SmallVec;

/// Session identity: the `(49, 56)` pair.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SessionKey {
    pub sender: Vec<u8>,
    pub target: Vec<u8>,
}

/// A MsgSeqNum gap detected inside a single session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeqGap {
    pub from_seq: u32,
    pub to_seq: u32,
    pub ordinal_before: u32,
    pub ordinal_after: u32,
}

/// Per-session aggregated metrics.
#[derive(Clone, Debug, Default)]
pub struct SessionStats {
    pub in_count: u32,
    pub out_count: u32,
    pub by_msg_type: HashMap<SmallVec<[u8; 2]>, u32>,
    pub seq_min: Option<u32>,
    pub seq_max: Option<u32>,
    pub gaps: Vec<SeqGap>,
    pub ordinals: Vec<u32>,
}

/// Aggregate view of sessions observed in a log.
#[derive(Clone, Debug, Default)]
pub struct SessionMap {
    pub by_key: HashMap<SessionKey, SessionStats>,
    pub by_ordinal: Vec<SessionKey>,
}

impl SessionMap {
    /// Full build pass over `index.messages`.
    pub fn build(_index: &LogIndex, _buf: &[u8], _format: &LogFormat) -> Self {
        todo!("P4-T02")
    }

    /// Incremental append starting from `from_ordinal`. Used under `--follow`.
    pub fn append_from(
        &mut self,
        _index: &LogIndex,
        _buf: &[u8],
        _format: &LogFormat,
        _from_ordinal: u32,
    ) {
        todo!("P4-T02")
    }
}
