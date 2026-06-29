# Interactive TUI

## What it does

An interactive terminal viewer (`fixlog tui`) for browsing large FIX logs: a virtual, scrollable
list with semantic columns, a resolved detail panel, a live filter with preview, search, and a stack
of analysis overlays — all rendering well under the 16 ms frame budget (~737 µs/frame on 1M messages).

## Launching

```bash
fixlog tui FILE [--filter EXPR] [-F/--follow] [--sort natural|seq|transact|sending]
rg "35=D" *.log | fixlog tui            # stdin pipe (FILE omitted); --follow rejected
```

## Surface

- **List columns**: `TIME | MESSAGE | CLIENT ORDER ID | STATUS | DETAIL` (per-MsgType semantic
  summary; unknown types fall back to Side/Qty/Symbol).
- **Navigation**: vim-style `j/k`, `g/G`, `Ctrl+D/U`, `PageUp/Down`; `F` toggles Follow/Browse;
  `o` cycles sort key (natural / 34 / 60 / 52).
- **Filter & search**: `:filter EXPR` (live preview, `Esc` rolls back), `/` search + `n`/`N`,
  `f`/`x` filter-from-detail (`tag=value` / `NOT`), `c` skip header/trailer tags, `H` hide heartbeats.
- **Overlays**: `:sessions`, `:orders [id]` / `O`, `:diff` (`dd` set A, `dD`/`D` set B), `:marks`,
  `:histogram [bucket]`, `:consolidated`. Overlays are navigable and support an `Esc` parent-stack
  (drill from `:consolidated` → `Enter` → order timeline → `Esc` back to consolidated).
- **Bookmarks**: `m<0-9>` set, `'<0-9>` jump. **Yank**: `yy` (raw), `yY` (pretty) to clipboard.
- **Export**: `:export <csv|json|fix|pretty> <path>` writes the current filtered view.
- **Help**: `?` or `:help` opens a scrollable overlay listing all keys/commands.

## Files involved

- `crates/fixlog-tui/src/` — `state.rs` (`AppState`, `SortKey`, overlays), `app.rs`
  (`on_event`, `overlay_intercept`, parent-stack), `input.rs` (`map_key → Action`),
  `command.rs` (`:` commands), `follow.rs` (watcher), `view/*` (list, detail, status, command,
  overlays: sessions/orders/diff/marks/histogram/consolidated/help), `summary.rs`, `theme.rs`,
  `clipboard.rs`, `export.rs`. See `docs/agent/crates/tui.md`.
- `crates/fixlog-cli/src/commands/tui.rs` — thin CLI wrapper + stdin/`/dev/tty` handling.

## Data flow

`bootstrap` mmaps + sniffs + builds the parallel index + compiles the initial filter into
`AppState`. The event loop maps crossterm events → `Action`s → state mutations; the renderer draws
only the visible slice of the list plus the detail panel for the message under the cursor (parsed
lazily, cached by ordinal so it survives mmap swaps under `--follow`). Overlays are built on demand
and rendered O(viewport).

## Edge cases

- **`:consolidated` blocks the foreground thread** while it streams the whole mmap (no caching /
  append-invalidation yet) — noticeable on very large live files.
- **Follow latency** up to ~250 ms (stat-based poll, not `notify`).
- **Clipboard unavailable** (headless/CI, no display) surfaces as a status message, never a crash.
- **Stdin pipe**: after draining the pipe the TUI re-points stdin at `/dev/tty` (Unix only) so the
  keyboard still works; Windows requires a file path.
