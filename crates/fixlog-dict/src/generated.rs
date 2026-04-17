//! Bridge to the build-script-generated dictionary tables.
//!
//! The generated file lives in `$OUT_DIR/dict_generated.rs` and contains one
//! `mod <version> { ... }` per dictionary, each with `field_by_tag`,
//! `enum_value_label`, `msg_type_label`, and `FIELD_COUNT`/`MESSAGE_COUNT`.

/// Internal representation of a generated field definition. Matches the layout
/// emitted by `build.rs`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GenFieldDef {
    pub tag: u32,
    pub name: &'static str,
    pub ty: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/dict_generated.rs"));
