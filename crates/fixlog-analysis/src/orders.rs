//! Order lifecycle reconstruction.
//!
//! Given a `ClOrdID` (tag 11), [`OrderTimeline::build`] walks the
//! [`LogIndex`] secondary map:
//!
//! 1. `secondary.lookup(11, clordid)` → initial ordinals.
//! 2. `secondary.lookup(41, clordid)` → messages referencing `clordid`
//!    as `OrigClOrdID` (cancel/replace requests). For each such message,
//!    read its own tag 11 and feed it back into the worklist so the
//!    whole replacement chain is followed to fixed point.
//! 3. Re-parse the union from steps 1–2 to collect the `OrderID`
//!    (tag 37) values observed.
//! 4. For each observed `OrderID`, `secondary.lookup(37, ord_id)` expands
//!    the set to include execution reports that only reference `37`
//!    (common after the first ack).
//! 5. Dedup, sort ascending (ordinals are monotonic in time), and
//!    materialize [`OrderEvent`]s.
//!
//! # Ownership
//!
//! Timeline fields are owned (`Vec<u8>`, `SmallVec<[u8; N]>`) so the
//! timeline survives an `Arc<Mmap>` swap under `--follow`. This is the
//! same invariant as `ResolvedMessageOwned` in the TUI.

use std::collections::HashSet;
use std::time::SystemTime;

use fixlog_core::index::secondary::{TAG_CL_ORD_ID, TAG_ORDER_ID, TAG_ORIG_CL_ORD_ID};
use fixlog_core::parser::{TAG_MSG_TYPE, TAG_SENDING_TIME};
use fixlog_core::{LogFormat, LogIndex, parse_one_with_format};
use smallvec::SmallVec;

use crate::util::{find_tag, parse_sending_time};

/// Tag `6` — AvgPx. Volume-weighted average price across all fills for
/// this order so far. Reported on each ExecutionReport.
pub const TAG_AVG_PX: u32 = 6;
/// Tag `14` — CumQty.
pub const TAG_CUM_QTY: u32 = 14;
/// Tag `31` — LastPx. Execution price of the last fill in the current
/// ExecutionReport. Absent on pending/new/cancel events.
pub const TAG_LAST_PX: u32 = 31;
/// Tag `32` — LastQty. Quantity of the last fill in the current
/// ExecutionReport. Absent on pending/new/cancel events.
pub const TAG_LAST_QTY: u32 = 32;
/// Tag `39` — OrdStatus.
pub const TAG_ORD_STATUS: u32 = 39;
/// Tag `150` — ExecType.
pub const TAG_EXEC_TYPE: u32 = 150;

/// One event (message) in the lifetime of an order. The `last_*` fields
/// are per-fill (this ExecutionReport only); `cum_qty` and `avg_px` are
/// cumulative across the order's fills.
#[derive(Clone, Debug)]
pub struct OrderEvent {
    pub ordinal: u32,
    pub msg_type: SmallVec<[u8; 2]>,
    pub sending_time: Option<SystemTime>,
    pub exec_type: Option<SmallVec<[u8; 2]>>,
    pub ord_status: Option<SmallVec<[u8; 2]>>,
    pub last_qty: Option<SmallVec<[u8; 16]>>,
    pub last_px: Option<SmallVec<[u8; 16]>>,
    pub cum_qty: Option<SmallVec<[u8; 16]>>,
    pub avg_px: Option<SmallVec<[u8; 16]>>,
}

/// Full ordered timeline for a single ClOrdID chain.
#[derive(Clone, Debug)]
pub struct OrderTimeline {
    pub clordid: Vec<u8>,
    pub order_ids: SmallVec<[Vec<u8>; 2]>,
    pub events: Vec<OrderEvent>,
}

