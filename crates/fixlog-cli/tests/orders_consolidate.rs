//! Integration tests for `fixlog orders consolidate`.
//!
//! The binary is resolved via `CARGO_BIN_EXE_fixlog` so these run under
//! both `cargo test` and `cargo test --release`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fixlog"))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolvable")
}

fn fixture() -> PathBuf {
    workspace_root().join("fixtures/orders/FIXT.1.1-2X00070015_7-MME.messages.20260514.log")
}

fn run(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("fixlog binary should be runnable");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn pretty_lists_known_filled_orders() {
    let path = fixture();
    let (stdout, _, code) = run(&[
        "orders",
        "consolidate",
        path.to_str().unwrap(),
        "--format",
        "pretty",
    ]);
    assert_eq!(code, 0, "exit 0 when orders are found");
    // Header + at least one known filled order from the fixture.
    assert!(stdout.contains("ClOrdID"), "header present");
    assert!(stdout.contains("Notional"), "notional column header");
    assert!(
        stdout.contains("FILLED"),
        "at least one FILLED order present"
    );
}

#[test]
fn csv_has_header_and_rows() {
    let path = fixture();
    let (stdout, _, code) = run(&[
        "orders",
        "consolidate",
        path.to_str().unwrap(),
        "--format",
        "csv",
    ]);
    assert_eq!(code, 0);
    let first = stdout.lines().next().expect("at least the header");
    assert_eq!(
        first,
        "root_clordid,family,side,symbol,order_qty,cum_qty,notional,avg_px,fills,final_ord_status"
    );
    assert!(stdout.lines().count() > 1, "expect rows beyond the header");
}

#[test]
fn json_is_array_with_objects() {
    let path = fixture();
    let (stdout, _, code) = run(&[
        "orders",
        "consolidate",
        path.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert_eq!(code, 0);
    let s = stdout.trim();
    assert!(s.starts_with('['), "json output starts with array");
    assert!(s.ends_with(']'), "json output ends with array");
    // Spot-check a known field name.
    assert!(s.contains("\"root_clordid\""));
    assert!(s.contains("\"cum_qty\""));
}

#[test]
fn split_log_into_plain_plus_gz_matches_whole() {
    // Split the fixture at a `8=FIX` boundary near the midpoint, gzip
    // the tail, and feed `<head_plain> <tail.gz>` to consolidate.
    // The output must equal the single-file run — proves that
    // multi-input, .gz auto-detect, and mid-stream message boundaries
    // all carry across.
    let path = fixture();
    let bytes = std::fs::read(&path).unwrap();
    let mid = bytes.len() / 2;
    let boundary = (mid..bytes.len())
        .find(|i| bytes[*i..].starts_with(b"8=FIX"))
        .expect("must find a boundary near the midpoint");
    let (head, tail) = bytes.split_at(boundary);

    let dir = tempfile::tempdir().unwrap();
    let head_path = dir.path().join("head.log");
    let tail_gz = dir.path().join("tail.log.gz");
    std::fs::write(&head_path, head).unwrap();
    let mut enc = flate2::write::GzEncoder::new(
        std::fs::File::create(&tail_gz).unwrap(),
        flate2::Compression::default(),
    );
    enc.write_all(tail).unwrap();
    enc.finish().unwrap();

    let (whole, _, code1) = run(&[
        "orders",
        "consolidate",
        path.to_str().unwrap(),
        "--format",
        "csv",
    ]);
    assert_eq!(code1, 0);

    let (split, _, code2) = run(&[
        "orders",
        "consolidate",
        head_path.to_str().unwrap(),
        tail_gz.to_str().unwrap(),
        "--format",
        "csv",
    ]);
    assert_eq!(code2, 0);

    assert_eq!(
        whole, split,
        "splitting + gzipping the tail must not change the consolidated output"
    );
}

#[test]
fn no_inputs_errors_via_clap() {
    let (_, stderr, code) = run(&["orders", "consolidate"]);
    assert_ne!(code, 0, "missing required <inputs>");
    assert!(
        stderr.to_lowercase().contains("inputs")
            || stderr.to_lowercase().contains("required")
            || stderr.to_lowercase().contains("usage"),
        "clap should complain about missing positional"
    );
}

#[test]
fn timeline_mode_still_works() {
    // Backward-compat: `fixlog orders <file>` keeps its top-N listing.
    let path = fixture();
    let (stdout, _, code) = run(&["orders", path.to_str().unwrap(), "--limit", "5"]);
    assert_eq!(code, 0);
    assert!(
        stdout.lines().count() >= 3,
        "header + separator + at least one row"
    );
}
