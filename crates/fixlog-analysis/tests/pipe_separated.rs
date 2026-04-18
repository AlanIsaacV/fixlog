//! Regression: analysis builders must honour `LogFormat.separator` when
//! re-parsing messages. Before Fase 5 / Task 1, the three builders called
//! `parse_one(bytes)` directly, which hard-codes SOH — causing silent
//! failure (0 sessions / 0 events / 0 bins) on any pipe-separated log such
//! as QuickFIX-J captures.

use std::time::Duration;

use fixlog_analysis::histogram::Histogram;
use fixlog_analysis::orders::OrderTimeline;
use fixlog_analysis::sessions::{SessionKey, SessionMap};
use fixlog_core::{build_from_bytes, sniff};

const PIPE_LOG: &[u8] = include_bytes!("../../../fixtures/synthetic/pipe_separated.log");

#[test]
fn sessions_build_over_pipe_separated_log() {
    let fmt = sniff(PIPE_LOG).expect("sniffable");
    // Sanity: the fixture is pipe-separated, not SOH.
    assert_ne!(
        fmt.separator.as_byte(),
        b'\x01',
        "fixture must be pipe-separated"
    );

    let index = build_from_bytes(PIPE_LOG, &fmt);
    let map = SessionMap::build(&index, PIPE_LOG, &fmt);

    assert_eq!(
        map.by_key.len(),
        1,
        "pipe_separated.log has one canonical SENDER/TARGET session"
    );
    let (key, stats) = map.by_key.iter().next().unwrap();
    assert_eq!(key, &SessionKey::canonical(b"SENDER", b"TARGET"));
    assert_eq!(
        stats.in_count + stats.out_count,
        index.len() as u32,
        "every indexed message should belong to the session",
    );
}

#[test]
fn order_timeline_finds_events_in_pipe_separated_log() {
    let fmt = sniff(PIPE_LOG).expect("sniffable");
    let index = build_from_bytes(PIPE_LOG, &fmt);

    // ORDER001 has a NewOrderSingle (D) plus 3 ExecutionReports (8) in the
    // fixture, sharing OrderID=EXEC001.
    let tl = OrderTimeline::build(&index, PIPE_LOG, &fmt, b"ORDER001")
        .expect("ORDER001 timeline should resolve on pipe-separated log");
    assert!(
        tl.events.len() >= 4,
        "expected ≥ 4 events for ORDER001, got {}",
        tl.events.len()
    );
    assert_eq!(tl.events[0].msg_type.as_slice(), b"D");
    assert_eq!(tl.order_ids.len(), 1);
    assert_eq!(tl.order_ids[0].as_slice(), b"EXEC001");
}

#[test]
fn histogram_buckets_pipe_separated_timestamps() {
    let fmt = sniff(PIPE_LOG).expect("sniffable");
    let index = build_from_bytes(PIPE_LOG, &fmt);
    let h = Histogram::build(&index, PIPE_LOG, &fmt, Duration::from_secs(1));
    assert!(
        !h.bins.is_empty(),
        "histogram should bucket at least one pipe-separated timestamp",
    );
    assert_eq!(
        h.total() + h.dropped_no_time as u64,
        index.len() as u64,
        "every indexed message must be binned or dropped with a reason",
    );
}
