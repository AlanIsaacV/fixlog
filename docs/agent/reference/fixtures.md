# Fixture catalog

Quick reference for `fixtures/`. For narrative details see `fixtures/README.md`.

## `fixtures/synthetic/` (committed, deterministic)

| File | Separator | Prefix | Expected messages | Purpose |
|------|-----------|--------|-------------------|---------|
| `minimal_4.4.log` | SOH | — | 10 valid, 0 errors | Baseline. 7 MsgTypes: A (1), 0 (2), 1 (1), D (1), 8 (3), F (1), 5 (1). |
| `pipe_separated.log` | `\|` | — | 10 valid | Pipe-separator variant of the above. |
| `with_timestamp_prefix.log` | SOH | `YYYYMMDD-HH:MM:SS.sss : ` | 10 valid | QuickFIX-style prefix; exercises sniffer line-prefix detection (though the parser ignores it — see `crates/parser.md`). |
| `malformed.log` | SOH | — | 5 emitted / 1 structural error | Robustness. Mix of: 3 valid + 1 empty-value (surfaced anyway) + 1 bad-checksum (emitted, checksum non-fatal) + 1 bad-BodyLength (structural error, walks past EOF). |

Checksums and body lengths in synthetic files are correct **for the rendered bytes** (pipe or SOH, prefix or none). Regenerating these files requires byte-identical output; timestamps are fixed at `20260416-…`.

### Golden test expectations

Encoded in `crates/fixlog-parser/tests/synthetic.rs`:

- `minimal_4_4_parses_all_ten_messages`: 10 ok, 0 err, exact MsgType counts `{A:1, 0:2, 1:1, D:1, 8:3, F:1, 5:1}`.
- `malformed_emits_valid_messages_and_logs_errors`: 5 ok, 1 err; MsgTypes `{A:1, 0:1, 1:1, D:2}`.
- `parse_all_yields_distinct_offsets_for_each_message`: offsets strictly increasing.

Encoded in `tests/with_format.rs`: sniffing `pipe_separated.log` detects `Separator::Pipe`; sniffing `with_timestamp_prefix.log` detects `LinePrefix::Fixed(24)`; both parse to 10 messages via `parse_all_with_format`.

## `fixtures/real/` (gitignored)

User-provided, anonymized offline. These are **not** in version control; they live on the developer's disk. Do not assume presence in CI.

Naming: `<version>-<kind>.log` e.g. `fix44-om.log` (FIX 4.4 order management), `fixt11-md.log` (FIXT.1.1 market data).

Last-known parse stats (for reference when debugging regressions):

| File | Size | Messages | Errors | MsgType breakdown |
|------|------|----------|--------|-------------------|
| `fix44-om.log` | 2.1 MB | 5419 | 0 | 8 (3124), D (1535), 0 (489), F (268), A (2), 9 (1). |
| `fixt11-md.log` | 8.7 MB | 8229 | 0 | X (5359), W (1980), V (435), x (145), y (145), 0 (105), CO (20), Y (18), j (12), 3 (6), plus 2 Logon, 1 Logout. |

`fixt11-md.log` contains a re-rendered log (SOH → `|`) so all messages have checksum mismatches — the parser emits them anyway (see `crates/parser.md` checksum invariant).

## Loading a fixture from test code

```rust
fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
        .join(rel)
}
```

Use `std::fs::read(...)` for synthetic fixtures (small); use `memmap2::Mmap` for real ones if the test is size-sensitive.

## Adding a new synthetic fixture

1. Generate the bytes with correct BodyLength and CheckSum for the **rendered** separator.
2. Drop into `fixtures/synthetic/<name>.log`.
3. Add a row to `fixtures/README.md` and to this catalog.
4. Add a golden test under the parser's or dict's `tests/` that asserts exact counts.

## Do not

- Do not commit anything from `fixtures/real/`. It's ignored for a reason.
- Do not let synthetic-fixture tests depend on `fixtures/real/` — real fixtures may not exist on CI / new-clone machines.
