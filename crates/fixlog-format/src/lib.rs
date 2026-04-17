#![forbid(unsafe_code)]

//! Detect the layout of a FIX log file from a small byte sample.
//!
//! [`sniff`] inspects the first chunk of bytes and returns a [`LogFormat`] that downstream
//! consumers (parser, indexer) need in order to read the rest of the file correctly.

pub mod sniffer;

pub use sniffer::sniff;

/// Everything needed to read a FIX log: how fields are separated, how each line is prefixed,
/// what encoding is used, and how lines end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFormat {
    pub separator: Separator,
    pub line_prefix: LinePrefix,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
    pub message_boundary: MessageBoundary,
}

/// Byte that separates `tag=value` pairs inside a single FIX message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separator {
    /// Standard FIX separator, ASCII `0x01`.
    Soh,
    /// `|`, common in human-readable logs.
    Pipe,
    /// `^`, used by some legacy tools that escape SOH as `^A`.
    Caret,
    /// `;`, occasionally used by hand-rolled loggers.
    Semicolon,
    /// Anything else.
    Custom(u8),
}

impl Separator {
    /// Byte representation of this separator.
    pub fn as_byte(self) -> u8 {
        match self {
            Separator::Soh => 0x01,
            Separator::Pipe => b'|',
            Separator::Caret => b'^',
            Separator::Semicolon => b';',
            Separator::Custom(b) => b,
        }
    }
}

/// What appears at the start of each line, before the FIX `8=` BeginString tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinePrefix {
    /// No prefix; lines start directly at `8=`.
    None,
    /// A fixed number of bytes to strip from the beginning of each line.
    /// Detected when the offset of `8=FIX` is identical across the sampled lines.
    Fixed(usize),
}

/// Character encoding of the file. We only support text encodings; binary FIX is out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// ASCII / UTF-8 — the only thing real FIX logs use in practice.
    Utf8,
}

/// Line terminator used by the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// Unix-style `\n`.
    Lf,
    /// Windows-style `\r\n`.
    CrLf,
}

/// How to find the boundary between two FIX messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageBoundary {
    /// One message per line. The line ending terminates the message.
    Line,
    /// Messages can span lines. The CheckSum (tag 10) ends each message.
    Checksum,
}

/// Errors produced by [`sniff`] when it cannot determine the format with confidence.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SniffError {
    #[error("sample is empty")]
    EmptySample,
    #[error("could not find a FIX BeginString (`8=FIX`) in the sample")]
    NoBeginString,
    #[error("could not infer a consistent field separator from the sample")]
    NoSeparator,
}
