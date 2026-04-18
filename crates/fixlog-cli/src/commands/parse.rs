//! `fixlog parse <file>` — parse messages and print them as a table or JSONL.

use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};
use fixlog_core::{ResolvedField, ResolvedMessage, parse_all_with_format, resolve};

use crate::ParseFormat;
use crate::io::{head, mmap_file};

const SNIFF_WINDOW: usize = 64 * 1024;

pub fn run(path: &Path, first: Option<usize>, format: ParseFormat) -> Result<()> {
    let mmap = mmap_file(path)?;
    let log_format = fixlog_core::sniff(head(&mmap, SNIFF_WINDOW))
        .with_context(|| format!("sniffing {}", path.display()))?;

    let limit = first.unwrap_or(usize::MAX);
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    for msg in parse_all_with_format(&mmap, &log_format)
        .filter_map(Result::ok)
        .take(limit)
    {
        let resolved = resolve(&msg);
        match format {
            ParseFormat::Pretty => write_pretty(&mut out, &resolved, msg.raw.len())?,
            ParseFormat::Json => write_jsonl(&mut out, &resolved, msg.raw.len())?,
        }
    }
    out.flush()?;
    Ok(())
}

pub(crate) fn write_pretty<W: Write>(
    out: &mut W,
    msg: &ResolvedMessage<'_>,
    raw_len: usize,
) -> Result<()> {
    let header = match msg.msg_type_name {
        Some(name) => format!(" {name}"),
        None => String::new(),
    };
    writeln!(
        out,
        "Message @ offset {} ({} bytes){}",
        msg.offset, raw_len, header,
    )?;
    // Compute a width that accommodates the longest field name so columns align even
    // when a message mixes known and unknown tags.
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
) -> Result<()> {
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
        )?,
        None => writeln!(
            out,
            "  {:>5}  {:<width$} = {}",
            f.tag,
            name,
            lossy(f.value),
            width = name_width,
        )?,
    }
    Ok(())
}

pub(crate) fn write_jsonl<W: Write>(
    out: &mut W,
    msg: &ResolvedMessage<'_>,
    raw_len: usize,
) -> Result<()> {
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

fn lossy(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

/// Write `bytes` into `out` escaped as a JSON string fragment (without the surrounding quotes).
/// Replaces non-UTF-8 bytes with the Unicode replacement character via [`String::from_utf8_lossy`].
fn write_json_string<W: Write>(out: &mut W, bytes: &[u8]) -> std::io::Result<()> {
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
