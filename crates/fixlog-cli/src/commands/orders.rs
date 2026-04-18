//! `fixlog orders <file> [--id CLORDID]` — order lifecycle reconstruction.

use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use fixlog_analysis::orders::{OrderTimeline, render_gantt};
use fixlog_core::index::secondary::TAG_CL_ORD_ID;
use fixlog_core::{LogFormat, LogIndex, build_from_bytes_parallel, parse_one_with_format};

use crate::ParseFormat;
use crate::io::{head, mmap_file};

const SNIFF_WINDOW: usize = 64 * 1024;
const GANTT_WIDTH: usize = 60;

pub fn run(path: &Path, id: Option<&str>, limit: usize, format: ParseFormat) -> Result<()> {
    let mmap = mmap_file(path)?;
    let log_format = fixlog_core::sniff(head(&mmap, SNIFF_WINDOW))
        .with_context(|| format!("sniffing {}", path.display()))?;
    let index = build_from_bytes_parallel(&mmap, &log_format);

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    match id {
        Some(clordid) => {
            let tl = OrderTimeline::build(&index, &mmap, &log_format, clordid.as_bytes())
                .ok_or_else(|| anyhow!("no events found for ClOrdID={clordid}"))?;
            match format {
                ParseFormat::Pretty => write_timeline_pretty(&mut out, &tl)?,
                ParseFormat::Json => write_timeline_json(&mut out, &tl)?,
            }
        }
        None => {
            let counts = collect_clordid_counts(&mmap, &index, &log_format);
            let mut entries: Vec<_> = counts.into_iter().collect();
            entries.sort_by_key(|b| std::cmp::Reverse(b.1));
            entries.truncate(limit);
            match format {
                ParseFormat::Pretty => write_index_pretty(&mut out, &entries)?,
                ParseFormat::Json => write_index_json(&mut out, &entries)?,
            }
            if entries.is_empty() {
                out.flush()?;
                std::process::exit(1);
            }
        }
    }
    out.flush()?;
    Ok(())
}

/// Scan the index once and bucket messages by their first `tag 11` value.
///
/// The secondary index already has every (tag, value) combination
/// pre-grouped, but doesn't expose its key set via the public API. A
/// linear scan is fine here — the parser runs at ~300 MiB/s and this is
/// a one-shot CLI command.
fn collect_clordid_counts(
    buf: &[u8],
    index: &LogIndex,
    format: &LogFormat,
) -> HashMap<Vec<u8>, u32> {
    let mut counts: HashMap<Vec<u8>, u32> = HashMap::new();
    for ord in 0..index.len() {
        let Some(bytes) = index.message_bytes(buf, ord) else {
            continue;
        };
        let Ok((msg, _)) = parse_one_with_format(bytes, format) else {
            continue;
        };
        for (t, v) in &msg.tags {
            if *t == TAG_CL_ORD_ID {
                *counts.entry(v.to_vec()).or_insert(0) += 1;
                break;
            }
        }
    }
    counts
}

fn write_timeline_pretty<W: Write>(out: &mut W, tl: &OrderTimeline) -> Result<()> {
    writeln!(
        out,
        "ClOrdID: {}   OrderID(s): [{}]   events: {}",
        lossy(&tl.clordid),
        tl.order_ids
            .iter()
            .map(|o| lossy(o).into_owned())
            .collect::<Vec<_>>()
            .join(", "),
        tl.events.len(),
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "{:>8}  {:>6}  {:>8}  {:>8}  {:>8}",
        "ordinal", "type", "exec", "status", "cum-qty"
    )?;
    writeln!(out, "{}", "-".repeat(8 + 6 + 8 + 8 + 8 + 8))?;
    for e in &tl.events {
        writeln!(
            out,
            "{:>8}  {:>6}  {:>8}  {:>8}  {:>8}",
            e.ordinal,
            lossy(&e.msg_type),
            e.exec_type
                .as_ref()
                .map(|v| lossy(v).into_owned())
                .unwrap_or_else(|| "-".into()),
            e.ord_status
                .as_ref()
                .map(|v| lossy(v).into_owned())
                .unwrap_or_else(|| "-".into()),
            e.cum_qty
                .as_ref()
                .map(|v| lossy(v).into_owned())
                .unwrap_or_else(|| "-".into()),
        )?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "Gantt ({GANTT_WIDTH} cols, N=D X=8 C=F R=G !=3/j ?=other):"
    )?;
    writeln!(out, "  {}", render_gantt(tl, GANTT_WIDTH))?;
    Ok(())
}

fn write_timeline_json<W: Write>(out: &mut W, tl: &OrderTimeline) -> Result<()> {
    write!(out, r#"{{"clordid":""#)?;
    write_json_bytes(out, &tl.clordid)?;
    out.write_all(br#"","order_ids":["#)?;
    for (i, oid) in tl.order_ids.iter().enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        out.write_all(b"\"")?;
        write_json_bytes(out, oid)?;
        out.write_all(b"\"")?;
    }
    out.write_all(br#"],"events":["#)?;
    for (i, e) in tl.events.iter().enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        write!(out, r#"{{"ordinal":{},"msg_type":""#, e.ordinal)?;
        write_json_bytes(out, &e.msg_type)?;
        out.write_all(b"\"")?;
        if let Some(v) = e.exec_type.as_ref() {
            out.write_all(br#","exec_type":""#)?;
            write_json_bytes(out, v)?;
            out.write_all(b"\"")?;
        }
        if let Some(v) = e.ord_status.as_ref() {
            out.write_all(br#","ord_status":""#)?;
            write_json_bytes(out, v)?;
            out.write_all(b"\"")?;
        }
        if let Some(v) = e.cum_qty.as_ref() {
            out.write_all(br#","cum_qty":""#)?;
            write_json_bytes(out, v)?;
            out.write_all(b"\"")?;
        }
        out.write_all(b"}")?;
    }
    out.write_all(b"]}\n")?;
    Ok(())
}

fn write_index_pretty<W: Write>(out: &mut W, entries: &[(Vec<u8>, u32)]) -> Result<()> {
    if entries.is_empty() {
        writeln!(out, "(no orders)")?;
        return Ok(());
    }
    writeln!(out, "{:<40} {:>8}", "clordid", "events")?;
    writeln!(out, "{}", "-".repeat(40 + 8 + 1))?;
    for (id, count) in entries {
        writeln!(out, "{:<40} {:>8}", truncate(&lossy(id), 40), count)?;
    }
    Ok(())
}

fn write_index_json<W: Write>(out: &mut W, entries: &[(Vec<u8>, u32)]) -> Result<()> {
    for (id, count) in entries {
        write!(out, r#"{{"clordid":""#)?;
        write_json_bytes(out, id)?;
        writeln!(out, r#"","events":{count}}}"#)?;
    }
    Ok(())
}

fn lossy(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn write_json_bytes<W: Write>(out: &mut W, bytes: &[u8]) -> std::io::Result<()> {
    let s = String::from_utf8_lossy(bytes);
    for c in s.chars() {
        match c {
            '"' => out.write_all(br#"\""#)?,
            '\\' => out.write_all(br"\\")?,
            '\n' => out.write_all(br"\n")?,
            '\r' => out.write_all(br"\r")?,
            '\t' => out.write_all(br"\t")?,
            c if (c as u32) < 0x20 => write!(out, "\\u{:04x}", c as u32)?,
            c => {
                let mut buf = [0u8; 4];
                out.write_all(c.encode_utf8(&mut buf).as_bytes())?;
            }
        }
    }
    Ok(())
}
