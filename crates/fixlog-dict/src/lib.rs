#![forbid(unsafe_code)]

//! FIX dictionary: resolve tag numbers into field names and wire values into
//! human-readable labels across multiple FIX versions.
//!
//! Dictionaries are generated at build time from `dictionaries/*.xml` into
//! `$OUT_DIR/dict_generated.rs`. Each dictionary ([`FixVersion`]) exposes the
//! same trio of compile-time `match`-based lookups: `field_by_tag`,
//! `enum_value_label`, and `msg_type_label`.
//!
//! Real-world logs often require *chained* resolution: a FIXT.1.1 session
//! carries session admin messages defined in the FIXT dictionary, while the
//! application-layer messages follow FIX 5.0 / 5.0SP1 / 5.0SP2. The
//! [`DictChain`] type expresses that ordering: lookups try each version in
//! turn until one matches.
//!
//! The [`resolve`] function chooses a chain automatically from the message's
//! `BeginString` and, when present, `ApplVerID` (tag 1128).

mod generated;
pub mod groups;
pub mod resolver;

pub use groups::group_members;
pub use resolver::{ResolvedField, ResolvedMessage, resolve, resolve_with_chain};

/// Description of a single FIX field, as declared in the dictionary.
///
/// `name` and `field_type` are static strings from the generated table, so
/// holding a `FieldDef` never ties you to a specific message buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDef {
    /// Tag number (e.g. `35` for `MsgType`).
    pub tag: u32,
    /// Field name (e.g. `"MsgType"`).
    pub name: &'static str,
    /// FIX data type declared in the dictionary (e.g. `"STRING"`, `"PRICE"`).
    pub field_type: &'static str,
}

/// FIX dictionary versions bundled with this crate.
///
/// More versions can be added by extending `DICTIONARIES` in `build.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixVersion {
    /// FIX 4.4 — the most widely deployed pre-transport-split version.
    Fix44,
    /// FIXT.1.1 — session-layer messages that sit below FIX 5.x application
    /// dictionaries.
    Fixt11,
    /// FIX 5.0 — original 5.0 application-layer dictionary.
    Fix50,
    /// FIX 5.0 Service Pack 1 — 5.0SP1 application-layer dictionary.
    Fix50Sp1,
    /// FIX 5.0 Service Pack 2 — current application-layer dictionary.
    Fix50Sp2,
}

/// An ordered sequence of dictionaries to consult when resolving a tag.
///
/// Chains are looked up in order and the first match wins. Chains are
/// `&'static`, so they can be embedded in consts without allocation.
pub type DictChain = &'static [FixVersion];

/// Chain for messages with `BeginString = FIX.4.4`.
pub const CHAIN_FIX44: DictChain = &[FixVersion::Fix44];

/// Chain for FIXT.1.1 sessions carrying FIX 5.0SP2 application messages.
/// Session-layer tags resolve via FIXT11 first; application tags fall back to
/// FIX 5.0SP2. Also used as a default when `ApplVerID` is absent.
pub const CHAIN_FIXT11_FIX50SP2: DictChain = &[FixVersion::Fixt11, FixVersion::Fix50Sp2];

/// Chain for FIXT.1.1 sessions where `ApplVerID = 6` (FIX 4.4 application).
pub const CHAIN_FIXT11_FIX44: DictChain = &[FixVersion::Fixt11, FixVersion::Fix44];

/// Chain for FIXT.1.1 sessions where `ApplVerID = 7` (FIX 5.0 application).
pub const CHAIN_FIXT11_FIX50: DictChain = &[FixVersion::Fixt11, FixVersion::Fix50];

/// Chain for FIXT.1.1 sessions where `ApplVerID = 8` (FIX 5.0SP1 application).
pub const CHAIN_FIXT11_FIX50SP1: DictChain = &[FixVersion::Fixt11, FixVersion::Fix50Sp1];

/// Look up a field by its tag number in a specific dictionary.
pub fn field_by_tag(version: FixVersion, tag: u32) -> Option<FieldDef> {
    let def = match version {
        FixVersion::Fix44 => generated::fix44::field_by_tag(tag),
        FixVersion::Fixt11 => generated::fixt11::field_by_tag(tag),
        FixVersion::Fix50 => generated::fix50::field_by_tag(tag),
        FixVersion::Fix50Sp1 => generated::fix50sp1::field_by_tag(tag),
        FixVersion::Fix50Sp2 => generated::fix50sp2::field_by_tag(tag),
    }?;
    Some(FieldDef {
        tag: def.tag,
        name: def.name,
        field_type: def.ty,
    })
}

