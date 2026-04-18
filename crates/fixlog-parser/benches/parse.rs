//! Baseline throughput benchmarks for the parser.
//!
//! Phase-1 closing measurement: we care about `parse_all` throughput over the three synthetic
//! shapes (SOH, pipe, prefixed) plus the two real fixtures. Criterion reports both time and
//! bytes/sec via `Throughput::Bytes(len)`, which is the number we compare against in Phase 2.
//!
//! Run:
//!
//! ```text
//! cargo bench -p fixlog-parser
//! ```

use std::path::{Path, PathBuf};

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use fixlog_format::{
    Encoding, LineEnding, LinePrefix, LogFormat, MessageBoundary, Separator, sniff,
};
use fixlog_parser::parse_all_with_format;

/// Workspace-root-relative path to the shared `fixtures/` directory.
fn fixture(rel: &str) -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest).join("../..").join(rel)
}

/// Load a fixture file into memory once. Benchmarks should not time I/O.
fn load(rel: &str) -> Vec<u8> {
    let path = fixture(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Sniff the format of `bytes`, panicking if the sniffer cannot identify it.
/// Benchmarks depend on the sniffer succeeding; if it ever regresses we want a loud failure.
fn sniff_format(bytes: &[u8]) -> LogFormat {
    sniff(bytes).expect("fixture must be sniffable")
}

/// Tight SOH-only layout. Used for the amplified synthetic buffer where we already know
/// the shape and want to skip the sniffer from the hot path.
fn soh_layout() -> LogFormat {
    LogFormat {
        separator: Separator::Soh,
        line_prefix: LinePrefix::None,
        encoding: Encoding::Utf8,
        line_ending: LineEnding::Lf,
        message_boundary: MessageBoundary::Line,
    }
}

/// Repeat `buf` until at least `target_bytes` bytes are produced.
///
/// The tiny synthetic fixtures (~1 KB) measure noise more than throughput. Amplifying to a
/// few MB gives criterion enough work per iteration to produce stable numbers.
fn amplify(buf: &[u8], target_bytes: usize) -> Vec<u8> {
    let copies = target_bytes.div_ceil(buf.len().max(1));
    let mut out = Vec::with_capacity(copies * buf.len());
    for _ in 0..copies {
        out.extend_from_slice(buf);
    }
    out
}

/// Drive `parse_all_with_format` to completion and return the number of messages consumed,
/// using `black_box` so the optimizer cannot elide the work.
fn parse_count(buf: &[u8], fmt: &LogFormat) -> usize {
    let mut count = 0usize;
    for m in parse_all_with_format(buf, fmt).flatten() {
        black_box(&m.tags);
        count += 1;
    }
    count
}

fn bench_synthetic(c: &mut Criterion) {
    let cases: &[(&str, &str)] = &[
        ("soh", "fixtures/synthetic/minimal_4.4.log"),
        ("pipe", "fixtures/synthetic/pipe_separated.log"),
        ("prefix", "fixtures/synthetic/with_timestamp_prefix.log"),
    ];

    let mut group = c.benchmark_group("parse_synthetic");
    const TARGET: usize = 4 * 1024 * 1024; // 4 MiB per iteration

    for (label, rel) in cases {
        let raw = load(rel);
        let fmt = sniff_format(&raw);
        let amplified = amplify(&raw, TARGET);
        group.throughput(Throughput::Bytes(amplified.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), &amplified, |b, buf| {
            b.iter(|| parse_count(black_box(buf), &fmt));
        });
    }

    group.finish();
}

fn bench_real(c: &mut Criterion) {
    let cases: &[(&str, &str)] = &[
        ("fix44_om", "fixtures/real/fix44-om.log"),
        ("fixt11_md", "fixtures/real/fixt11-md.log"),
    ];

    let mut group = c.benchmark_group("parse_real");

    for (label, rel) in cases {
        let raw = load(rel);
        let fmt = sniff_format(&raw);
        group.throughput(Throughput::Bytes(raw.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), &raw, |b, buf| {
            b.iter(|| parse_count(black_box(buf), &fmt));
        });
    }

    group.finish();
}

fn bench_known_soh(c: &mut Criterion) {
    // Isolates the tokenizer from the sniffer: fixed SOH layout, no prefix, single-line messages.
    // Any Phase-2 regression in the hot loop will show up here first.
    let raw = load("fixtures/synthetic/minimal_4.4.log");
    let amplified = amplify(&raw, 8 * 1024 * 1024);
    let fmt = soh_layout();

    let mut group = c.benchmark_group("parse_known_soh");
    group.throughput(Throughput::Bytes(amplified.len() as u64));
    group.bench_function("8MiB", |b| {
        b.iter(|| parse_count(black_box(&amplified), &fmt));
    });
    group.finish();
}

criterion_group!(benches, bench_synthetic, bench_real, bench_known_soh);
criterion_main!(benches);
