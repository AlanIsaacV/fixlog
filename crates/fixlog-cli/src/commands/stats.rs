//! `fixlog stats <file>` — print a summary of a log file.
//!
//! Computes: total messages parsed, number of parse errors, top-N MsgType
//! breakdown with human-readable names, unique `(SenderCompID, TargetCompID)`
//! session pairs, and the min/max `SendingTime` (tag 52) values seen.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use fixlog_core::dict::{chain_for, chain_msg_type_label};
use fixlog_core::parser::{
    TAG_BEGIN_STRING, TAG_MSG_TYPE, TAG_SENDER_COMP_ID, TAG_SENDING_TIME, TAG_TARGET_COMP_ID,
};
use fixlog_core::{RawMessage, parse_all_with_format};

use crate::io::{head, mmap_file};

const SNIFF_WINDOW: usize = 64 * 1024;
const TAG_APPL_VER_ID: u32 = 1128;
const MSG_TYPE_TOP_N: usize = 10;

pub fn run(path: &Path) -> Result<()> {
    let mmap = mmap_file(path)?;
    let log_format = fixlog_core::sniff(head(&mmap, SNIFF_WINDOW))
        .with_context(|| format!("sniffing {}", path.display()))?;

    let mut stats = Stats::default();
    for result in parse_all_with_format(&mmap, &log_format) {
        match result {
            Ok(msg) => stats.record(&msg),
            Err(_) => stats.errors += 1,
        }
    }
    stats.print(path);
    Ok(())
}

#[derive(Default)]
struct Stats {
    total: u64,
    errors: u64,
    /// MsgType wire value → occurrences.
    msg_type_counts: BTreeMap<Vec<u8>, u64>,
    /// (sender, target) unique session pairs.
    sessions: BTreeMap<(Vec<u8>, Vec<u8>), u64>,
    /// Lexicographic min/max of SendingTime (tag 52). Works for the standard
    /// `YYYYMMDD-HH:MM:SS[.sss]` format because it sorts identically to a
    /// real chronological order.
    min_time: Option<Vec<u8>>,
    max_time: Option<Vec<u8>>,
    /// Last-seen session hints for dictionary chain selection in the summary.
    last_begin_string: Option<Vec<u8>>,
    last_appl_ver_id: Option<Vec<u8>>,
}

impl Stats {
    fn record(&mut self, msg: &RawMessage<'_>) {
        self.total += 1;
        let mut sender: Option<&[u8]> = None;
        let mut target: Option<&[u8]> = None;
        for &(tag, value) in &msg.tags {
            match tag {
                TAG_BEGIN_STRING => self.last_begin_string = Some(value.to_vec()),
                TAG_MSG_TYPE => {
                    *self.msg_type_counts.entry(value.to_vec()).or_default() += 1;
                }
                TAG_SENDER_COMP_ID => sender = Some(value),
                TAG_TARGET_COMP_ID => target = Some(value),
                TAG_SENDING_TIME => self.update_time_range(value),
                TAG_APPL_VER_ID => self.last_appl_ver_id = Some(value.to_vec()),
                _ => {}
            }
        }
        if let (Some(s), Some(t)) = (sender, target) {
            *self.sessions.entry((s.to_vec(), t.to_vec())).or_default() += 1;
        }
    }

    fn update_time_range(&mut self, value: &[u8]) {
        match &mut self.min_time {
            Some(current) if value < current.as_slice() => *current = value.to_vec(),
            None => self.min_time = Some(value.to_vec()),
            _ => {}
        }
        match &mut self.max_time {
            Some(current) if value > current.as_slice() => *current = value.to_vec(),
            None => self.max_time = Some(value.to_vec()),
            _ => {}
        }
    }

    fn print(&self, path: &Path) {
        let chain = chain_for(
            self.last_begin_string.as_deref().unwrap_or(b""),
            self.last_appl_ver_id.as_deref(),
        );

        println!("File:            {}", path.display());
        println!("Messages parsed: {}", self.total);
        println!("Parse errors:    {}", self.errors);
        if let (Some(min), Some(max)) = (&self.min_time, &self.max_time) {
            println!("Time range:      {} .. {}", lossy(min), lossy(max));
        }

        println!("Sessions:        {}", self.sessions.len());
        // Sort sessions by traffic desc, then by sender/target for stability.
        let mut sessions: Vec<_> = self.sessions.iter().collect();
        sessions.sort_by(|(ak, av), (bk, bv)| bv.cmp(av).then_with(|| ak.cmp(bk)));
        for ((s, t), count) in sessions {
            println!("  {:>10}  {} → {}", count, lossy(s), lossy(t));
        }

        println!(
            "Message types:   {} ({} shown)",
            self.msg_type_counts.len(),
            self.msg_type_counts.len().min(MSG_TYPE_TOP_N),
        );
        let mut mtypes: Vec<_> = self.msg_type_counts.iter().collect();
        mtypes.sort_by(|(ak, av), (bk, bv)| bv.cmp(av).then_with(|| ak.cmp(bk)));
        for (mt, count) in mtypes.iter().take(MSG_TYPE_TOP_N) {
            let label = chain_msg_type_label(chain, mt).unwrap_or("?");
            println!("  {:>10}  {:<4} {}", count, lossy(mt), label);
        }
    }
}

fn lossy(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}
