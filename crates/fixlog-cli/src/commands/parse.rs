//! `fixlog parse <file>` — parse messages and print them as a table or JSONL.

use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};
use fixlog_core::parse_all_with_format;
use fixlog_core::render::{write_jsonl, write_pretty};
use fixlog_core::resolve;

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
