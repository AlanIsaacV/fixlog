//! Index build benchmarks: single-thread vs rayon-parallel.
//!
//! Two groups:
//! - `index_real/*`: real fixtures at their native size.
//! - `index_amplified/*`: real fixture amplified to ~40 MiB so the parallel path has enough
//!   work to actually win against the single-thread path.
//!
//! Run: `cargo bench -p fixlog-index --bench index`.

use std::path::{Path, PathBuf};

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use fixlog_format::{LogFormat, sniff};
use fixlog_index::{build_from_bytes, build_from_bytes_parallel};

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn load(rel: &str) -> Vec<u8> {
    std::fs::read(fixture(rel)).expect("fixture readable")
}

fn amplify(buf: &[u8], target: usize) -> Vec<u8> {
    let copies = target.div_ceil(buf.len().max(1));
    let mut out = Vec::with_capacity(copies * buf.len());
    for _ in 0..copies {
        out.extend_from_slice(buf);
    }
    out
}

fn sniff_or_panic(bytes: &[u8]) -> LogFormat {
    sniff(bytes).expect("sniffable")
}

fn bench_real(c: &mut Criterion) {
    let cases: &[(&str, &str)] = &[
        ("fix44_om", "fixtures/real/fix44-om.log"),
        ("fixt11_md", "fixtures/real/fixt11-md.log"),
    ];
    let mut group = c.benchmark_group("index_real");
    for (label, rel) in cases {
        let raw = load(rel);
        let fmt = sniff_or_panic(&raw);
        group.throughput(Throughput::Bytes(raw.len() as u64));
        group.bench_with_input(BenchmarkId::new("single_thread", label), &raw, |b, buf| {
            b.iter(|| black_box(build_from_bytes(buf, &fmt)))
        });
        group.bench_with_input(BenchmarkId::new("parallel", label), &raw, |b, buf| {
            b.iter(|| black_box(build_from_bytes_parallel(buf, &fmt)))
        });
    }
    group.finish();
}

fn bench_amplified(c: &mut Criterion) {
    let raw = load("fixtures/real/fix44-om.log");
    let fmt = sniff_or_panic(&raw);
    let amp = amplify(&raw, 40 * 1024 * 1024);
    let mut group = c.benchmark_group("index_amplified");
    group.throughput(Throughput::Bytes(amp.len() as u64));
    group.bench_function("single_thread_40MiB", |b| {
        b.iter(|| black_box(build_from_bytes(&amp, &fmt)))
    });
    group.bench_function("parallel_40MiB", |b| {
        b.iter(|| black_box(build_from_bytes_parallel(&amp, &fmt)))
    });
    group.finish();
}

criterion_group!(benches, bench_real, bench_amplified);
criterion_main!(benches);
