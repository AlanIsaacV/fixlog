# Cross-cutting patterns

Recurring idioms used across crates. Reference when adding new code so it fits the existing style.

## Zero-copy lifetime pattern

Core types borrow from the source buffer:

```rust
pub struct RawMessage<'a> {
    pub offset: u64,
    pub raw: &'a [u8],
    pub tags: SmallVec<[(u32, &'a [u8]); 32]>,
}
```

Lifetime rule: anything derived from a `RawMessage<'a>` that still references wire bytes must carry the same `'a`. `ResolvedMessage<'a>` follows this:

```rust
pub struct ResolvedField<'a> { pub value: &'a [u8], /* static strs for name/label */ }
pub struct ResolvedMessage<'a> { pub fields: Vec<ResolvedField<'a>>, /* … */ }
```

Static strings (`name: &'static str`, `value_label: Option<&'static str>`) come from the generated dictionary tables, so they don't extend the lifetime.

**Do not** collect into owned `Vec<u8>` / `String` inside hot paths. Materialize only at presentation boundaries (CLI output).

## Error handling split

- **Library crates** (`fixlog-parser`, `fixlog-format`, `fixlog-dict`, `fixlog-core`): public errors are `thiserror`-derived enums (`ParseError`, `SniffError`). `#![forbid(unsafe_code)]` at the top.
- **Binary crate** (`fixlog-cli`): uses `anyhow::Result` end-to-end. `.with_context(|| ...)` at I/O boundaries. `main` returns `anyhow::Result<()>`.
- **Tests**: `unwrap()` / `expect()` freely. Production code must not.

When library code wants to distinguish fatal vs non-fatal conditions, emit the non-fatal ones through `tracing::debug!` / `warn!` and return `Ok(...)` anyway. See the checksum-mismatch pattern in `crates/parser.md`.

## Tracing conventions

- `error!` — condition that prevents producing a result the caller asked for.
- `warn!` — recoverable anomaly the user probably wants to know about.
- `info!` — high-level milestones (e.g. "parsed N messages from FILE").
- `debug!` — fine-grained diagnostic; per-message events go here. Checksum mismatches use this level.
- `trace!` — not currently used; reserve for byte-level tracing.

CLI exposes `-v` (info), `-vv` (debug). Respects `RUST_LOG` env var via `EnvFilter::try_from_default_env()`.

## `memmap2` pattern

The CLI is the only place that performs the `unsafe` `Mmap::map`. Encapsulated in `crates/fixlog-cli/src/io.rs::mmap_file`:

```rust
let file = File::open(path)?;
// SAFETY: we only ever hand out shared references to the mapping.
let mmap = unsafe { Mmap::map(&file) }?;
```

Library crates take `&[u8]`. Never pass an `Mmap` around — deref it at the boundary.

## Building a synthetic FIX message for tests

Use this helper (see `crates/fixlog-dict/src/resolver.rs::tests::build_with_trailer`):

```rust
fn build_with_trailer(body: &[u8], begin_string: &[u8]) -> Vec<u8> {
    let mut head = Vec::new();
    head.extend_from_slice(b"8=");
    head.extend_from_slice(begin_string);
    head.push(0x01);
    head.extend_from_slice(format!("9={}\x01", body.len()).as_bytes());
    head.extend_from_slice(body);
    let cs = (head.iter().map(|&b| b as u32).sum::<u32>() % 256) as u8;
    head.extend_from_slice(format!("10={cs:03}\x01").as_bytes());
    head
}
```

`body` must be `tag=value<SOH>…` starting at `35=…` and ending with `<SOH>`. `BodyLength` = `body.len()`. Checksum is computed over `head` at the point just before the trailer is appended (i.e. includes everything from `8=` through the SOH that closes the last body tag).

## Finding a tag in a `RawMessage`

Linear scan over `msg.tags`. For most messages (<32 tags) this is a handful of comparisons, faster than building any map:

```rust
fn find_tag<'a>(msg: &RawMessage<'a>, tag: u32) -> Option<&'a [u8]> {
    msg.tags.iter().find(|(t, _)| *t == tag).map(|(_, v)| *v)
}
```

See `crates/fixlog-dict/src/resolver.rs` for the reference implementation.

## Writing to stdout from a command

`BufWriter` around `std::io::stdout().lock()` is the pattern. Flush at the end:

```rust
let stdout = std::io::stdout();
let mut out = BufWriter::new(stdout.lock());
/* write! writeln! write_all! */
out.flush()?;
```

This dominates over raw `println!` for any loop that emits many messages.

## JSON escaping

Manual escape function in `crates/fixlog-cli/src/commands/parse.rs::write_json_string`. Handles: `"`, `\\`, `\n`, `\r`, `\t`, control chars (`\u00XX`), lossy UTF-8 via `String::from_utf8_lossy`. Do not pull in `serde_json` just for this — the output format is fixed and manual is ~25 LoC.

## Clippy gotchas encountered

- `collapsible_if` — nested `if let`s must use `&& let`:
  ```rust
  if let Some(x) = a && let Some(y) = b(x) { /* use y */ }
  ```
- `useless_asref` — comparing `&[u8]` to `Vec<u8>::as_slice()`, don't `value.as_ref()`. Just `value`.
- `assertions_on_constants` — in test files, replace `#[test] fn f() { assert!(CONST >= N); }` with `const _: () = { assert!(CONST >= N); };` for compile-time checks (requires `const fn` accessors).

## `const fn` for compile-time assertions

Accessor functions that need to feed `const { assert!(...) }` must themselves be `const fn`. See `field_count` / `message_count` in `fixlog-dict`.

## Do not

- Do not add `serde` / `serde_json` unless a consumer genuinely needs structured data beyond JSONL output. The manual writers are fine.
- Do not add `tokio` anywhere. fixlog is synchronous end-to-end; async comes only if/when Phase 3 TUI needs it.
- Do not add `log` + `env_logger`; the project uses `tracing` + `tracing-subscriber` exclusively.
