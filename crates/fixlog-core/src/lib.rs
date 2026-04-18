#![forbid(unsafe_code)]

//! `fixlog-core` — facade over the core building blocks of fixlog.
//!
//! Pulls in [`fixlog_format`] (log-format sniffer), [`fixlog_parser`]
//! (zero-copy tokenizer), [`fixlog_dict`] (multi-version dictionary),
//! [`fixlog_index`] (offset-based index), and [`fixlog_query`] (filter DSL).
//! Downstream binaries and tools should depend on this crate and pick what
//! they need from the re-exported modules instead of wiring up the lower-level
//! crates individually.

pub use fixlog_dict as dict;
pub use fixlog_format as format;
pub use fixlog_index as index;
pub use fixlog_parser as parser;
pub use fixlog_query as query;

/// The most commonly-used items, re-exported at the crate root so callers can
/// write `fixlog_core::parse_all_with_format(...)` and `fixlog_core::resolve(...)`
/// without pulling the sub-modules.
pub use fixlog_dict::{
    CHAIN_FIX44, CHAIN_FIXT11_FIX44, CHAIN_FIXT11_FIX50SP2, DictChain, FieldDef, FixVersion,
    ResolvedField, ResolvedMessage, chain_for, resolve, resolve_with_chain,
};
pub use fixlog_format::{LogFormat, sniff};
pub use fixlog_index::{
    HotTags, IndexError, LogIndex, MessageOffset, SecondaryIndex, build_from_bytes,
    build_from_bytes_parallel, build_from_bytes_parallel_with_hot_tags,
    build_from_bytes_with_hot_tags,
};
pub use fixlog_parser::{
    ParseError, RawMessage, parse_all, parse_all_with_format, parse_one, parse_one_with_format,
};
pub use fixlog_query::{Expr as QueryExpr, QueryError, parse as parse_query};