/// Look up the human-readable label for an enum value in a specific
/// dictionary. Returns `None` for non-enum fields or unknown values.
pub fn enum_value_label(version: FixVersion, tag: u32, value: &[u8]) -> Option<&'static str> {
    match version {
        FixVersion::Fix44 => generated::fix44::enum_value_label(tag, value),
        FixVersion::Fixt11 => generated::fixt11::enum_value_label(tag, value),
        FixVersion::Fix50 => generated::fix50::enum_value_label(tag, value),
        FixVersion::Fix50Sp1 => generated::fix50sp1::enum_value_label(tag, value),
        FixVersion::Fix50Sp2 => generated::fix50sp2::enum_value_label(tag, value),
    }
}

/// Look up the message-type label for a `35=` value in a specific dictionary.
pub fn msg_type_label(version: FixVersion, value: &[u8]) -> Option<&'static str> {
    match version {
        FixVersion::Fix44 => generated::fix44::msg_type_label(value),
        FixVersion::Fixt11 => generated::fixt11::msg_type_label(value),
        FixVersion::Fix50 => generated::fix50::msg_type_label(value),
        FixVersion::Fix50Sp1 => generated::fix50sp1::msg_type_label(value),
        FixVersion::Fix50Sp2 => generated::fix50sp2::msg_type_label(value),
    }
}

/// Try each dictionary in `chain` and return the first field definition found.
pub fn chain_field_by_tag(chain: DictChain, tag: u32) -> Option<FieldDef> {
    chain.iter().find_map(|&v| field_by_tag(v, tag))
}

/// Try each dictionary in `chain` and return the first enum label found.
pub fn chain_enum_value_label(chain: DictChain, tag: u32, value: &[u8]) -> Option<&'static str> {
    chain.iter().find_map(|&v| enum_value_label(v, tag, value))
}

/// Try each dictionary in `chain` and return the first MsgType label found.
pub fn chain_msg_type_label(chain: DictChain, value: &[u8]) -> Option<&'static str> {
    chain.iter().find_map(|&v| msg_type_label(v, value))
}

/// Pick a dictionary chain from the session's `BeginString` and `ApplVerID`.
///
/// | BeginString | ApplVerID | Chain                         |
/// |-------------|-----------|-------------------------------|
/// | `FIX.4.4`   | —         | [`CHAIN_FIX44`]               |
/// | `FIXT.1.1`  | `6`       | [`CHAIN_FIXT11_FIX44`]        |
/// | `FIXT.1.1`  | `7`       | [`CHAIN_FIXT11_FIX50`]        |
/// | `FIXT.1.1`  | `8`       | [`CHAIN_FIXT11_FIX50SP1`]     |
/// | `FIXT.1.1`  | `9` / other / — | [`CHAIN_FIXT11_FIX50SP2`] |
/// | other       | —         | [`CHAIN_FIX44`] (best-effort) |
///
/// ApplVerID numeric values follow QuickFIX conventions: `6`=FIX44,
/// `7`=FIX50, `8`=FIX50SP1, `9`=FIX50SP2. Unknown or missing ApplVerID on
/// a FIXT.1.1 session falls back to SP2 — the most common wire version
/// today and a strict superset of older application dictionaries for the
/// session-layer tags we care about.
pub fn chain_for(begin_string: &[u8], appl_ver_id: Option<&[u8]>) -> DictChain {
    if begin_string == b"FIXT.1.1" {
        match appl_ver_id {
            Some(b"6") => CHAIN_FIXT11_FIX44,
            Some(b"7") => CHAIN_FIXT11_FIX50,
            Some(b"8") => CHAIN_FIXT11_FIX50SP1,
            _ => CHAIN_FIXT11_FIX50SP2,
        }
    } else {
        // Default: FIX 4.4 dictionary for `FIX.4.4` and any other classic
        // BeginString. Unknown BeginStrings still get a best-effort
        // resolution — tags that overlap with FIX 4.4 will resolve.
        CHAIN_FIX44
    }
}

/// Total number of fields in the specified dictionary.
pub const fn field_count(version: FixVersion) -> usize {
    match version {
        FixVersion::Fix44 => generated::fix44::FIELD_COUNT,
        FixVersion::Fixt11 => generated::fixt11::FIELD_COUNT,
        FixVersion::Fix50 => generated::fix50::FIELD_COUNT,
        FixVersion::Fix50Sp1 => generated::fix50sp1::FIELD_COUNT,
        FixVersion::Fix50Sp2 => generated::fix50sp2::FIELD_COUNT,
    }
}

/// Total number of message types in the specified dictionary.
pub const fn message_count(version: FixVersion) -> usize {
    match version {
        FixVersion::Fix44 => generated::fix44::MESSAGE_COUNT,
        FixVersion::Fixt11 => generated::fixt11::MESSAGE_COUNT,
        FixVersion::Fix50 => generated::fix50::MESSAGE_COUNT,
        FixVersion::Fix50Sp1 => generated::fix50sp1::MESSAGE_COUNT,
        FixVersion::Fix50Sp2 => generated::fix50sp2::MESSAGE_COUNT,
    }
}
