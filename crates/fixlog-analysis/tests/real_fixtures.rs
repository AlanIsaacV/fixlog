//! End-to-end checks against the real fixtures.

use std::time::Duration;

use fixlog_analysis::histogram::Histogram;
use fixlog_analysis::orders::OrderTimeline;
use fixlog_analysis::sessions::{SessionKey, SessionMap};
use fixlog_core::parser::TAG_MSG_TYPE;
use fixlog_core::{build_from_bytes, parse_all_with_format, sniff};

const FIX44_OM: &[u8] = include_bytes!("../../../fixtures/real/fix44-om.log");

#[test]
fn fix44_om_has_one_session_no_gaps() {
    let fmt = sniff(FIX44_OM).expect("sniffable");
    let index = build_from_bytes(FIX44_OM, &fmt);
    let map = SessionMap::build(&index, FIX44_OM, &fmt);

    assert_eq!(
        map.by_key.len(),
        1,
        "fix44-om.log is expected to hold exactly one canonical session"
    );

    let (key, stats) = map.by_key.iter().next().unwrap();
    // Canonical: BVLORSG < KALDMA1TRI lexicographically.
    assert_eq!(key, &SessionKey::canonical(b"KALDMA1TRI", b"BVLORSG"));
    assert_eq!(stats.in_count + stats.out_count, 5419);
    assert!(
        stats.gaps.is_empty(),
        "fix44-om.log has no MsgSeqNum gaps, found {:?}",
        stats.gaps
    );
}

#[test]
fn fix44_om_order_timeline_starts_with_new_order_single() {
    let fmt = sniff(FIX44_OM).expect("sniffable");
    let index = build_from_bytes(FIX44_OM, &fmt);

    // Pull the ClOrdID of the first `35=D` message via a streaming parse.
    let clordid: Vec<u8> = parse_all_with_format(FIX44_OM, &fmt)
        .filter_map(Result::ok)
        .find(|m| {
            m.tags
                .iter()
                .any(|(t, v)| *t == TAG_MSG_TYPE && *v == *b"D")
        })
        .and_then(|m| {
            m.tags
                .iter()
                .find(|(t, _)| *t == 11)
                .map(|(_, v)| v.to_vec())
        })
        .expect("first 35=D has a ClOrdID");

    let tl = OrderTimeline::build(&index, FIX44_OM, &fmt, &clordid)
        .expect("timeline for first known ClOrdID");
    assert!(
        tl.events.len() >= 2,
        "expected ≥ 2 events, got {}",
        tl.events.len()
    );
    assert_eq!(
        tl.events[0].msg_type.as_slice(),
        b"D",
        "first event should be NewOrderSingle"
    );
}

#[test]
fn fix44_om_histogram_covers_all_messages() {
    let fmt = sniff(FIX44_OM).expect("sniffable");
    let index = build_from_bytes(FIX44_OM, &fmt);
    let h = Histogram::build(&index, FIX44_OM, &fmt, Duration::from_secs(1));
    assert_eq!(
        h.total() + h.dropped_no_time as u64,
        5419,
        "every message accounted for — binned or dropped"
    );
    assert!(!h.bins.is_empty());
    let spark = h.render_sparkline(80);
    assert_eq!(spark.chars().count(), 80);
}