impl OrderTimeline {
    /// Reconstruct the timeline for `clordid`. Returns `None` if the
    /// given id is never referenced — neither as a tag 11 nor as a tag 41
    /// (OrigClOrdID) — anywhere in the index.
    pub fn build(index: &LogIndex, buf: &[u8], format: &LogFormat, clordid: &[u8]) -> Option<Self> {
        // Worklist over the family of ClOrdIDs reachable from `clordid`
        // via `OrigClOrdID` (tag 41). A cancel/replace carries a fresh
        // `11=<new>` and points back at the predecessor through
        // `41=<old>`; iterating to fixed point collects the full chain.
        let mut ordinals_set: HashSet<u32> = HashSet::new();
        let mut cl_ids_seen: HashSet<Vec<u8>> = HashSet::new();
        let mut cl_ids_pending: Vec<Vec<u8>> = vec![clordid.to_vec()];

        while let Some(cid) = cl_ids_pending.pop() {
            if !cl_ids_seen.insert(cid.clone()) {
                continue;
            }
            // Direct: messages whose own ClOrdID is this id.
            let direct = index.secondary.lookup(TAG_CL_ORD_ID, &cid);
            ordinals_set.extend(direct.iter().copied());
            // Referencers: cancel/replace requests whose OrigClOrdID is
            // this id. Their own tag 11 is a new ClOrdID that extends the
            // chain; queue it for the next iteration.
            let referers = index.secondary.lookup(TAG_ORIG_CL_ORD_ID, &cid);
            ordinals_set.extend(referers.iter().copied());
            for &ord in referers {
                let Some(bytes) = index.message_bytes(buf, ord as usize) else {
                    continue;
                };
                let Ok((msg, _)) = parse_one_with_format(bytes, format) else {
                    continue;
                };
                if let Some(new_cl) = find_tag(&msg, TAG_CL_ORD_ID)
                    && !cl_ids_seen.contains(new_cl)
                {
                    cl_ids_pending.push(new_cl.to_vec());
                }
            }
        }

        if ordinals_set.is_empty() {
            return None;
        }

        // Expand via OrderID (tag 37) so execution reports that only
        // reference the exchange-assigned id are included.
        let mut order_ids_set: HashSet<Vec<u8>> = HashSet::new();
        for &ord in &ordinals_set {
            let Some(bytes) = index.message_bytes(buf, ord as usize) else {
                continue;
            };
            let Ok((msg, _)) = parse_one_with_format(bytes, format) else {
                continue;
            };
            if let Some(oid) = find_tag(&msg, TAG_ORDER_ID) {
                order_ids_set.insert(oid.to_vec());
            }
        }
        for oid in &order_ids_set {
            let expansion = index.secondary.lookup(TAG_ORDER_ID, oid);
            ordinals_set.extend(expansion.iter().copied());
        }

        let mut ordinals: Vec<u32> = ordinals_set.into_iter().collect();
        ordinals.sort_unstable();

        // Materialize the events.
        let mut events = Vec::with_capacity(ordinals.len());
        for ord in ordinals {
            let Some(bytes) = index.message_bytes(buf, ord as usize) else {
                continue;
            };
            let Ok((msg, _)) = parse_one_with_format(bytes, format) else {
                continue;
            };
            let msg_type = find_tag(&msg, TAG_MSG_TYPE)
                .map(SmallVec::from_slice)
                .unwrap_or_default();
            let sending_time = find_tag(&msg, TAG_SENDING_TIME).and_then(parse_sending_time);
            let exec_type = find_tag(&msg, TAG_EXEC_TYPE).map(SmallVec::from_slice);
            let ord_status = find_tag(&msg, TAG_ORD_STATUS).map(SmallVec::from_slice);
            let last_qty = find_tag(&msg, TAG_LAST_QTY).map(SmallVec::from_slice);
            let last_px = find_tag(&msg, TAG_LAST_PX).map(SmallVec::from_slice);
            let cum_qty = find_tag(&msg, TAG_CUM_QTY).map(SmallVec::from_slice);
            let avg_px = find_tag(&msg, TAG_AVG_PX).map(SmallVec::from_slice);

            events.push(OrderEvent {
                ordinal: ord,
                msg_type,
                sending_time,
                exec_type,
                ord_status,
                last_qty,
                last_px,
                cum_qty,
                avg_px,
            });
        }

        // Deterministic order for `order_ids` field (set iteration is
        // unordered). Sort by lex value; small enough that this is free.
        let mut order_ids: SmallVec<[Vec<u8>; 2]> = order_ids_set.into_iter().collect();
        order_ids.sort();

        Some(Self {
            clordid: clordid.to_vec(),
            order_ids,
            events,
        })
    }
}

