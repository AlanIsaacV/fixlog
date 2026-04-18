//! Export the currently-visible message set to disk.
//!
//! Called from `:export <fmt> <path>`. Writes are **synchronous** — the
//! event loop blocks during large exports. For >100K messages we preview a
//! warning in `StatusMessage` before committing; see P4-T12 for the UX
//! decision.
//!
//! Formats (all delegated to `fixlog_core::render`):
//!
//! - `csv`    — `ordinal,offset,msg_type,sender,target,seq_num,sending_time`
//! - `json`   — one JSON object per line, matching `fixlog parse --format json`
//! - `fix`    — raw bytes per message (zero-copy from mmap), `\n` separator
//! - `pretty` — table layout, matching `fixlog parse --format pretty`

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use fixlog_core::render::{write_csv_header, write_csv_row, write_fix, write_jsonl, write_pretty};
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
        write_csv_header(&mut out)?;
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
            ExportFormat::Csv => write_csv_row(&mut out, ord, off, &msg)?,
            ExportFormat::Fix => write_fix(&mut out, bytes)?,
            ExportFormat::Json => {
                let r = resolve(&msg);
                write_jsonl(&mut out, &r, bytes.len())?;
            }
            ExportFormat::Pretty => {
                let r = resolve(&msg);
                write_pretty(&mut out, &r, bytes.len())?;
            }
        }
        written += 1;
    }
    out.flush()?;
    Ok(written)
}
