//! Format smoke tests — lock the output shape of each writer so that any
//! accidental drift (trailing whitespace, field ordering, JSON schema
//! keys) surfaces in CI before a consumer starts depending on the new
//! behaviour.

use fixlog_dict::{CHAIN_FIX44, ResolvedField, ResolvedMessage};
use fixlog_parser::parse_one;
use fixlog_render::{write_csv_header, write_csv_row, write_fix, write_jsonl, write_pretty};

fn sample_resolved<'a>() -> ResolvedMessage<'a> {
    ResolvedMessage {
        offset: 0,
        msg_type_name: Some("Logon"),
        chain: CHAIN_FIX44,
        fields: vec![
            ResolvedField {
                tag: 35,
                name: Some("MsgType"),
                value: b"A",
                value_label: Some("Logon"),
            },
            ResolvedField {
                tag: 49,
                name: Some("SenderCompID"),
                value: b"SENDER",
                value_label: None,
            },
        ],
    }
}

#[test]
fn pretty_emits_aligned_block_with_header_and_trailing_blank_line() {
    let msg = sample_resolved();
    let mut buf = Vec::new();
    write_pretty(&mut buf, &msg, 42).unwrap();
    let out = String::from_utf8(buf).unwrap();

    assert!(out.starts_with("Message @ offset 0 (42 bytes) Logon\n"));
    assert!(out.contains("   35  MsgType      = A (Logon)\n"));
    assert!(out.contains("   49  SenderCompID = SENDER\n"));
    assert!(
        out.ends_with("\n\n"),
        "trailing blank line separates blocks"
    );
}

#[test]
fn jsonl_emits_one_line_with_expected_keys() {
    let msg = sample_resolved();
    let mut buf = Vec::new();
    write_jsonl(&mut buf, &msg, 42).unwrap();
    let out = String::from_utf8(buf).unwrap();

    assert_eq!(out.matches('\n').count(), 1, "exactly one trailing newline");
    assert!(out.contains(r#""offset":0"#));
    assert!(out.contains(r#""raw_len":42"#));
    assert!(out.contains(r#""msg_type_name":"Logon""#));
    assert!(out.contains(r#""tag":35"#));
    assert!(out.contains(r#""name":"MsgType""#));
    assert!(out.contains(r#""value":"A""#));
    assert!(out.contains(r#""value_label":"Logon""#));
}

#[test]
fn fix_emits_bytes_plus_newline() {
    let raw: &[u8] = b"8=FIX.4.4\x019=5\x0135=A\x0110=000\x01";
    let mut buf = Vec::new();
    write_fix(&mut buf, raw).unwrap();
    assert_eq!(buf.last(), Some(&b'\n'));
    assert_eq!(buf.len(), raw.len() + 1);
    assert_eq!(&buf[..raw.len()], raw);
}

#[test]
fn csv_header_and_row_shape() {
    let raw = b"8=FIX.4.4\x019=55\x0135=A\x0134=2\x0149=SENDER\x0156=TARGET\x0152=20260416-13:30:00.000\x0110=000\x01";
    let (msg, _) = parse_one(raw).unwrap();

    let mut buf = Vec::new();
    write_csv_header(&mut buf).unwrap();
    write_csv_row(&mut buf, 7, 128, &msg).unwrap();
    let out = String::from_utf8(buf).unwrap();

    let mut lines = out.lines();
    assert_eq!(
        lines.next().unwrap(),
        "ordinal,offset,msg_type,sender,target,seq_num,sending_time"
    );
    assert_eq!(
        lines.next().unwrap(),
        "7,128,A,SENDER,TARGET,2,20260416-13:30:00.000"
    );
    assert!(lines.next().is_none());
}

#[test]
fn csv_row_escapes_embedded_commas_and_quotes() {
    // Build a synthetic message with an awkward sender containing a comma.
    // Checksum is ignored by `parse_one` in this test — any 3-digit value
    // passes because the parser treats checksum mismatches as non-fatal.
    let body = b"35=A\x0134=1\x0149=EVIL,CORP\x0156=T\x0152=X\x01";
    let mut raw: Vec<u8> = format!("8=FIX.4.4\x019={}\x01", body.len()).into_bytes();
    raw.extend_from_slice(body);
    raw.extend_from_slice(b"10=000\x01");
    let (msg, _) = parse_one(&raw).unwrap();

    let mut buf = Vec::new();
    write_csv_row(&mut buf, 0, 0, &msg).unwrap();
    let line = String::from_utf8(buf).unwrap();
    assert!(
        line.contains("\"EVIL,CORP\""),
        "comma-bearing field must be quoted: {line}"
    );
}
