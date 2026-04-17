# `fixlog-dict`

Multi-version FIX dictionary + resolver. Converts tag numbers to field names and enum values to labels, borrowing from static tables generated at build time.

## Files

- `crates/fixlog-dict/Cargo.toml` — runtime dep `fixlog-parser`; build-dep `quick-xml = "0.39"`.
- `crates/fixlog-dict/build.rs` — XML → Rust codegen. See `Codegen pipeline` below.
- `crates/fixlog-dict/src/lib.rs` — public API (`FixVersion`, chains, lookups).
- `crates/fixlog-dict/src/generated.rs` — `include!()` bridge to `OUT_DIR/dict_generated.rs`.
- `crates/fixlog-dict/src/resolver.rs` — `resolve` / `resolve_with_chain`.
- `crates/fixlog-dict/tests/dict.rs` — integration tests per version.

## Dictionary XML sources (workspace-root `dictionaries/`)

| File | Fields | Messages | Notes |
|------|--------|----------|-------|
| `FIX44.xml` | 916 | 92 | Classic pre-transport-split. |
| `FIXT11.xml` | 74 | 8 | Session layer only; defines `ApplVerID` (1137), etc. |
| `FIX50SP2.xml` | 1432 | 120 | Application layer for FIXT sessions. |

Sources: QuickFIX/J at `quickfixj-messages-<v>/src/main/resources/<V>.xml`. `FIX44.xml` is at `quickfixj-messages-fix44` (not `quickfixj-core`).

## Public API

```rust
pub enum FixVersion { Fix44, Fixt11, Fix50Sp2 }

pub struct FieldDef {
    pub tag: u32,
    pub name: &'static str,
    pub field_type: &'static str,   // e.g. "STRING", "PRICE", "SEQNUM" — FIX data-type names
}

pub fn field_by_tag(v: FixVersion, tag: u32) -> Option<FieldDef>;
pub fn enum_value_label(v: FixVersion, tag: u32, value: &[u8]) -> Option<&'static str>;
pub fn msg_type_label(v: FixVersion, value: &[u8]) -> Option<&'static str>;

// Compile-time constants:
pub const fn field_count(v: FixVersion) -> usize;
pub const fn message_count(v: FixVersion) -> usize;

// Chain API:
pub type DictChain = &'static [FixVersion];
pub const CHAIN_FIX44: DictChain;
pub const CHAIN_FIXT11_FIX50SP2: DictChain;
pub const CHAIN_FIXT11_FIX44: DictChain;

pub fn chain_field_by_tag(chain: DictChain, tag: u32) -> Option<FieldDef>;
pub fn chain_enum_value_label(chain: DictChain, tag: u32, value: &[u8]) -> Option<&'static str>;
pub fn chain_msg_type_label(chain: DictChain, value: &[u8]) -> Option<&'static str>;

pub fn chain_for(begin_string: &[u8], appl_ver_id: Option<&[u8]>) -> DictChain;
```

Chain semantics: try each `FixVersion` in order, first hit wins.

## Chain selection (`chain_for`)

| BeginString | ApplVerID | Chain |
|-------------|-----------|-------|
| `FIX.4.4` | — | `CHAIN_FIX44` |
| `FIXT.1.1` | `6` | `CHAIN_FIXT11_FIX44` |
| `FIXT.1.1` | other / absent | `CHAIN_FIXT11_FIX50SP2` |
| anything else | — | `CHAIN_FIX44` (best-effort) |

See `reference/fix-protocol.md` for the full `ApplVerID` value table.

## Resolver

