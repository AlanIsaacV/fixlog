//! Temporal histogram over SendingTime (tag 52).
//!
//! Buckets are uniform width (`bucket_ns`) and span `[t_min, t_max]`
//! rounded down/up to whole buckets. Messages without a parseable tag 52
//! are silently dropped (counted against `dropped_no_time`, exposed on
//! [`Histogram::dropped_no_time`]).

use std::time::Duration;

use fixlog_core::parser::TAG_SENDING_TIME;
use fixlog_core::{LogFormat, LogIndex};
use rayon::prelude::*;

use crate::util::{extract_tag_raw, parse_sending_time, system_time_to_nanos};

/// One uniform-width time bucket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bin {
    pub start_ns: u128,
    pub end_ns: u128,
    pub count: u32,
}

/// Uniform-width histogram of message timestamps.
#[derive(Clone, Debug, Default)]
pub struct Histogram {
    pub bucket_ns: u64,
    pub bins: Vec<Bin>,
    /// How many messages lacked a parseable tag 52 and were dropped.
    pub dropped_no_time: u32,
}

impl Histogram {
    /// Build a histogram over `index.messages`.
    ///
    /// `bucket` is clamped to a minimum of 1 ns to avoid division by zero.
    /// An empty index (or an index where no message has a valid timestamp)
    /// produces a histogram with an empty `bins` vector.
    ///
    /// Implementation notes — the hot path:
    /// - Timestamps are extracted with a narrow scan
    ///   ([`extract_tag_raw`]) that short-circuits on the first match,
    ///   avoiding the per-message allocation and full tokenization that
    ///   a [`parse_one_with_format`] call would incur.
    /// - The per-message extraction runs in parallel over `rayon` since
    ///   it's embarrassingly parallel and dominated the old build time.
    /// - The reduction stage computes `t_min` / `t_max` and collects
    ///   valid timestamps + drop count in a single pass, without a
    ///   separate `.iter().min()` / `.iter().max()` scan.
    pub fn build(index: &LogIndex, buf: &[u8], format: &LogFormat, bucket: Duration) -> Self {
        let bucket_ns_u64 = bucket.as_nanos().max(1).min(u64::MAX as u128) as u64;
        let bucket_ns = bucket_ns_u64 as u128;

        // Per-ordinal timestamp extraction. `None` means the message
        // lacked a parseable tag 52 (or was out of range) and must count
        // against `dropped_no_time`.
        let results: Vec<Option<u128>> = (0..index.len())
            .into_par_iter()
            .map(|ord| {
                let bytes = index.message_bytes(buf, ord)?;
                let raw = extract_tag_raw(bytes, format, TAG_SENDING_TIME)?;
                parse_sending_time(raw).and_then(system_time_to_nanos)
            })
            .collect();

        let mut times: Vec<u128> = Vec::with_capacity(results.len());
        let mut dropped: u32 = 0;
        let mut t_min: u128 = u128::MAX;
        let mut t_max: u128 = 0;
        for r in results {
            match r {
                Some(t) => {
                    if t < t_min {
                        t_min = t;
                    }
                    if t > t_max {
                        t_max = t;
                    }
                    times.push(t);
                }
                None => {
                    dropped = dropped.saturating_add(1);
                }
            }
        }

        if times.is_empty() {
            return Self {
                bucket_ns: bucket_ns_u64,
                bins: Vec::new(),
                dropped_no_time: dropped,
            };
        }

        let start = (t_min / bucket_ns) * bucket_ns;
        let end = ((t_max / bucket_ns) + 1) * bucket_ns;
        let n_bins = ((end - start) / bucket_ns) as usize;

        let mut bins: Vec<Bin> = (0..n_bins)
            .map(|i| {
                let bs = start + (i as u128) * bucket_ns;
                Bin {
                    start_ns: bs,
                    end_ns: bs + bucket_ns,
                    count: 0,
                }
            })
            .collect();

        for t in times {
            let idx = ((t - start) / bucket_ns) as usize;
            let idx = idx.min(n_bins - 1);
            bins[idx].count = bins[idx].count.saturating_add(1);
        }

        Self {
            bucket_ns: bucket_ns_u64,
            bins,
            dropped_no_time: dropped,
        }
    }

