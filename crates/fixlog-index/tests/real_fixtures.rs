//! End-to-end checks that the index matches the parser on the real fixtures.
//!
//! Numbers here must track `state.md` under "Real-fixture parse metrics". If these tests
//! diverge it means either the parser or the indexer regressed.

use fixlog_format::sniff;
use fixlog_index::build_from_bytes;
use fixlog_parser::parse_all_with_format;

const FIX44_OM: &[u8] = include_bytes!("../../../fixtures/real/fix44-om.log");
const FIXT11_MD: &[u8] = include_bytes!("../../../fixtures/real/fixt11-md.log");

fn parser_count(buf: &[u8], fmt: &fixlog_format::LogFormat) -> usize {
    parse_all_with_format(buf, fmt)
        .filter_map(Result::ok)
        .count()
}

#[test]
fn fix44_om_index_matches_parser() {
    let fmt = sniff(FIX44_OM).expect("sniffable");
    let index = build_from_bytes(FIX44_OM, &fmt);
    assert_eq!(index.len(), parser_count(FIX44_OM, &fmt));
    assert_eq!(index.consumed, index.messages.last().unwrap().end());
}

#[test]
fn fixt11_md_index_matches_parser() {
    let fmt = sniff(FIXT11_MD).expect("sniffable");
    let index = build_from_bytes(FIXT11_MD, &fmt);
    assert_eq!(index.len(), parser_count(FIXT11_MD, &fmt));
    assert_eq!(index.consumed, index.messages.last().unwrap().end());
}

#[test]
fn secondary_lookup_covers_every_message_of_35d_in_fix44_om() {
    // For the 35=D predicate, the secondary index must return exactly the ordinals that
    // a full-scan would return. This guards against off-by-one / dedup bugs.
    let fmt = sniff(FIX44_OM).expect("sniffable");
    let index = build_from_bytes(FIX44_OM, &fmt);

    let mut scan_ordinals: Vec<u32> = Vec::new();
    for (i, _) in index.messages.iter().enumerate() {
        let bytes = index.message_bytes(FIX44_OM, i).unwrap();
        let msg = parse_all_with_format(bytes, &fmt)
            .filter_map(Result::ok)
            .next()
            .unwrap();
        if msg.tags.iter().any(|(t, v)| *t == 35 && v == b"D") {
            scan_ordinals.push(i as u32);
        }
    }
    let lookup = index.secondary.lookup(35, b"D");
    assert_eq!(lookup, scan_ordinals.as_slice());
}

#[test]
fn indexed_slice_reparses_to_single_message_sample() {
    // Sample 50 messages evenly distributed to keep the test cheap.
    let fmt = sniff(FIX44_OM).expect("sniffable");
    let index = build_from_bytes(FIX44_OM, &fmt);
    assert!(index.len() > 50, "fixture expected to hold >50 messages");
    let step = index.len() / 50;
    for i in (0..index.len()).step_by(step) {
        let slice = index.message_bytes(FIX44_OM, i).expect("in range");
        let n = parse_all_with_format(slice, &fmt)
            .filter_map(Result::ok)
            .count();
        assert_eq!(n, 1, "message #{i} should re-parse as exactly 1 message");
    }
}
