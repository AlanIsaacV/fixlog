#![forbid(unsafe_code)]

//! Shared rendering helpers for FIX messages.
//!
//! Before this crate existed, identical (~100 LOC) copies of `write_pretty`
//! and `write_jsonl` lived in `fixlog-cli/src/commands/parse.rs` and
//! `fixlog-tui/src/export.rs`, and a CSV export plus a raw-FIX passthrough
//! were inlined in the TUI export pipeline. Promoting them here lets any
//! future consumer (grep output, additional CLI subcommands, batch export
//! tooling) reuse the same output formats without having to re-derive
//! escape conventions or column widths.
//!
//! All writers emit to a generic `Write` and return `io::Result<()>`. They
//! do not allocate outside of transient `String::from_utf8_lossy` for
//! non-UTF-8 byte values — consistent with the rest of fixlog's zero-copy
//! style.
//!
//! # Formats
//!
//! - [`write_pretty`] — aligned `tag name = value [(label)]` table, one
//!   message per block. Matches `fixlog parse --format pretty`.
//! - [`write_jsonl`] — one JSON object per line, keys `offset`, `raw_len`,
//!   optional `msg_type_name`, and a `tags` array. Matches
//!   `fixlog parse --format json`.
//! - [`write_fix`] — raw message bytes followed by a trailing `\n`. Used by
//!   the TUI `:export fix` command to produce a re-playable log.
//! - [`write_csv_header`] + [`write_csv_row`] — fixed schema
//!   `ordinal,offset,msg_type,sender,target,seq_num,sending_time`. Rows
//!   escape commas, quotes, and newlines per RFC 4180.

use std::io::{self, Write};

use fixlog_dict::{ResolvedField, ResolvedMessage};
use fixlog_parser::{
    RawMessage, TAG_MSG_SEQ_NUM, TAG_MSG_TYPE, TAG_SENDER_COMP_ID, TAG_SENDING_TIME,
    TAG_TARGET_COMP_ID,
};

/// Write a resolved message as a human-readable table block.
///
/// Columns are aligned so known and unknown tags sit on the same grid.
/// `raw_len` is included in the header so readers can correlate output
/// with raw byte counts.
pub fn write_pretty<W: Write>(
    out: &mut W,
    msg: &ResolvedMessage<'_>,
    raw_len: usize,
) -> io::Result<()> {
    let header = match msg.msg_type_name {
        Some(name) => format!(" {name}"),
        None => String::new(),
    };
    writeln!(
        out,
        "Message @ offset {} ({} bytes){}",
        msg.offset, raw_len, header,
    )?;
    let name_width = msg
        .fields
        .iter()
        .map(|f| f.name.map(str::len).unwrap_or(0))
        .max()
        .unwrap_or(0);
    for f in &msg.fields {
        write_pretty_field(out, f, name_width)?;
    }
    writeln!(out)?;
    Ok(())
}

fn write_pretty_field<W: Write>(
    out: &mut W,
    f: &ResolvedField<'_>,
    name_width: usize,
) -> io::Result<()> {
    let name = f.name.unwrap_or("?");
    match f.value_label {
        Some(label) => writeln!(
            out,
            "  {:>5}  {:<width$} = {} ({})",
            f.tag,
            name,
            lossy(f.value),
            label,
            width = name_width,
        ),
        None => writeln!(
            out,
            "  {:>5}  {:<width$} = {}",
            f.tag,
            name,
            lossy(f.value),
            width = name_width,
        ),
    }
}

/// Write a resolved message as one JSONL record. Matches `fixlog parse
/// --format json` exactly, character for character, so downstream `jq`
/// pipelines and integrations see a stable schema across CLI and TUI.
pub fn write_jsonl<W: Write>(
    out: &mut W,
    msg: &ResolvedMessage<'_>,
    raw_len: usize,
) -> io::Result<()> {
    write!(out, r#"{{"offset":{},"raw_len":{}"#, msg.offset, raw_len)?;
    if let Some(name) = msg.msg_type_name {
        out.write_all(br#","msg_type_name":""#)?;
        write_json_string(out, name.as_bytes())?;
        out.write_all(b"\"")?;
    }
    out.write_all(br#","tags":["#)?;
    for (i, f) in msg.fields.iter().enumerate() {
        if i > 0 {
            out.write_all(b",")?;
        }
        write!(out, r#"{{"tag":{},"#, f.tag)?;
        match f.name {
            Some(n) => {
                out.write_all(br#""name":""#)?;
                write_json_string(out, n.as_bytes())?;
                out.write_all(br#"","#)?;
            }
            None => out.write_all(br#""name":null,"#)?,
        }
        out.write_all(br#""value":""#)?;
        write_json_string(out, f.value)?;
        out.write_all(b"\"")?;
        if let Some(label) = f.value_label {
            out.write_all(br#","value_label":""#)?;
            write_json_string(out, label.as_bytes())?;
            out.write_all(b"\"")?;
        }
        out.write_all(b"}")?;
    }
    out.write_all(b"]}\n")?;
    Ok(())
}

/// Write the raw message bytes followed by a newline separator. Intended
/// for `:export fix` to produce a log the user can re-feed through the
/// parser or a second tool.
pub fn write_fix<W: Write>(out: &mut W, bytes: &[u8]) -> io::Result<()> {
    out.write_all(bytes)?;
    out.write_all(b"\n")
}

/// Write the CSV header row used by [`write_csv_row`].
///
/// Fixed schema — if consumers need a different column set, they should
/// build a new writer rather than re-using this one, to keep the pairing
/// of header and row definitions obvious.
pub fn write_csv_header<W: Write>(out: &mut W) -> io::Result<()> {
    writeln!(
        out,
        "ordinal,offset,msg_type,sender,target,seq_num,sending_time"
    )
}

/// Write one CSV row summarising a raw message. `ordinal` and `offset` are
/// passed in separately because ordinals are a view-layer concept (index
/// position) while `offset` may come from either the index or the message
/// itself; keeping them as explicit arguments avoids coupling the renderer
/// to any specific index type.
///
/// Values are extracted from `msg.tags` via linear scan — fine for export
/// paths where rendering cost is dominated by I/O, not tag lookup.
pub fn write_csv_row<W: Write>(
    out: &mut W,
    ordinal: u32,
    offset: u64,
    msg: &RawMessage<'_>,
) -> io::Result<()> {
    let tag = |t| {
        msg.tags
            .iter()
            .find(|(tt, _)| *tt == t)
            .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_default()
    };
    writeln!(
        out,
        "{ordinal},{offset},{},{},{},{},{}",
        csv_escape(&tag(TAG_MSG_TYPE)),
        csv_escape(&tag(TAG_SENDER_COMP_ID)),
        csv_escape(&tag(TAG_TARGET_COMP_ID)),
        csv_escape(&tag(TAG_MSG_SEQ_NUM)),
        csv_escape(&tag(TAG_SENDING_TIME)),
    )
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

fn lossy(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

/// Escape `bytes` as a JSON string fragment (without the surrounding
/// quotes). Non-UTF-8 bytes fall through `String::from_utf8_lossy` to the
/// replacement character — matches the parse/grep CLI conventions.
fn write_json_string<W: Write>(out: &mut W, bytes: &[u8]) -> io::Result<()> {
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
