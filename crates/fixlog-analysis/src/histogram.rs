//! Temporal histogram over SendingTime (tag 52): bins of uniform
//! width (`bucket_ns`) with per-bin message counts.

use std::time::Duration;

use fixlog_core::{LogFormat, LogIndex};

/// One uniform-width time bucket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bin {
    pub start_ns: u128,
    pub end_ns: u128,
    pub count: u32,
}

/// Uniform-width histogram of message timestamps.
#[derive(Clone, Debug, Default)]
pub struct Histogram {
    pub bucket_ns: u64,
    pub bins: Vec<Bin>,
}

impl Histogram {
    /// Single-pass histogram over `index.messages`.
    pub fn build(_index: &LogIndex, _buf: &[u8], _format: &LogFormat, _bucket: Duration) -> Self {
        todo!("P4-T04")
    }
}