/// Render a horizontal ASCII Gantt bar for a timeline.
///
/// Width is clamped to `[10, 500]`. Each event is placed at a character
/// column proportional to `(event.sending_time - first) / total_range`.
/// If the timeline has fewer than two events with timestamps, all chars
/// go to column 0 (best effort — a single point still wants to render).
///
/// Character mapping:
///
/// - `D` → `N` (NewOrderSingle)
/// - `8` → `X` (ExecutionReport)
/// - `F` → `C` (OrderCancelRequest)
/// - `G` → `R` (OrderCancelReplaceRequest)
/// - `3` / `j` → `!` (Reject / BusinessMessageReject)
/// - anything else → `?`
pub fn render_gantt(timeline: &OrderTimeline, width: usize) -> String {
    let width = width.clamp(10, 500);
    let mut row = vec![b'.'; width];

    let times: Vec<SystemTime> = timeline
        .events
        .iter()
        .filter_map(|e| e.sending_time)
        .collect();
    let (first, last) = match (times.first(), times.last()) {
        (Some(a), Some(b)) if a != b => (*a, *b),
        _ => {
            for e in &timeline.events {
                row[0] = gantt_char(&e.msg_type);
            }
            return String::from_utf8(row).unwrap_or_default();
        }
    };
    let total = last
        .duration_since(first)
        .map(|d| d.as_nanos())
        .unwrap_or(1)
        .max(1);

    for e in &timeline.events {
        let Some(t) = e.sending_time else {
            continue;
        };
        let offset = t.duration_since(first).map(|d| d.as_nanos()).unwrap_or(0);
        let col = ((offset * (width as u128 - 1)) / total) as usize;
        let col = col.min(width - 1);
        let ch = gantt_char(&e.msg_type);
        // Precedence: later events win the slot so the last status is
        // what you see. For a tied column, that's the desired behaviour
        // (you care about the outcome, not the intermediate state).
        row[col] = ch;
    }
    String::from_utf8(row).unwrap_or_default()
}

