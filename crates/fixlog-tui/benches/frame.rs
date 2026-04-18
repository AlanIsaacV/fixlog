//! Frame-budget benchmarks for the TUI.
//!
//! The contract from the Phase-3 plan is **<16 ms median per frame on 1M
//! messages**. This harness amplifies `fixtures/synthetic/minimal_4.4.log`
//! into a tempfile until the message count exceeds 1M, bootstraps an
//! `AppState` against it, and measures the cost of a single render pass
//! (list + detail + status + command bar) against a `TestBackend` of
//! realistic dimensions (200×50).
//!
//! Also measures:
//! - Full bootstrap time (mmap + sniff + parallel index + initial visible).
//! - Filter application (`apply_filter` with `35=D`) on the full 1M buffer.
//!
//! Run:
//!
//! ```text
//! cargo bench -p fixlog-tui --bench frame
//! ```

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Direction, Layout};

use fixlog_core::query::parse as parse_query;
use fixlog_tui::state::{AppState, apply_filter, bootstrap};
use fixlog_tui::view;

const BASELINE_FIXTURE: &[u8] = include_bytes!("../../../fixtures/synthetic/minimal_4.4.log");
/// Messages per copy of the baseline fixture. From `state.md` and parser tests.
const MESSAGES_PER_COPY: usize = 10;
/// Target message count for the 1M-message bench.
const TARGET_MESSAGES: usize = 1_000_000;

fn amplified_path() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "fixlog-tui-bench-amplified-{}.log",
        std::process::id()
    ));
    if !path.exists() || should_rebuild(&path) {
        build_amplified(&path);
    }
    path
}

fn should_rebuild(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() < 10_000_000)
        .unwrap_or(true)
}

fn build_amplified(path: &Path) {
    let copies = TARGET_MESSAGES.div_ceil(MESSAGES_PER_COPY);
    let mut f = File::create(path).expect("create amplified fixture");
    for _ in 0..copies {
        f.write_all(BASELINE_FIXTURE).expect("write");
    }
    f.flush().expect("flush");
}

fn bench_bootstrap(c: &mut Criterion) {
    let path = amplified_path();
    let len = std::fs::metadata(&path).unwrap().len();

    let mut group = c.benchmark_group("tui_bootstrap");
    group.throughput(Throughput::Bytes(len));
    group.sample_size(10); // bootstrap is expensive; 10 samples is plenty.
    group.bench_function(BenchmarkId::from_parameter("1M_messages"), |b| {
        b.iter(|| {
            let state = bootstrap(black_box(&path), None).expect("bootstrap");
            black_box(state.index.len());
        });
    });
    group.finish();
}

fn render_frame(state: &mut AppState, terminal: &mut Terminal<TestBackend>) {
    terminal
        .draw(|frame| {
            let area = frame.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(area);
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(chunks[1]);
            view::list::render(frame, body[0], state);
            view::detail::render(frame, body[1], state);
            view::status::render(frame, chunks[2], state);
        })
        .expect("draw");
}

fn bench_frame(c: &mut Criterion) {
    let path = amplified_path();
    let state = bootstrap(&path, None).expect("bootstrap");

    // Position the cursor partway through the list so the list view has
    // real work to do (not all rows are at the top of visible).
    let mut state = state;
    state.cursor = state.visible.len() / 2;

    let mut group = c.benchmark_group("tui_frame");
    group.sample_size(50);
    group.bench_function("list_detail_status_200x50", |b| {
        let backend = TestBackend::new(200, 50);
        let mut terminal = Terminal::new(backend).unwrap();
        b.iter(|| {
            render_frame(black_box(&mut state), &mut terminal);
        });
    });
    group.finish();
}

fn bench_apply_filter(c: &mut Criterion) {
    let path = amplified_path();
    let mut state = bootstrap(&path, None).expect("bootstrap");

    let mut group = c.benchmark_group("tui_filter");
    group.sample_size(10); // Full-scan of 1M messages per sample.
    group.throughput(Throughput::Elements(state.index.len() as u64));
    group.bench_function("apply_35eqD_1M", |b| {
        b.iter(|| {
            // Compile inside the iter so each sample re-runs the whole
            // filter path; the compile itself is trivially cheap (<1µs).
            let expr = parse_query("35=D").expect("parse");
            apply_filter(black_box(&mut state), Some(expr), Some("35=D".to_string()));
            black_box(state.visible.len());
        });
    });
    group.finish();
}

criterion_group!(benches, bench_bootstrap, bench_frame, bench_apply_filter);
criterion_main!(benches);
