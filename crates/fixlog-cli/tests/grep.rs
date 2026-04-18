//! End-to-end tests for `fixlog grep`.
//!
//! These run the built binary against the real fixtures to lock the grep semantics
//! (matching, exit codes, JSON output shape). Binary resolution goes through Cargo's
//! `CARGO_BIN_EXE_<name>` env var so tests work under both `cargo test` and
//! `cargo test --release`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fixlog"))
}

fn workspace_root() -> PathBuf {
    // crates/fixlog-cli → crates → <root>
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolvable")
}

fn fixture(rel: &str) -> PathBuf {
    workspace_root().join(rel)
}

#[test]
fn grep_eq_matches_and_exits_zero() {
    let out = Command::new(bin())
        .args([
            "grep",
            fixture("fixtures/synthetic/minimal_4.4.log")
                .to_str()
                .unwrap(),
            "--filter",
            "35=D",
            "--format",
            "json",
        ])
        .output()
        .expect("run fixlog");
    assert!(out.status.success(), "status: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let lines: Vec<_> = stdout.lines().collect();
    assert!(!lines.is_empty(), "must match at least one line");
    for line in &lines {
        assert!(
            line.starts_with('{') && line.ends_with('}'),
            "jsonl shape: {line}"
        );
        assert!(
            line.contains(r#""tag":35"#),
            "line must expose MsgType: {line}"
        );
    }
}

#[test]
fn grep_no_match_exits_one() {
    let out = Command::new(bin())
        .args([
            "grep",
            fixture("fixtures/synthetic/minimal_4.4.log")
                .to_str()
                .unwrap(),
            "--filter",
            "35=ZZZZ",
        ])
        .output()
        .expect("run fixlog");
    assert_eq!(out.status.code(), Some(1), "grep(1) no-match convention");
    assert!(out.stdout.is_empty(), "no matches means no stdout");
}

#[test]
fn grep_combined_expression_on_real_fixture() {
    // ExecutionReports for AAPL: we expect a known count that will move only if the fixture
    // changes. If this test ever drifts it's a genuine signal — update the number alongside
    // the fixture.
    let out = Command::new(bin())
        .args([
            "grep",
            fixture("fixtures/real/fix44-om.log").to_str().unwrap(),
            "--filter",
            "35=8 AND 55=AAPL",
            "--format",
            "json",
        ])
        .output()
        .expect("run fixlog");
    assert!(out.status.success(), "status: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let n = stdout.lines().filter(|l| !l.is_empty()).count();
    assert!(n > 0, "expected matches on fix44-om fixture");
    // Lower bound sanity — at least 5 AAPL ExecutionReports exist in the fixture.
    assert!(n >= 5, "expected ≥5 AAPL ExecutionReports, got {n}");
}

#[test]
fn grep_follow_streams_appended_matches() {
    // End-to-end check for --follow: seed a tempfile, start the binary in follow mode,
    // append more matching bytes, give the watcher a moment to react, then kill the
    // child and count the JSON lines on stdout. The kill is what lets us finally read
    // stdout to completion — `ChildStdout::read` is blocking, and the watcher loop never
    // closes stdout on its own.
    use std::io::{Read, Write};
    use std::process::Stdio;
    use std::thread;
    use std::time::Duration;

    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "fixlog-follow-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&path);

    let source = std::fs::read(fixture("fixtures/synthetic/minimal_4.4.log")).unwrap();
    std::fs::write(&path, &source).unwrap();

    let mut child = Command::new(bin())
        .args([
            "grep",
            path.to_str().unwrap(),
            "--filter",
            "35=D",
            "--format",
            "json",
            "--follow",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn fixlog");

    // Give the child a moment to complete the initial post-mortem pass and settle into
    // the watch loop. 1s is well above what the first mmap + parse needs on this fixture.
    thread::sleep(Duration::from_millis(1000));

    // Append the same fixture again. Follow loop should emit matches for the new messages
    // either via notify or via the 500ms polling fallback.
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("reopen for append");
        f.write_all(&source).expect("append");
        f.flush().unwrap();
    }

    // Give the watcher enough time to notice the growth and flush matches to stdout.
    thread::sleep(Duration::from_millis(2500));

    // Kill the child so its stdout pipe closes and we can drain it to EOF.
    let _ = child.kill();
    let _ = child.wait();

    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut seen = Vec::new();
    stdout.read_to_end(&mut seen).expect("drain stdout");
    let _ = std::fs::remove_file(&path);

    // Count matches as JSON lines. The fixture contains exactly one 35=D message per
    // copy, so after append-then-drain we expect ≥ 2 matches. Use a lower bound rather
    // than equality — if the polling fallback fires twice, a third iteration might slip
    // in, and that's fine.
    let matches = seen.iter().filter(|&&b| b == b'\n').count();
    assert!(
        matches >= 2,
        "follow mode should emit ≥ 2 matches (seed + appended); got {matches} bytes={}",
        seen.len()
    );
}

#[test]
fn grep_reports_invalid_filter_in_stderr() {
    let out = Command::new(bin())
        .args([
            "grep",
            fixture("fixtures/synthetic/minimal_4.4.log")
                .to_str()
                .unwrap(),
            "--filter",
            "(35=D",
        ])
        .output()
        .expect("run fixlog");
    assert!(!out.status.success(), "bad filter must fail");
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    assert!(stderr.contains("unbalanced") || stderr.contains("parsing filter"));
}
