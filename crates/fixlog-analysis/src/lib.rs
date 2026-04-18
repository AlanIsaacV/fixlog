#![forbid(unsafe_code)]

//! Session tracking, order lifecycle reconstruction, and temporal
//! histogram analysis over an indexed FIX log.
//!
//! This crate is a pure library: given a [`LogIndex`], a buffer `&[u8]`,
//! and a [`LogFormat`], builders produce derived structures without
//! mutating the source. Analysis outputs are materialized (owned) so
//! they can survive an `Arc<Mmap>` swap during `--follow`.
//!
//! [`LogIndex`]: fixlog_core::LogIndex
//! [`LogFormat`]: fixlog_core::LogFormat

pub mod histogram;
pub mod orders;
pub mod sessions;
pub mod util;

use fixlog_core::ParseError;

/// Errors produced by analysis builders.
#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    /// Parse error delegated from `fixlog-parser`.
    #[error(transparent)]
    Parse(#[from] ParseError),

    /// A required tag was missing from the message at `ordinal`.
    #[error("missing tag {tag} at ordinal {ordinal}")]
    MissingTag { tag: u32, ordinal: u32 },
}
