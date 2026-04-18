//! Integration tests for the generated FIX dictionaries.

use fixlog_dict::{
    FixVersion, chain_field_by_tag, chain_msg_type_label, enum_value_label, field_by_tag,
    field_count, message_count, msg_type_label,
};

#[test]
fn fix44_session_fields_resolve() {
    let v = FixVersion::Fix44;
    assert_eq!(field_by_tag(v, 8).unwrap().name, "BeginString");
    assert_eq!(field_by_tag(v, 9).unwrap().name, "BodyLength");
    assert_eq!(field_by_tag(v, 10).unwrap().name, "CheckSum");
    assert_eq!(field_by_tag(v, 35).unwrap().name, "MsgType");
    assert_eq!(field_by_tag(v, 34).unwrap().name, "MsgSeqNum");
    assert_eq!(field_by_tag(v, 49).unwrap().name, "SenderCompID");
    assert_eq!(field_by_tag(v, 52).unwrap().name, "SendingTime");
    assert_eq!(field_by_tag(v, 56).unwrap().name, "TargetCompID");
}

#[test]
fn unknown_tag_returns_none() {
    assert!(field_by_tag(FixVersion::Fix44, 99999).is_none());
    assert!(field_by_tag(FixVersion::Fix44, 0).is_none());
}

#[test]
fn fix44_msg_types_resolve() {
    let v = FixVersion::Fix44;
    assert_eq!(msg_type_label(v, b"A"), Some("Logon"));
    assert_eq!(msg_type_label(v, b"5"), Some("Logout"));
    assert_eq!(msg_type_label(v, b"0"), Some("Heartbeat"));
    assert_eq!(msg_type_label(v, b"1"), Some("TestRequest"));
    assert_eq!(msg_type_label(v, b"D"), Some("NewOrderSingle"));
    assert_eq!(msg_type_label(v, b"8"), Some("ExecutionReport"));
    assert_eq!(msg_type_label(v, b"F"), Some("OrderCancelRequest"));
    assert!(msg_type_label(v, b"ZZZZ").is_none());
}

#[test]
fn fix44_side_enum_values() {
    let v = FixVersion::Fix44;
    assert_eq!(enum_value_label(v, 54, b"1"), Some("BUY"));
    assert_eq!(enum_value_label(v, 54, b"2"), Some("SELL"));
    assert!(enum_value_label(v, 54, b"Z").is_none());
    assert!(enum_value_label(v, 11, b"foo").is_none());
}

#[test]
fn fixt11_defines_session_fields() {
    // FIXT11 is the session-layer-only dictionary; it defines the admin tags.
    assert_eq!(
        field_by_tag(FixVersion::Fixt11, 8).unwrap().name,
        "BeginString"
    );
    assert_eq!(
        field_by_tag(FixVersion::Fixt11, 35).unwrap().name,
        "MsgType"
    );
    assert_eq!(
        field_by_tag(FixVersion::Fixt11, 1128).unwrap().name,
        "ApplVerID"
    );
}

#[test]
fn fix50sp2_defines_application_fields() {
    // Application-layer tag introduced after FIX 4.4 lives in FIX 5.0SP2.
    assert_eq!(
        field_by_tag(FixVersion::Fix50Sp2, 1301).unwrap().name,
        "MarketID"
    );
    // FIXT-session-only tags are NOT duplicated into the app dictionary.
    assert!(field_by_tag(FixVersion::Fix50Sp2, 1137).is_none());
}

#[test]
fn chain_falls_through_fixt11_to_fix50sp2() {
    let chain = fixlog_dict::CHAIN_FIXT11_FIX50SP2;
    // Tag 1137 resolves in FIXT11 (first hit in chain).
    assert_eq!(
        chain_field_by_tag(chain, 1137).unwrap().name,
        "DefaultApplVerID"
    );
    // Tag 1301 only lives in FIX50SP2; the chain falls through to it.
    assert_eq!(chain_field_by_tag(chain, 1301).unwrap().name, "MarketID");
    // MsgType 'A' (Logon) exists in FIXT11 — first hit wins.
    assert_eq!(chain_msg_type_label(chain, b"A"), Some("Logon"));
}

#[test]
fn fix50_defines_application_fields() {
    // Application messages exist in plain FIX 5.0 (pre-SP1/SP2). Well-known
    // app MsgTypes and tags must resolve.
    assert_eq!(
        msg_type_label(FixVersion::Fix50, b"D"),
        Some("NewOrderSingle")
    );
    assert_eq!(
        msg_type_label(FixVersion::Fix50, b"8"),
        Some("ExecutionReport")
    );
    // Standard app-layer tag — ClOrdID on NewOrderSingle.
    assert_eq!(field_by_tag(FixVersion::Fix50, 11).unwrap().name, "ClOrdID");
}

#[test]
fn fix50sp1_defines_application_fields() {
    assert_eq!(
        msg_type_label(FixVersion::Fix50Sp1, b"D"),
        Some("NewOrderSingle")
    );
    // ApplicationSequenceControl component landed in SP1 — its underlying
    // ApplID tag (tag 1180) must resolve in the 5.0SP1 dictionary.
    assert_eq!(
        field_by_tag(FixVersion::Fix50Sp1, 1180).unwrap().name,
        "ApplID"
    );
}

#[test]
fn chain_for_routes_applverid_to_right_fix50_version() {
    use fixlog_dict::{
        CHAIN_FIXT11_FIX50, CHAIN_FIXT11_FIX50SP1, CHAIN_FIXT11_FIX50SP2, chain_for,
    };

    assert_eq!(chain_for(b"FIXT.1.1", Some(b"7")), CHAIN_FIXT11_FIX50);
    assert_eq!(chain_for(b"FIXT.1.1", Some(b"8")), CHAIN_FIXT11_FIX50SP1);
    assert_eq!(chain_for(b"FIXT.1.1", Some(b"9")), CHAIN_FIXT11_FIX50SP2);
    // Unknown / missing ApplVerID still defaults to SP2 — no regression.
    assert_eq!(chain_for(b"FIXT.1.1", None), CHAIN_FIXT11_FIX50SP2);
}

// FIX 4.4 has ~912 fields / 92 messages; FIX 5.0 / 5.0SP1 / 5.0SP2 grow
// over time; FIXT11 has ~70 / ~10. Lower-bound asserts detect accidental
// regression in the build-script.
const _: () = {
    assert!(field_count(FixVersion::Fix44) >= 900);
    assert!(message_count(FixVersion::Fix44) >= 90);
    assert!(field_count(FixVersion::Fix50) >= 900);
    assert!(message_count(FixVersion::Fix50) >= 50);
    assert!(field_count(FixVersion::Fix50Sp1) >= 900);
    assert!(message_count(FixVersion::Fix50Sp1) >= 50);
    assert!(field_count(FixVersion::Fix50Sp2) >= 1400);
    assert!(message_count(FixVersion::Fix50Sp2) >= 100);
    assert!(field_count(FixVersion::Fixt11) >= 50);
    assert!(message_count(FixVersion::Fixt11) >= 5);
};
