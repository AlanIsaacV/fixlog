//! `fixlog sessions <file>` — aggregate by `(SenderCompID, TargetCompID)` pair.

use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};
use fixlog_analysis::sessions::{SessionMap, SessionStats};
use fixlog_core::build_from_bytes_parallel;

use crate::ParseFormat;
use crate::io::{head, mmap_file};

const SNIFF_WINDOW: usize = 64 * 1024;

pub fn run(path: &Path, format: ParseFormat) -> Result<()> {
    let mmap = mmap_file(path)?;
    let log_format = fixlog_core::sniff(head(&mmap, SNIFF_WINDOW))
        .with_context(|| format!("sniffing {}", path.display()))?;
    let index = build_from_bytes_parallel(&mmap, &log_format);
    let map = SessionMap::build(&index, &mmap, &log_format);

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    match format {
        ParseFormat::Pretty => write_pretty(&mut out, &map)?,
        ParseFormat::Json => write_jsonl(&mut out, &map)?,
    }
    out.flush()?;

    if map.by_key.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn write_pretty<W: Write>(out: &mut W, map: &SessionMap) -> Result<()> {
    if map.by_key.is_empty() {
        writeln!(out, "(no sessions)")?;
        return Ok(());
    }
    // Sort entries for stable output. Canonical keys already collapse
    // direction, so a lex sort on `sender` then `target` is enough.
    let mut rows: Vec<_> = map.by_key.iter().collect();
    rows.sort_by(|(a, _), (b, _)| a.sender.cmp(&b.sender).then(a.target.cmp(&b.target)));

    writeln!(
        out,
        "{:<32} {:>8} {:>18} {:>16} {:>6}",
        "session", "msgs", "by-msg-type", "seq-range", "gaps"
    )?;
    writeln!(out, "{}", "-".repeat(32 + 8 + 18 + 16 + 6 + 4))?;
    for (key, stats) in rows {
        let session = format!("{} ↔ {}", lossy(&key.sender), lossy(&key.target),);
        let msgs = stats.in_count + stats.out_count;
        let by_type = format_msg_types(stats, 3);
        let seq_range = match (stats.seq_min, stats.seq_max) {
            (Some(mn), Some(mx)) => format!("{mn}..{mx}"),
            _ => "-".into(),
        };
        writeln!(
            out,
            "{:<32} {:>8} {:>18} {:>16} {:>6}",
            truncate(&session, 32),
            msgs,
            truncate(&by_type, 18),
            seq_range,
            stats.gaps.len(),
        )?;
    }
    Ok(())
}

fn write_jsonl<W: Write>(out: &mut W, map: &SessionMap) -> Result<()> {
    let mut rows: Vec<_> = map.by_key.iter().collect();
    rows.sort_by(|(a, _), (b, _)| a.sender.cmp(&b.sender).then(a.target.cmp(&b.target)));

    for (key, stats) in rows {
        write!(out, r#"{{"sender":""#)?;
        write_json_bytes(out, &key.sender)?;
        write!(out, r#"","target":""#)?;
        write_json_bytes(out, &key.target)?;
        write!(
            out,
            r#"","in_count":{},"out_count":{}"#,
            stats.in_count, stats.out_count
        )?;
        write!(out, r#","by_msg_type":{{"#)?;
        let mut first = true;
        let mut mt_entries: Vec<_> = stats.by_msg_type.iter().collect();
        mt_entries.sort_by(|a, b| a.0.as_slice().cmp(b.0.as_slice()));
        for (mt, count) in mt_entries {
            if !first {
                out.write_all(b",")?;
            }
            first = false;
            out.write_all(b"\"")?;
            write_json_bytes(out, mt)?;
            write!(out, "\":{count}")?;
        }
        write!(out, "}}")?;
        if let (Some(mn), Some(mx)) = (stats.seq_min, stats.seq_max) {
            write!(out, r#","seq_min":{mn},"seq_max":{mx}"#)?;
        }
        write!(out, r#","gaps":["#)?;
        for (i, g) in stats.gaps.iter().enumerate() {
            if i > 0 {
                out.write_all(b",")?;
            }
            write!(
                out,
                r#"{{"from_seq":{},"to_seq":{},"ordinal_before":{},"ordinal_after":{}}}"#,
                g.from_seq, g.to_seq, g.ordinal_before, g.ordinal_after
            )?;
        }
        out.write_all(b"]}\n")?;
    }
    Ok(())
}

fn format_msg_types(stats: &SessionStats, top_n: usize) -> String {
    let mut entries: Vec<_> = stats.by_msg_type.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1).then(a.0.as_slice().cmp(b.0.as_slice())));
    entries.truncate(top_n);
    let mut out = String::new();
    for (i, (mt, count)) in entries.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{}={count}", lossy(mt)));
    }
    out
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