```rust
pub struct ResolvedField<'a> {
    pub tag: u32,
    pub name: Option<&'static str>,        // None for unknown/custom tags
    pub value: &'a [u8],                   // borrowed from RawMessage
    pub value_label: Option<&'static str>, // None for non-enum values or unknown enums
}

pub struct ResolvedMessage<'a> {
    pub offset: u64,
    pub msg_type_name: Option<&'static str>,
    pub chain: &'static [FixVersion],      // the chain actually used
    pub fields: Vec<ResolvedField<'a>>,    // same order as RawMessage.tags
}

pub fn resolve<'a>(msg: &RawMessage<'a>) -> ResolvedMessage<'a>;
pub fn resolve_with_chain<'a>(msg: &RawMessage<'a>, chain: DictChain) -> ResolvedMessage<'a>;
```

`resolve` inspects the message itself to pick the chain (reads tags 8 and 1128). For bulk work where the session is known, prefer `resolve_with_chain` to avoid re-inference per message.

Tag 35 (MsgType) uses `chain_msg_type_label`; other tags use `chain_enum_value_label`.

`TAG_APPL_VER_ID` constant (`1128`) is exported from `resolver.rs`.

## Codegen pipeline (`build.rs`)

Dictionaries to generate are declared in a const:

```rust
const DICTIONARIES: &[(&str, &str)] = &[
    ("fix44",    "FIX44.xml"),
    ("fixt11",   "FIXT11.xml"),
    ("fix50sp2", "FIX50SP2.xml"),
];
```

For each entry:

1. Read `dictionaries/<xml>` (path resolved via `CARGO_MANIFEST_DIR/../..`).
2. Stream-parse with `quick_xml::Reader`, extracting:
   - `<field number=N name=X type=T>` under `<fields>` → `FieldInfo`.
   - Nested `<value enum=E description=D/>` → enum table for that field.
   - `<message name=X msgtype=Y>` under `<messages>` → `MessageInfo`.
3. Emit a Rust module `pub(crate) mod <module> { … }` into `$OUT_DIR/dict_generated.rs` with:
   - `field_by_tag(u32) -> Option<&'static GenFieldDef>` (giant `match` on tag).
   - `enum_value_label(u32, &[u8]) -> Option<&'static str>` (nested `match tag → match value`).
   - `msg_type_label(&[u8]) -> Option<&'static str>` (`match value`).
   - `const FIELD_COUNT: usize` and `const MESSAGE_COUNT: usize`.

`GenFieldDef` is defined in `src/generated.rs` and imported via `use super::GenFieldDef;` at the top of each generated module.

### `cargo:rerun-if-changed` emitted by build.rs

- `build.rs` itself.
- Every dictionary file listed in `DICTIONARIES`.

### Generated output size

`$OUT_DIR/dict_generated.rs` is roughly 10 K lines. The match arms compile down to jump tables so runtime lookups are O(1) amortized.

## Adding a new FIX version

1. Download the XML into `dictionaries/` (QuickFIX/J layout).
2. Add a `(module_name, "FILE.xml")` entry to `DICTIONARIES` in `build.rs`.
3. Add a `FixVersion::<Name>` variant to `lib.rs`.
4. Extend the three top-level match expressions (`field_by_tag`, `enum_value_label`, `msg_type_label`) + `field_count` / `message_count`.
5. Optionally declare a new `CHAIN_*` constant and extend `chain_for` routing.
6. Add tests for version-specific fields under `tests/dict.rs`.

## INVARIANT: zero-alloc lookups

`FieldDef::name`, `FieldDef::field_type`, and all enum/msg-type labels are `&'static str`. They're immutable literals in the generated module. Do not change the API to return `String` — callers rely on zero-alloc resolution.

## Test expectations

- `tests/dict.rs` asserts compile-time lower bounds: `FIELD_COUNT(Fix44) >= 900`, `FIELD_COUNT(Fix50Sp2) >= 1400`, `FIELD_COUNT(Fixt11) >= 50`.
- `tests/dict.rs::chain_falls_through_fixt11_to_fix50sp2` guards the chain fallback logic.
- `src/resolver.rs::tests` cover the per-message chain inference and enum resolution (FIX 4.4 + FIXT.1.1 Logon).
