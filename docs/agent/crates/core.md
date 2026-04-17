# `fixlog-core`

Facade crate. Re-exports the three building blocks and hoists the most-used items to the crate root.

## Files

- `crates/fixlog-core/Cargo.toml` — deps: `fixlog-dict`, `fixlog-format`, `fixlog-parser` (all path deps).
- `crates/fixlog-core/src/lib.rs` — only re-exports, no logic.

## Re-export shape

```rust
// Sub-modules:
pub use fixlog_dict   as dict;
pub use fixlog_format as format;
pub use fixlog_parser as parser;

// Hoisted to crate root (the common 90%):
pub use fixlog_dict::{
    CHAIN_FIX44, CHAIN_FIXT11_FIX44, CHAIN_FIXT11_FIX50SP2,
    DictChain, FieldDef, FixVersion,
    ResolvedField, ResolvedMessage,
    chain_for, resolve, resolve_with_chain,
};
pub use fixlog_format::{LogFormat, sniff};
pub use fixlog_parser::{
    ParseError, RawMessage,
    parse_all, parse_all_with_format, parse_one,
};
```

## Rule

Downstream crates (`fixlog-cli`, and future `fixlog-tui`) depend on `fixlog-core` — **not** on the underlying three crates. This gives one place to:

- Version the surface area consistently.
- Hide or upgrade internal deps without touching every consumer.
- Document the "official" public API.

## When to add to the re-export list

Hoist to the crate root when it's used in >1 command / >1 consumer. Otherwise leave it accessible via `fixlog_core::dict::X`, `fixlog_core::parser::Y`. Keep the root surface small.

## Do not

- Do not put logic here. This is a re-export-only crate.
- Do not re-export with renaming; keep names identical to the source crate.
- Do not add new dependencies here without also adding them to the `use` block in `lib.rs`.
