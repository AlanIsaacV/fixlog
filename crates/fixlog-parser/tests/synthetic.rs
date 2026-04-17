//! Integration tests that run the SOH tokenizer against the committed synthetic fixtures.

use std::collections::BTreeMap;
use std::path::PathBuf;

use fixlog_parser::{TAG_MSG_TYPE, parse_all};

/// Resolve the path to a file under `fixtures/` from the repo root.
fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
        .join(rel)
}

fn read_fixture(rel: &str) -> Vec<u8> {
    let path = fixture(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn msg_type_counts(msgs: &[&fixlog_parser::RawMessage<'_>]) -> BTreeMap<Vec<u8>, usize> {
    let mut counts: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
    for m in msgs {
        if let Some((_, v)) = m.tags.iter().find(|(t, _)| *t == TAG_MSG_TYPE) {
            *counts.entry(v.to_vec()).or_default() += 1;
        }
    }
    counts
}

#[test]
fn minimal_4_4_parses_all_ten_messages() {
    let bytes = read_fixture("synthetic/minimal_4.4.log");
    let results: Vec<_> = parse_all(&bytes).collect();

    let oks: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    let errs = results.iter().filter(|r| r.is_err()).count();

    assert_eq!(oks.len(), 10, "expected 10 valid messages");
    assert_eq!(errs, 0, "expected no parse errors");

    let counts = msg_type_counts(&oks);
    let expected: BTreeMap<Vec<u8>, usize> = [
        (b"A".to_vec(), 1), // Logon
        (b"0".to_vec(), 2), // Heartbeat
        (b"1".to_vec(), 1), // TestRequest
        (b"D".to_vec(), 1), // NewOrderSingle
        (b"8".to_vec(), 3), // ExecutionReport
        (b"F".to_vec(), 1), // OrderCancelRequest
        (b"5".to_vec(), 1), // Logout
    ]
    .into_iter()
    .collect();
    assert_eq!(counts, expected);
}

#[test]
fn malformed_emits_valid_messages_and_logs_errors() {
    let bytes = read_fixture("synthetic/malformed.log");
    let results: Vec<_> = parse_all(&bytes).collect();

    let oks: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    let errs = results.iter().filter(|r| r.is_err()).count();

    // Emitted: 3 valid + 1 empty-value (semantically odd, structurally fine) + 1 bad-checksum
    // (checksum mismatches are non-fatal so the message is still surfaced).
    assert_eq!(oks.len(), 5, "expected 5 emitted messages");
    // Only structural failure: inconsistent BodyLength (9=999) walks past EOF.
    assert_eq!(errs, 1, "expected 1 structural error");

    // Emitted MsgTypes: A (Logon), 0 (Heartbeat), 1 (TestRequest, bad-csum), D, D.
    let counts = msg_type_counts(&oks);
    let expected: BTreeMap<Vec<u8>, usize> = [
        (b"A".to_vec(), 1),
        (b"0".to_vec(), 1),
        (b"1".to_vec(), 1),
        (b"D".to_vec(), 2),
    ]
    .into_iter()
    .collect();
    assert_eq!(counts, expected);
}

#[test]
fn parse_all_yields_distinct_offsets_for_each_message() {
    let bytes = read_fixture("synthetic/minimal_4.4.log");
    let offsets: Vec<u64> = parse_all(&bytes)
        .filter_map(Result::ok)
        .map(|m| m.offset)
        .collect();
    let mut sorted = offsets.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(offsets, sorted, "offsets must be strictly increasing");
}
