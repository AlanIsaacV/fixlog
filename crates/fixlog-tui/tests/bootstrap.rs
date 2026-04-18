//! Integration tests: bootstrap against the real fixtures. Numbers here must
//! track `docs/agent/state.md` under "Real-fixture parse metrics" (5419 for
//! fix44-om, 8229 for fixt11-md). If these diverge, either the parser or the
//! index builder regressed.

use std::path::PathBuf;

use fixlog_tui::state::{ViewMode, bootstrap};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/real")).join(name)
}

#[test]
fn bootstrap_fix44_om_counts_match_parser() {
    let state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
    assert_eq!(state.index.len(), 5419, "index length");
    assert_eq!(state.visible.len(), 5419, "visible length (no filter)");
    assert_eq!(state.mode, ViewMode::Follow);
    assert_eq!(state.cursor, 5418);
}

#[test]
fn bootstrap_fixt11_md_counts_match_parser() {
    let state = bootstrap(&fixture("fixt11-md.log"), None).expect("bootstrap");
    assert_eq!(state.index.len(), 8229);
    assert_eq!(state.visible.len(), 8229);
    assert_eq!(state.cursor, 8228);
}

#[test]
fn bootstrap_applies_initial_filter() {
    let state = bootstrap(&fixture("fix44-om.log"), Some("35=D")).expect("bootstrap");
    assert_eq!(state.index.len(), 5419, "index not filtered");
    assert!(
        state.visible.len() < 5419,
        "visible should drop after 35=D filter (got {})",
        state.visible.len()
    );
    assert!(
        !state.visible.is_empty(),
        "35=D should match at least one NewOrderSingle"
    );
    // cursor sits at the end of `visible`, not the end of the file.
    assert_eq!(state.cursor, state.visible.len() - 1);
}

#[test]
fn bootstrap_rejects_bad_filter() {
    match bootstrap(&fixture("fix44-om.log"), Some("35=")) {
        Ok(_) => panic!("incomplete predicate should not bootstrap"),
        Err(e) => {
            let msg = format!("{e:#}");
            assert!(
                msg.contains("parsing initial filter"),
                "expected context, got: {msg}"
            );
        }
    }
}

#[test]
fn bootstrap_preserves_consumed_watermark() {
    let state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
    let last = state.index.messages.last().expect("non-empty index");
    assert_eq!(
        state.index.consumed,
        last.end(),
        "consumed invariant: end of last indexed message"
    );
}
