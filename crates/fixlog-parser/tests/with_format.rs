//! End-to-end integration of the sniffer + parser pipeline against the synthetic fixtures
//! that exercise non-default layouts (pipe separator, timestamp prefix).

use std::path::PathBuf;

use fixlog_format::{LinePrefix, Separator, sniff};
use fixlog_parser::{TAG_MSG_TYPE, parse_all_with_format};

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

#[test]
fn sniff_then_parse_pipe_separated() {
    let bytes = read_fixture("synthetic/pipe_separated.log");
    let format = sniff(&bytes).expect("sniffer must recognize the layout");
    assert_eq!(format.separator, Separator::Pipe);
    assert_eq!(format.line_prefix, LinePrefix::None);

    let results: Vec<_> = parse_all_with_format(&bytes, &format).collect();
    let oks: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    let errs = results.iter().filter(|r| r.is_err()).count();

    assert_eq!(errs, 0, "no errors expected on a clean fixture");
    assert_eq!(oks.len(), 10, "fixture has 10 well-formed messages");

    // First message is a Logon (35=A); MsgType field carries the literal byte.
    let first_msg_type = oks[0]
        .tags
        .iter()
        .find(|(t, _)| *t == TAG_MSG_TYPE)
        .map(|(_, v)| *v)
        .unwrap();
    assert_eq!(first_msg_type, b"A");
}

#[test]
fn sniff_then_parse_timestamp_prefix() {
    let bytes = read_fixture("synthetic/with_timestamp_prefix.log");
    let format = sniff(&bytes).expect("sniffer must recognize the layout");
    assert_eq!(format.separator, Separator::Soh);
    // "20260416-13:30:00.000 : " is exactly 24 bytes.
    assert_eq!(format.line_prefix, LinePrefix::Fixed(24));

    let results: Vec<_> = parse_all_with_format(&bytes, &format).collect();
    let oks: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    let errs = results.iter().filter(|r| r.is_err()).count();

    assert_eq!(errs, 0);
    assert_eq!(oks.len(), 10);
}
