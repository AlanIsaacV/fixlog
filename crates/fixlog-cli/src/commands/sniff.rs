//! `fixlog sniff <file>` — print the detected log format.

use std::path::Path;

use anyhow::{Context, Result};
use fixlog_core::format::{
    Encoding, LineEnding, LinePrefix, LogFormat, MessageBoundary, Separator,
};

use crate::io::{head, mmap_file};

/// How many bytes to feed to the sniffer. 64KB is enough to see hundreds of messages even with
/// long lines, and stays well below the default mmap working set.
const SNIFF_WINDOW: usize = 64 * 1024;

pub fn run(path: &Path) -> Result<()> {
    let mmap = mmap_file(path)?;
    let sample = head(&mmap, SNIFF_WINDOW);
    let format =
        fixlog_core::sniff(sample).with_context(|| format!("sniffing {}", path.display()))?;
    print_format(&format, path);
    Ok(())
}

fn print_format(fmt: &LogFormat, path: &Path) {
    println!("File:           {}", path.display());
    println!("Separator:      {}", describe_separator(fmt.separator));
    println!("Line prefix:    {}", describe_prefix(&fmt.line_prefix));
    println!("Line ending:    {}", describe_line_ending(fmt.line_ending));
    println!("Encoding:       {}", describe_encoding(fmt.encoding));
    println!(
        "Msg boundary:   {}",
        describe_boundary(fmt.message_boundary)
    );
}

fn describe_separator(s: Separator) -> &'static str {
    match s {
        Separator::Soh => "SOH (0x01)",
        Separator::Pipe => "| (0x7C)",
        Separator::Caret => "^ (0x5E)",
        Separator::Semicolon => "; (0x3B)",
        Separator::Custom(_) => "custom",
    }
}

fn describe_prefix(p: &LinePrefix) -> String {
    match p {
        LinePrefix::None => "none".to_owned(),
        LinePrefix::Fixed(n) => format!("fixed {n} bytes"),
    }
}

fn describe_line_ending(le: LineEnding) -> &'static str {
    match le {
        LineEnding::Lf => "LF (\\n)",
        LineEnding::CrLf => "CRLF (\\r\\n)",
    }
}

fn describe_encoding(enc: Encoding) -> &'static str {
    match enc {
        Encoding::Utf8 => "UTF-8 / ASCII",
    }
}

fn describe_boundary(b: MessageBoundary) -> &'static str {
    match b {
        MessageBoundary::Line => "line",
        MessageBoundary::Checksum => "checksum",
    }
}
