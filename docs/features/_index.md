# Features

User-facing capability docs. Each file describes what the feature does, its CLI/TUI surface, the
files involved, the data flow, and edge cases. For per-crate internals see `docs/agent/crates/*.md`.

| Feature | Summary | Doc |
|---------|---------|-----|
| Parsing & format sniffing | Auto-detect layout, zero-copy tokenize, multi-version dictionary resolution. | [parsing-and-format.md](parsing-and-format.md) |
| Query, grep & live tailing | Filter DSL, grep-style matching, `--follow` tailing, hot-tag pre-filter. | [query-grep-tailing.md](query-grep-tailing.md) |
| Interactive TUI | ratatui viewer: virtual list, detail panel, live filter, search, overlays, export. | [interactive-tui.md](interactive-tui.md) |
| Order & session analysis, consolidation | Sessions, order lifecycle, histogram, consolidated orders across rotated logs. | [order-analysis-consolidation.md](order-analysis-consolidation.md) |
