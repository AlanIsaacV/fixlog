//! Order lifecycle reconstruction: given a ClOrdID (tag 11), walk the
//! index (including OrderID=37 cross-references) to build a timeline
//! of events ordered by ordinal.
//!
//! Timeline fields are owned to survive mmap re-mappings under `--follow`.

use std::time::SystemTime;

use fixlog_core::{LogFormat, LogIndex};
use smallvec::SmallVec;

/// One event (message) in the lifetime of an order.
#[derive(Clone, Debug)]
pub struct OrderEvent {
    pub ordinal: u32,
    pub msg_type: SmallVec<[u8; 2]>,
    pub sending_time: Option<SystemTime>,
    pub exec_type: Option<SmallVec<[u8; 2]>>,
    pub ord_status: Option<SmallVec<[u8; 2]>>,
    pub cum_qty: Option<SmallVec<[u8; 16]>>,
}

/// Full ordered timeline for a single ClOrdID.
#[derive(Clone, Debug)]
pub struct OrderTimeline {
    pub clordid: Vec<u8>,
    pub order_ids: SmallVec<[Vec<u8>; 2]>,
    pub events: Vec<OrderEvent>,
}

impl OrderTimeline {
    /// Build the timeline for `clordid`. Returns `None` if no message
    /// with `11=<clordid>` exists in the index.
    pub fn build(
        _index: &LogIndex,
        _buf: &[u8],
        _format: &LogFormat,
        _clordid: &[u8],
    ) -> Option<Self> {
        todo!("P4-T03")
    }
}