    /// Render a Unicode-block sparkline (`" ▁▂▃▄▅▆▇█"`) of `width`
    /// characters.
    ///
    /// Heights are mapped by **percentile**, not by raw max, so a single
    /// peak bucket doesn't flatten the rest of the chart.
    pub fn render_sparkline(&self, width: usize) -> String {
        if self.bins.is_empty() || width == 0 {
            return String::new();
        }
        const LEVELS: &[char] = &[' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

        // Merge bins into `width` buckets by summing counts.
        let n = self.bins.len();
        let mut merged: Vec<u32> = Vec::with_capacity(width);
        for i in 0..width {
            let lo = (i * n) / width;
            let hi = ((i + 1) * n) / width;
            let hi = hi.max(lo + 1).min(n);
            let sum: u32 = self.bins[lo..hi].iter().map(|b| b.count).sum();
            merged.push(sum);
        }

        // Percentile-based scale: find p95, use that as the "full" mark.
        // If every bucket has the same count, spark it at mid-level.
        let mut sorted = merged.clone();
        sorted.sort_unstable();
        let p95 = sorted[(sorted.len() * 95).saturating_sub(1) / 100];
        let scale = if p95 == 0 { 1 } else { p95 };

        merged
            .iter()
            .map(|&c| {
                let idx = ((c as u64 * (LEVELS.len() as u64 - 1)) / scale as u64)
                    .min(LEVELS.len() as u64 - 1) as usize;
                LEVELS[idx]
            })
            .collect()
    }

    /// Top-`k` bins by count, descending. Ties broken by earlier
    /// `start_ns`. Returns at most `self.bins.len()` entries.
    pub fn peaks(&self, k: usize) -> Vec<&Bin> {
        let mut refs: Vec<&Bin> = self.bins.iter().filter(|b| b.count > 0).collect();
        refs.sort_by(|a, b| b.count.cmp(&a.count).then(a.start_ns.cmp(&b.start_ns)));
        refs.truncate(k);
        refs
    }

    /// Sum of counts across all bins. Equals the number of messages that
    /// contributed a valid timestamp.
    pub fn total(&self) -> u64 {
        self.bins.iter().map(|b| b.count as u64).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixlog_core::{build_from_bytes, sniff};

    fn build_msg(body_fields: &str) -> Vec<u8> {
        let body_len = body_fields.len();
        let head = format!("8=FIX.4.4\x019={body_len}\x01");
        let payload: Vec<u8> = head.bytes().chain(body_fields.bytes()).collect();
        let sum: u8 = payload.iter().fold(0u8, |a, b| a.wrapping_add(*b));
        let trailer = format!("10={sum:03}\x01");
        payload.into_iter().chain(trailer.bytes()).collect()
    }

    fn synthetic_timestamps(seconds: &[u32]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, s) in seconds.iter().enumerate() {
            out.extend(build_msg(&format!(
                "35=0\x0134={}\x0149=A\x0156=B\x0152=20260417-12:00:{s:02}\x01",
                i + 1
            )));
        }
        out
    }

    #[test]
    fn empty_index_has_empty_bins() {
        // Derive a format from a known sniffable sample so we don't have
        // to construct `LogFormat` by hand.
        let sample = synthetic_timestamps(&[0]);
        let fmt = sniff(&sample).expect("sniff sample");
        let empty: Vec<u8> = Vec::new();
        let index = build_from_bytes(&empty, &fmt);
        let h = Histogram::build(&index, &empty, &fmt, Duration::from_secs(1));
        assert!(h.bins.is_empty());
        assert_eq!(h.total(), 0);
    }

    #[test]
    fn counts_match_input_and_cover_range() {
        let buf = synthetic_timestamps(&[0, 1, 2, 3, 4, 10]);
        let fmt = sniff(&buf).expect("sniff");
        let index = build_from_bytes(&buf, &fmt);
        let h = Histogram::build(&index, &buf, &fmt, Duration::from_secs(1));
        assert_eq!(h.total(), 6);
        assert_eq!(h.bucket_ns, 1_000_000_000);
        // 11 bins covering [0s, 11s) at 1s granularity.
        assert_eq!(h.bins.len(), 11);
        assert_eq!(h.bins[0].count, 1); // 0s
        assert_eq!(h.bins[1].count, 1); // 1s
        assert_eq!(h.bins[5].count, 0); // 5s: empty
        assert_eq!(h.bins[10].count, 1); // 10s
    }

    #[test]
    fn sparkline_width_and_empty() {
        let buf = synthetic_timestamps(&[0, 1, 2, 3, 4]);
        let fmt = sniff(&buf).expect("sniff");
        let index = build_from_bytes(&buf, &fmt);
        let h = Histogram::build(&index, &buf, &fmt, Duration::from_secs(1));
        let spark = h.render_sparkline(20);
        assert_eq!(
            spark.chars().count(),
            20,
            "sparkline should match requested width in chars"
        );
        assert_eq!(h.render_sparkline(0), "");
    }

    #[test]
    fn peaks_returns_top_k() {
        let buf = synthetic_timestamps(&[0, 0, 0, 1, 2]);
        let fmt = sniff(&buf).expect("sniff");
        let index = build_from_bytes(&buf, &fmt);
        let h = Histogram::build(&index, &buf, &fmt, Duration::from_secs(1));
        let peaks = h.peaks(2);
        assert_eq!(peaks.len(), 2);
        assert_eq!(peaks[0].count, 3);
        assert!(peaks[1].count <= 1);
    }

    #[test]
    fn bucket_clamped_to_minimum() {
        let buf = synthetic_timestamps(&[0]);
        let fmt = sniff(&buf).expect("sniff");
        let index = build_from_bytes(&buf, &fmt);
        let h = Histogram::build(&index, &buf, &fmt, Duration::from_nanos(0));
        assert!(h.bucket_ns >= 1);
        assert_eq!(h.total(), 1);
    }
}