fn gantt_char(msg_type: &[u8]) -> u8 {
    match msg_type {
        b"D" => b'N',
        b"8" => b'X',
        b"F" => b'C',
        b"G" => b'R',
        b"3" | b"j" => b'!',
        _ => b'?',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixlog_core::{build_from_bytes, sniff};

    /// Build a FIX message with a well-formed checksum. `body_fields` must
    /// already be separator-joined (SOH between fields, trailing SOH).
    fn build_msg(body_fields: &str) -> Vec<u8> {
        let body_len = body_fields.len();
        let head = format!("8=FIX.4.4\x019={body_len}\x01");
        let payload: Vec<u8> = head.bytes().chain(body_fields.bytes()).collect();
        let sum: u8 = payload.iter().fold(0u8, |a, b| a.wrapping_add(*b));
        let trailer = format!("10={sum:03}\x01");
        payload.into_iter().chain(trailer.bytes()).collect()
    }

    /// Generate a synthetic log with two order lifecycles:
    ///
    /// - `ABC` → `XYZ` (cancel flow): D, 8(PendingNew), 8(New), F, 8(PendingCancel), 8(Cancelled)
    /// - `DEF` → `GHI` (replace flow): D, 8(New), G, 8(Replaced), 8(PartialFill), 8(Fill)
    fn synthetic_order_lifecycle() -> Vec<u8> {
        // All messages share one session (A→B).
        let tail = "49=A\x0156=B\x01";
        let t = |sec| format!("52=20260417-12:34:{sec:02}\x01");
        let mut out = Vec::new();

        // Lifecycle 1 — cancel.
        out.extend(build_msg(&format!(
            "35=D\x0134=1\x01{tail}{}11=ABC\x0155=AAPL\x01",
            t(1)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=2\x01{tail}{}11=ABC\x0137=ord1\x01150=A\x0139=A\x0114=0\x01",
            t(2)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=3\x01{tail}{}11=ABC\x0137=ord1\x01150=0\x0139=0\x0114=0\x01",
            t(3)
        )));
        out.extend(build_msg(&format!(
            "35=F\x0134=4\x01{tail}{}11=XYZ\x0141=ABC\x0137=ord1\x01",
            t(4)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=5\x01{tail}{}11=XYZ\x0137=ord1\x01150=6\x0139=6\x0114=0\x01",
            t(5)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=6\x01{tail}{}11=XYZ\x0137=ord1\x01150=4\x0139=4\x0114=0\x01",
            t(6)
        )));

        // Lifecycle 2 — replace.
        out.extend(build_msg(&format!(
            "35=D\x0134=7\x01{tail}{}11=DEF\x0155=MSFT\x01",
            t(7)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=8\x01{tail}{}11=DEF\x0137=ord2\x01150=0\x0139=0\x0114=0\x01",
            t(8)
        )));
        out.extend(build_msg(&format!(
            "35=G\x0134=9\x01{tail}{}11=GHI\x0141=DEF\x0137=ord2\x01",
            t(9)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=10\x01{tail}{}11=GHI\x0137=ord2\x01150=5\x0139=5\x0114=0\x01",
            t(10)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=11\x01{tail}{}11=GHI\x0137=ord2\x01150=F\x0139=1\x0114=50\x01",
            t(11)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=12\x01{tail}{}11=GHI\x0137=ord2\x01150=F\x0139=2\x0114=100\x01",
            t(12)
        )));
        out
    }

    #[test]
    fn cancel_flow_reconstructs_six_events() {
        let buf = synthetic_order_lifecycle();
        let fmt = sniff(&buf).expect("sniff");
        let index = build_from_bytes(&buf, &fmt);
        let tl = OrderTimeline::build(&index, &buf, &fmt, b"ABC").expect("ABC timeline");
        assert_eq!(tl.events.len(), 6, "cancel flow: 6 events via OrderID link");
        assert_eq!(tl.events[0].msg_type.as_slice(), b"D");
        assert_eq!(tl.events[1].msg_type.as_slice(), b"8");
        assert_eq!(tl.events[3].msg_type.as_slice(), b"F");
        assert_eq!(tl.events[5].msg_type.as_slice(), b"8");
        assert_eq!(tl.events[5].exec_type.as_ref().unwrap().as_slice(), b"4");
        assert_eq!(tl.order_ids.len(), 1);
        assert_eq!(tl.order_ids[0].as_slice(), b"ord1");
    }

    #[test]
    fn replace_flow_reconstructs_six_events() {
        let buf = synthetic_order_lifecycle();
        let fmt = sniff(&buf).expect("sniff");
        let index = build_from_bytes(&buf, &fmt);
        let tl = OrderTimeline::build(&index, &buf, &fmt, b"DEF").expect("DEF timeline");
        assert_eq!(tl.events.len(), 6, "replace flow: 6 events");
        assert_eq!(tl.events[2].msg_type.as_slice(), b"G");
        assert_eq!(tl.events[5].cum_qty.as_ref().unwrap().as_slice(), b"100");
    }

    #[test]
    fn unknown_clordid_returns_none() {
        let buf = synthetic_order_lifecycle();
        let fmt = sniff(&buf).expect("sniff");
        let index = build_from_bytes(&buf, &fmt);
        assert!(OrderTimeline::build(&index, &buf, &fmt, b"no-such-id").is_none());
    }

    #[test]
    fn gantt_has_correct_width_and_chars() {
        let buf = synthetic_order_lifecycle();
        let fmt = sniff(&buf).expect("sniff");
        let index = build_from_bytes(&buf, &fmt);
        let tl = OrderTimeline::build(&index, &buf, &fmt, b"ABC").unwrap();
        let row = render_gantt(&tl, 60);
        assert_eq!(row.len(), 60);
        // Must contain at least one N (for D) and one X (for 8).
        assert!(row.contains('N'));
        assert!(row.contains('X'));
        assert!(row.contains('C')); // F = Cancel
    }

    #[test]
    fn gantt_clamps_width() {
        let buf = synthetic_order_lifecycle();
        let fmt = sniff(&buf).expect("sniff");
        let index = build_from_bytes(&buf, &fmt);
        let tl = OrderTimeline::build(&index, &buf, &fmt, b"ABC").unwrap();
        assert_eq!(render_gantt(&tl, 2).len(), 10);
        assert_eq!(render_gantt(&tl, 9999).len(), 500);
    }

    /// A cancel request issued before the exchange has assigned an
    /// `OrderID` carries only `11=<new>` and `41=<orig>`. The previous
    /// lookup chain (11 → 37 → 37) could not reach it. With the OrigClOrdID
    /// pushdown, it should now be part of the original order's timeline.
    #[test]
    fn cancel_request_without_order_id_is_included() {
        let tail = "49=A\x0156=B\x01";
        let t = |sec| format!("52=20260417-12:34:{sec:02}\x01");
        let mut out = Vec::new();
        // 1. NewOrderSingle: 11=NEW1, no OrderID yet.
        out.extend(build_msg(&format!(
            "35=D\x0134=1\x01{tail}{}11=NEW1\x0155=AAPL\x01",
            t(1)
        )));
        // 2. OrderCancelRequest before ack: 11=CXL1, 41=NEW1, no 37.
        out.extend(build_msg(&format!(
            "35=F\x0134=2\x01{tail}{}11=CXL1\x0141=NEW1\x01",
            t(2)
        )));
        let fmt = sniff(&out).expect("sniff");
        let index = build_from_bytes(&out, &fmt);
        let tl = OrderTimeline::build(&index, &out, &fmt, b"NEW1").expect("NEW1 timeline");
        assert_eq!(
            tl.events.len(),
            2,
            "cancel without OrderID must be included"
        );
        assert_eq!(tl.events[0].msg_type.as_slice(), b"D");
        assert_eq!(tl.events[1].msg_type.as_slice(), b"F");
    }

    /// A replace chain `A → B → C` (two consecutive OrderCancelReplace
    /// requests) must be followed to fixed point — querying `A` has to
    /// surface the entries for `B` and `C` too, even if they only share
    /// the `41` back-pointer (no shared OrderID).
    #[test]
    fn replace_chain_is_followed_transitively() {
        let tail = "49=A\x0156=B\x01";
        let t = |sec| format!("52=20260417-12:34:{sec:02}\x01");
        let mut out = Vec::new();
        // Chain: A → B → C via OrigClOrdID, no OrderID available yet.
        out.extend(build_msg(&format!(
            "35=D\x0134=1\x01{tail}{}11=A\x0155=AAPL\x01",
            t(1)
        )));
        out.extend(build_msg(&format!(
            "35=G\x0134=2\x01{tail}{}11=B\x0141=A\x01",
            t(2)
        )));
        out.extend(build_msg(&format!(
            "35=G\x0134=3\x01{tail}{}11=C\x0141=B\x01",
            t(3)
        )));
        let fmt = sniff(&out).expect("sniff");
        let index = build_from_bytes(&out, &fmt);
        let tl = OrderTimeline::build(&index, &out, &fmt, b"A").expect("A timeline");
        assert_eq!(
            tl.events.len(),
            3,
            "transitive chain A→B→C must reach 3 events"
        );
        assert_eq!(tl.events[0].msg_type.as_slice(), b"D");
        assert_eq!(tl.events[1].msg_type.as_slice(), b"G");
        assert_eq!(tl.events[2].msg_type.as_slice(), b"G");
    }
}
