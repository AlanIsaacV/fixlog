//! Export the currently-visible message set to disk.
//!
//! Called from `:export <fmt> <path>`. Writes are **synchronous** — the
//! event loop blocks during large exports. For >100K messages we preview a
//! warning in `StatusMessage` before committing; see P4-T12 for the UX
//! decision.
//!
//! Formats:
//!
//! - `csv`    — `ordinal,offset,msg_type,sender,target,seq_num,sending_time`
//! - `json`   — one JSON object per line, matching `fixlog parse --format json`
//! - `fix`    — raw bytes per message (zero-copy from mmap), `\n` separator
//! - `pretty` — table layout, matching `fixlog parse --format pretty`

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use fixlog_core::parser::{
    TAG_MSG_SEQ_NUM, TAG_MSG_TYPE, TAG_SENDER_COMP_ID, TAG_SENDING_TIME, TAG_TARGET_COMP_ID,
};
use fixlog_core::{parse_one_with_format, resolve};

use crate::state::AppState;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no visible messages to export")]
    Empty,
}

/// Output format for `:export`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
    Fix,
    Pretty,
}

impl ExportFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "csv" => Some(Self::Csv),
            "json" => Some(Self::Json),
            "fix" => Some(Self::Fix),
            "pretty" => Some(Self::Pretty),
            _ => None,
        }
    }
}

/// Export `state.visible` to `path`. Returns the number of messages written.
pub fn export(state: &AppState, fmt: ExportFormat, path: &Path) -> Result<usize, ExportError> {
    if state.visible.is_empty() {
        return Err(ExportError::Empty);
    }
    let file = File::create(path)?;
    let mut out = BufWriter::new(file);

    let mut written = 0usize;
    if matches!(fmt, ExportFormat::Csv) {
        writeln!(
            out,
            "ordinal,offset,msg_type,sender,target,seq_num,sending_time"
        )?;
    }
    for &ord in &state.visible {
        let Some(bytes) = state.index.message_bytes(&state.mmap, ord as usize) else {
            continue;
        };
        let Ok((msg, _)) = parse_one_with_format(bytes, &state.format) else {
            continue;
        };
        let off = state.index.messages[ord as usize].start;
        match fmt {
            ExportFormat::Csv => {
                let tag = |t| {
                    msg.tags
                        .iter()
                        .find(|(tt, _)| *tt == t)
                        .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
                        .unwrap_or_default()
                };
                writeln!(
                    out,
                    "{ord},{off},{},{},{},{},{}",
                    csv_escape(&tag(TAG_MSG_TYPE)),
                    csv_escape(&tag(TAG_SENDER_COMP_ID)),
                    csv_escape(&tag(TAG_TARGET_COMP_ID)),
                    csv_escape(&tag(TAG_MSG_SEQ_NUM)),
                    csv_escape(&tag(TAG_SENDING_TIME)),
                )?;
            }
            ExportFormat::Fix => {
                out.write_all(bytes)?;
                out.write_all(b"\n")?;
            }
            ExportFormat::Json => {
                let r = resolve(&msg);
                write_json_line(&mut out, &r, bytes.len())?;
            }
            ExportFormat::Pretty => {
                let r = resolve(&msg);
                write_pretty_block(&mut out, &r, bytes.len())?;
            }
        }
        written += 1;
    }
    out.flush()?;
    Ok(written)
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

fn write_json_line<W: Write>(
    out: &mut W,
    msg: &fixlog_core::dict::ResolvedMessage<'_>,
    raw_len: usize,
) -> std::io::Result<()> {
    write!(out, r#"{{"offset":{},"raw_len":{}"#, msg.offset, raw_len)?;
    if let Some(name) = msg.msg_type_name {
        out.write_all(br#","msg_type_name":""#)?;
        write_json_bytes(out, name.as_bytes())?;
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
                write_json_bytes(out, n.as_bytes())?;
                out.write_all(br#"","#)?;
            }
            None => out.write_all(br#""name":null,"#)?,
        }
        out.write_all(br#""value":""#)?;
        write_json_bytes(out, f.value)?;
        out.write_all(b"\"")?;
        if let Some(label) = f.value_label {
            out.write_all(br#","value_label":""#)?;
            write_json_bytes(out, label.as_bytes())?;
            out.write_all(b"\"")?;
        }
        out.write_all(b"}")?;
    }
    out.write_all(b"]}\n")?;
    Ok(())
}

fn write_pretty_block<W: Write>(
    out: &mut W,
    msg: &fixlog_core::dict::ResolvedMessage<'_>,
    raw_len: usize,
) -> std::io::Result<()> {
    let header = match msg.msg_type_name {
        Some(name) => format!(" {name}"),
        None => String::new(),
    };
    writeln!(
        out,
        "Message @ offset {} ({} bytes){}",
        msg.offset, raw_len, header
    )?;
    let name_width = msg
        .fields
        .iter()
        .map(|f| f.name.map(str::len).unwrap_or(0))
        .max()
        .unwrap_or(0);
    for f in &msg.fields {
        let name = f.name.unwrap_or("?");
        match f.value_label {
            Some(label) => writeln!(
                out,
                "  {:>5}  {:<width$} = {} ({})",
                f.tag,
                name,
                String::from_utf8_lossy(f.value),
                label,
                width = name_width,
            )?,
            None => writeln!(
                out,
                "  {:>5}  {:<width$} = {}",
                f.tag,
                name,
                String::from_utf8_lossy(f.value),
                width = name_width,
            )?,
        }
    }
    writeln!(out)?;
    Ok(())
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
