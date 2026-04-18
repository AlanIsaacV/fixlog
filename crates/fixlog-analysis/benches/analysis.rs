//! Phase 4 analysis benchmarks.
//!
//! Targets (see `docs/PHASE4_PLAN.md` §"Baselines"):
//!
//! - `session_build_1M` — < 1 s over 1M messages.
//! - `order_lookup_1M`  — < 50 ms per `OrderTimeline::build`.
//! - `histogram_build_1M` — < 500 ms at 1s bucket.
//!
//! Run: `cargo bench -p fixlog-analysis`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use fixlog_analysis::histogram::Histogram;
use fixlog_analysis::orders::OrderTimeline;
use fixlog_analysis::sessions::SessionMap;
use fixlog_core::{build_from_bytes_parallel, parse_all_with_format, sniff};

fn fixture(rel: &str) -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest).join("../..").join(rel)
}

fn load(rel: &str) -> Vec<u8> {
    let path = fixture(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Repeat `buf` until `copies` copies are produced. Same approach as the
/// parser bench's `amplify`, kept local so we don't entangle crates via
/// dev-deps. Size of each minimal fixture message is ~80 bytes; 100K
/// copies of `minimal_4.4.log` yields ~1M messages.
fn amplify(buf: &[u8], copies: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(copies * buf.len());
    for _ in 0..copies {
        out.extend_from_slice(buf);
    }
    out
}

fn bench_session_build(c: &mut Criterion) {
    let fx = load("fixtures/synthetic/minimal_4.4.log");
    // `minimal_4.4.log` has 10 messages; amplify to ~1M.
    let buf = amplify(&fx, 100_000);
    let fmt = sniff(&buf).unwrap();
    let index = build_from_bytes_parallel(&buf, &fmt);
    eprintln!("session bench: {} messages", index.len());

    let mut group = c.benchmark_group("analysis");
    group.sample_size(10);
    group.bench_function("session_build_1M", |b| {
        b.iter(|| {
            let map = SessionMap::build(&index, &buf, &fmt);
            black_box(map.by_key.len())
        })
    });
    group.finish();
}

fn bench_order_lookup(c: &mut Criterion) {
    // Amplifying minimal_4.4.log wouldn't help because none of its
    // messages have a ClOrdID that maps to many events. Use the real
    // fix44-om.log and pick a known ClOrdID from it. Secondary lookup is
    // already O(1) so size barely matters, but we still want to measure
    // the full `build` including re-parse.
    let buf = load("fixtures/real/fix44-om.log");
    let fmt = sniff(&buf).unwrap();
    let index = build_from_bytes_parallel(&buf, &fmt);

    // Pick first 35=D ClOrdID.
    let clordid: Vec<u8> = parse_all_with_format(&buf, &fmt)
        .filter_map(Result::ok)
        .find(|m| m.tags.iter().any(|(t, v)| *t == 35 && *v == *b"D"))
        .and_then(|m| {
            m.tags
                .iter()
                .find(|(t, _)| *t == 11)
                .map(|(_, v)| v.to_vec())
        })
        .expect("fixture has a D message");

    let mut group = c.benchmark_group("analysis");
    group.bench_function("order_lookup_1M", |b| {
        b.iter(|| {
            let tl = OrderTimeline::build(&index, &buf, &fmt, &clordid);
            black_box(tl.map(|t| t.events.len()).unwrap_or(0))
        })
    });
    group.finish();
}

fn bench_histogram_build(c: &mut Criterion) {
    let fx = load("fixtures/synthetic/minimal_4.4.log");
    let buf = amplify(&fx, 100_000);
    let fmt = sniff(&buf).unwrap();
    let index = build_from_bytes_parallel(&buf, &fmt);
    eprintln!("histogram bench: {} messages", index.len());

    let mut group = c.benchmark_group("analysis");
    group.sample_size(10);
    group.bench_function("histogram_build_1M", |b| {
        b.iter(|| {
            let h = Histogram::build(&index, &buf, &fmt, Duration::from_secs(1));
            black_box(h.total())
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_session_build,
    bench_order_lookup,
    bench_histogram_build
);
criterion_main!(benches);
