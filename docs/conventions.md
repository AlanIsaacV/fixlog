# Conventions

How code is written, formatted, tested and committed in this repo. Derived from `clippy.toml`,
`rustfmt.toml`, `rust-toolchain.toml`, the workspace `Cargo.toml`, and existing source.

## Toolchain

- **Channel**: `stable` (pinned in `rust-toolchain.toml`, `profile = "minimal"`, with `rustfmt` +
  `clippy` components). Tested against Rust **1.95**, **edition 2024**.
- Workspace metadata lives in `[workspace.package]` (edition, rust-version, license, repo, authors);
  each crate inherits it. Release profile: `lto = true`, `codegen-units = 1`.

## Code style

- **Zero-copy by default.** Parsers and resolvers work on `&[u8]`, not `String`. Prefer `&str` over
  `String` and `&[T]` over `Vec<T>` in public APIs.
- **No `panic!` / `unwrap()` / `expect()` in production code.** Tests may use them freely.
- **`unsafe` requires a `// SAFETY:` comment** stating the invariant being upheld (see
  `fixlog-cli/src/io.rs::mmap_file` for the canonical example). TUI/analysis crates push this
  further with `#![deny(unsafe_code)]` / `#![forbid(unsafe_code)]`.
- **No allocations in hot paths** without a measurable justification.
- `smallvec` is used for small inline buffers (e.g. tag values up to 16 bytes) to avoid heap traffic.

## Formatting & linting

- **Formatter**: `rustfmt` — `edition = 2024`, `max_width = 100`. Run `cargo fmt --all`.
- **Linter**: `cargo clippy --all-targets --all-features -- -D warnings` must pass with **zero
  warnings**. `clippy.toml` sets `avoid-breaking-exported-api = false` (lints may suggest changes to
  public APIs since the workspace is pre-1.0).

## Error handling

- **Library crates** use `thiserror` for typed, public error enums (`ParserError`, `IndexError`,
  `QueryError`, `AnalysisError`, `TuiError`, …). Streaming I/O failures surface as dedicated
  variants (e.g. `AnalysisError::Io(std::io::Error)`).
- **The binary** (`fixlog-cli`) uses `anyhow` with `.with_context(...)` for human-readable failure
  messages. Map library errors into `anyhow` at the command boundary.
- **Malformed input is data, not an error.** Checksum/BodyLength mismatches and empty lines are
  expected in real logs; emit the message and log at `debug`, never abort the run.

## Logging

- `tracing` throughout; the CLI initializes `tracing-subscriber` writing to **stderr**.
- Verbosity: `-v` → `info`, `-vv` → `debug` (default `warn`). `RUST_LOG` overrides the level filter
  (e.g. `RUST_LOG=fixlog_parser=debug`).

## Testing

- **Unit tests** live in the same file under `#[cfg(test)] mod tests`.
- **Integration tests** live in `crates/<crate>/tests/`.
- **Golden tests** for the parser: FIX input → expected resolved/JSON output.
- **Fixtures**: `fixtures/synthetic/` is versioned, deterministic, and committed (fixed timestamps so
  regenerations are byte-identical). `fixtures/real/` and `fixtures/orders/` are **gitignored real
  data** — never commit them; anonymize before adding anything new. See `fixtures/README.md`.
- Critical parser paths must have tests. Coverage is not a hard gate, but the full suite
  (`cargo test --all`) must be green before every commit.
- **Benchmarks**: `criterion` (`cargo bench -p <crate>`). Anti-regression anchors documented in
  `docs/agent/state.md` (parser throughput, index speedup, TUI frame budget, analysis timings).

## API design

> [INFERRED] from existing crate boundaries — validate against intent.

- Public APIs take borrowed input (`&[u8]`, `&str`, `&LogFormat`) and return owned aggregates only
  when the result must outlive the source (e.g. `ResolvedMessageOwned` exists because the mmap is
  swapped under `--follow`).
- Crates stay single-purpose and acyclic (see the dependency graph in `docs/architecture.md`).
  Cross-crate primitives flow through `fixlog-core`; analysis composes on top and is not re-exported.

## Commits

- **Conventional commits**: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`, `perf:`,
  `bench:`. Optional scope, e.g. `feat(tui): …`, `feat(analysis): …`.
- One logical unit per commit — no monster commits, no bundling unrelated phases.
- **Never commit without explicit instruction.** Run `cargo fmt --all`, then the clippy and test
  gates, before staging.

## Collaboration with the agent

- Plan-mode first for architectural tasks; confirm the approach before coding.
- Validate changes against real fixtures before declaring a task done.
- `docs/agent/state.md` is the authoritative record of current status (phases, task tables, benchmarks).
