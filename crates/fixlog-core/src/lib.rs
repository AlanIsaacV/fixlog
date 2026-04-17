#![forbid(unsafe_code)]

//! `fixlog-core` — facade over the three core building blocks of fixlog.
//!
//! Pulls in [`fixlog_format`] (log-format sniffer), [`fixlog_parser`]
//! (zero-copy tokenizer), and [`fixlog_dict`] (multi-version dictionary).
//! Downstream binaries and tools should depend on this crate and pick what
//! they need from the re-exported modules instead of wiring up the three
//! lower-level crates individually.

pub use fixlog_dict as dict;
pub use fixlog_format as format;
pub use fixlog_parser as parser;

/// The most commonly-used items, re-exported at the crate root so callers can
/// write `fixlog_core::parse_all_with_format(...)` and `fixlog_core::resolve(...)`
/// without pulling the sub-modules.
pub use fixlog_dict::{
    CHAIN_FIX44, CHAIN_FIXT11_FIX44, CHAIN_FIXT11_FIX50SP2, DictChain, FieldDef, FixVersion,
    ResolvedField, ResolvedMessage, chain_for, resolve, resolve_with_chain,
};
pub use fixlog_format::{LogFormat, sniff};
pub use fixlog_parser::{ParseError, RawMessage, parse_all, parse_all_with_format, parse_one};
