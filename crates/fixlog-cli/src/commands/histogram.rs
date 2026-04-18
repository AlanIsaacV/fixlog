//! `fixlog histogram <file>` — temporal histogram over SendingTime.

use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fixlog_analysis::histogram::Histogram;
use fixlog_core::build_from_bytes_parallel;

use crate::io::{head, mmap_file};

const SNIFF_WINDOW: usize = 64 * 1024;

pub fn run(path: &Path, bucket: &str, width: usize, peaks: usize) -> Result<()> {
    let bucket_dur = parse_duration(bucket)
        .ok_or_else(|| anyhow!("unrecognised duration: {bucket:?} (try `1s`, `500ms`, `2m`)"))?;

    let mmap = mmap_file(path)?;
    let log_format = fixlog_core::sniff(head(&mmap, SNIFF_WINDOW))
        .with_context(|| format!("sniffing {}", path.display()))?;
    let index = build_from_bytes_parallel(&mmap, &log_format);
    let h = Histogram::build(&index, &mmap, &log_format, bucket_dur);

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    writeln!(
        out,
        "total messages: {}   binned: {}   no-time: {}",
        h.total() + h.dropped_no_time as u64,
        h.total(),
        h.dropped_no_time,
    )?;
    writeln!(out, "bucket: {}", human_duration(bucket_dur))?;
    if h.bins.is_empty() {
        writeln!(out, "(no timestamped messages)")?;
        out.flush()?;
        return Ok(());
    }
    writeln!(
        out,
        "range:  {} → {}",
        h.bins.first().map(|b| b.start_ns).unwrap_or(0),
        h.bins.last().map(|b| b.end_ns).unwrap_or(0),
    )?;
    writeln!(out, "{}", h.render_sparkline(width))?;
    writeln!(out)?;
    writeln!(out, "top {} peaks (count · start_ns):", peaks)?;
    for bin in h.peaks(peaks) {
        writeln!(out, "  {:>8}  {}", bin.count, bin.start_ns)?;
    }
    out.flush()?;
    Ok(())
}

/// Parse `<N>(ms|s|m)`. Returns `None` on anything else.
pub(crate) fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (num_str, mult_ns) = if let Some(rest) = s.strip_suffix("ms") {
        (rest, 1_000_000_u128)
    } else if let Some(rest) = s.strip_suffix("us") {
        (rest, 1_000_u128)
    } else if let Some(rest) = s.strip_suffix("ns") {
        (rest, 1_u128)
    } else if let Some(rest) = s.strip_suffix('s') {
        (rest, 1_000_000_000_u128)
    } else if let Some(rest) = s.strip_suffix('m') {
        (rest, 60_000_000_000_u128)
    } else {
        return None;
    };
    let n: u64 = num_str.trim().parse().ok()?;
    let total_ns = (n as u128).checked_mul(mult_ns)?;
    let secs = (total_ns / 1_000_000_000) as u64;
    let nanos = (total_ns % 1_000_000_000) as u32;
    Some(Duration::new(secs, nanos))
}

fn human_duration(d: Duration) -> String {
    let ns = d.as_nanos();
    if ns.is_multiple_of(60_000_000_000) {
        format!("{}m", ns / 60_000_000_000)
    } else if ns.is_multiple_of(1_000_000_000) {
        format!("{}s", ns / 1_000_000_000)
    } else if ns.is_multiple_of(1_000_000) {
        format!("{}ms", ns / 1_000_000)
    } else if ns.is_multiple_of(1_000) {
        format!("{}us", ns / 1_000)
    } else {
        format!("{ns}ns")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_durations() {
        assert_eq!(parse_duration("1s"), Some(Duration::from_secs(1)));
        assert_eq!(parse_duration("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(parse_duration("2m"), Some(Duration::from_secs(120)));
        assert_eq!(parse_duration("100us"), Some(Duration::from_micros(100)));
        assert_eq!(parse_duration("  1s  "), Some(Duration::from_secs(1)));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_duration("nope").is_none());
        assert!(parse_duration("1").is_none());
        assert!(parse_duration("").is_none());
    }
}
