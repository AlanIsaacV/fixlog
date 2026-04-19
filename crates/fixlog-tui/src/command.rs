//! Parse and execute command-bar commands (`:q`, `:filter <expr>`, `:help`).
//!
//! Kept separate from `app.rs` so the parser/executor can be unit-tested
//! without constructing an `App`. The grammar is minimal by design — we
//! reuse `fixlog-query` for filter expressions, so there's nothing clever to
//! parse here.

use std::path::PathBuf;
use std::time::Duration;

use fixlog_analysis::histogram::Histogram;
use fixlog_analysis::orders::OrderTimeline;
use fixlog_analysis::sessions::SessionMap;
use fixlog_core::query::parse as parse_query;

use crate::export::{self, ExportFormat};
use crate::state::{AppState, Overlay, StatusMessage, recompute_effective_filter};

/// Result of executing a command. `Quit` signals the event loop to shut
/// down; `Continue` means keep running (possibly after mutating state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Quit,
}

/// Parsed command. Unknown text is surfaced as `Unknown` so the executor
/// can show a status-bar error instead of silently dropping it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Quit,
    Help,
    SetFilter(String),
    ClearFilter,
    Sessions,
    Orders(Option<String>),
    Marks,
    Histogram(Duration),
    Export { fmt: ExportFormat, path: PathBuf },
    DiffClear,
    Unknown(String),
}

pub fn parse(input: &str) -> Command {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Command::Unknown(String::new());
    }
    let (head, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((h, r)) => (h, r.trim()),
        None => (trimmed, ""),
    };
    match head {
        "q" | "quit" => Command::Quit,
        "help" | "h" => Command::Help,
        "filter" | "f" => {
            if rest.is_empty() {
                Command::ClearFilter
            } else {
                Command::SetFilter(rest.to_string())
            }
        }
        "sessions" => Command::Sessions,
        "orders" => {
            let id = if rest.is_empty() {
                None
            } else {
                Some(rest.to_string())
            };
            Command::Orders(id)
        }
        "marks" => Command::Marks,
        "histogram" => {
            let bucket = if rest.is_empty() {
                Duration::from_secs(1)
            } else {
                parse_duration(rest).unwrap_or(Duration::from_secs(1))
            };
            Command::Histogram(bucket)
        }
        "export" => {
            // `:export <fmt> <path>` — two whitespace-separated args.
            let mut parts = rest.splitn(2, char::is_whitespace);
            match (parts.next(), parts.next()) {
                (Some(fmt_s), Some(path_s)) if !fmt_s.is_empty() && !path_s.trim().is_empty() => {
                    match ExportFormat::parse(fmt_s) {
                        Some(fmt) => Command::Export {
                            fmt,
                            path: PathBuf::from(path_s.trim()),
                        },
                        None => Command::Unknown(trimmed.to_string()),
                    }
                }
                _ => Command::Unknown(trimmed.to_string()),
            }
        }
        "diff" => {
            if rest.trim() == "clear" {
                Command::DiffClear
            } else {
                Command::Unknown(trimmed.to_string())
            }
        }
        _ => Command::Unknown(trimmed.to_string()),
    }
}

/// Shared handler for `:orders <id>` / `:orders` / the `O` keybinding.
/// When `id` is `None`, extracts tag 11 from the ordinal under the cursor.
pub(crate) fn open_orders_overlay(state: &mut AppState, id: Option<&str>) {
    use fixlog_core::index::secondary::TAG_CL_ORD_ID;
    use fixlog_core::parse_one_with_format;

    let clordid: Vec<u8> = match id {
        Some(s) => s.as_bytes().to_vec(),
        None => {
            if state.visible.is_empty() {
                state.status = StatusMessage::warn("no message under cursor");
                return;
            }
            let ord = state.visible[state.cursor] as usize;
            let Some(bytes) = state.index.message_bytes(&state.mmap, ord) else {
                state.status = StatusMessage::error("cursor out of range");
                return;
            };
            let Ok((msg, _)) = parse_one_with_format(bytes, &state.format) else {
                state.status = StatusMessage::error("cannot parse current message");
                return;
            };
            let Some((_, v)) = msg.tags.iter().find(|(t, _)| *t == TAG_CL_ORD_ID) else {
                state.status = StatusMessage::warn("no ClOrdID in this message");
                return;
            };
            v.to_vec()
        }
    };

    match OrderTimeline::build(&state.index, &state.mmap, &state.format, &clordid) {
        Some(tl) => {
            let n = tl.events.len();
            state.overlay = Some(Overlay::Orders {
                timeline: tl,
                scroll: 0,
            });
            state.status = StatusMessage::info(format!("timeline: {n} events"));
        }
        None => {
            state.status = StatusMessage::warn(format!(
                "no events found for ClOrdID={}",
                String::from_utf8_lossy(&clordid)
            ));
        }
    }
}

fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (num_str, mult_ns) = if let Some(r) = s.strip_suffix("ms") {
        (r, 1_000_000_u128)
    } else if let Some(r) = s.strip_suffix("us") {
        (r, 1_000_u128)
    } else if let Some(r) = s.strip_suffix("ns") {
        (r, 1_u128)
    } else if let Some(r) = s.strip_suffix('s') {
        (r, 1_000_000_000_u128)
    } else if let Some(r) = s.strip_suffix('m') {
        (r, 60_000_000_000_u128)
    } else {
        return None;
    };
    let n: u64 = num_str.trim().parse().ok()?;
    let total_ns = (n as u128).checked_mul(mult_ns)?;
    let secs = (total_ns / 1_000_000_000) as u64;
    let nanos = (total_ns % 1_000_000_000) as u32;
    Some(Duration::new(secs, nanos))
}

/// Execute a parsed command against mutable app state. Returns whether the
/// event loop should keep running. Status-bar feedback is written into
/// `state.status`.
pub fn execute(state: &mut AppState, cmd: Command) -> Outcome {
    match cmd {
        Command::Quit => Outcome::Quit,
        Command::Help => {
            state.overlay = Some(Overlay::Help { scroll: 0 });
            Outcome::Continue
        }
        Command::SetFilter(expr) => match parse_query(&expr) {
            Ok(_) => {
                state.user_filter_text = Some(expr);
                recompute_effective_filter(state);
                state.status =
                    StatusMessage::info(format!("filter applied — {} match", state.visible.len()));
                Outcome::Continue
            }
            Err(e) => {
                state.status = StatusMessage::error(format!("invalid filter: {e}"));
                Outcome::Continue
            }
        },
        Command::ClearFilter => {
            state.user_filter_text = None;
            recompute_effective_filter(state);
            state.status = StatusMessage::info("filter cleared");
            Outcome::Continue
        }
        Command::Sessions => {
            let map = SessionMap::build(&state.index, &state.mmap, &state.format);
            let count = map.by_key.len();
            state.overlay = Some(Overlay::Sessions { map, cursor: 0 });
            state.status = StatusMessage::info(format!("sessions: {count}"));
            Outcome::Continue
        }
        Command::Orders(id) => {
            open_orders_overlay(state, id.as_deref());
            Outcome::Continue
        }
        Command::Marks => {
            state.overlay = Some(Overlay::Marks);
            Outcome::Continue
        }
        Command::Histogram(bucket) => {
            let h = Histogram::build(&state.index, &state.mmap, &state.format, bucket);
            let total = h.total();
            state.overlay = Some(Overlay::Histogram {
                histogram: h,
                width: 80,
            });
            state.status = StatusMessage::info(format!("histogram: {total} bins"));
            Outcome::Continue
        }
        Command::Export { fmt, path } => {
            match export::export(state, fmt, &path) {
                Ok(n) => {
                    state.status =
                        StatusMessage::info(format!("exported {n} msgs to {}", path.display()));
                }
                Err(e) => {
                    state.status = StatusMessage::error(format!("export failed: {e}"));
                }
            }
            Outcome::Continue
        }
        Command::DiffClear => {
            state.diff_slots = [None, None];
            state.overlay = None;
            state.status = StatusMessage::info("diff slots cleared");
            Outcome::Continue
        }
        Command::Unknown(raw) => {
            let msg = if raw.is_empty() {
                "empty command".to_string()
            } else {
                format!("unknown command: {raw}")
            };
            state.status = StatusMessage::error(msg);
            Outcome::Continue
        }
    }
}

/// Live-preview hook: invoked from the event loop after every keystroke
/// while the user is editing the command buffer. If the buffer looks like
/// `filter …` / `f …`, compile and apply the expression in-place so the
/// visible set updates as you type. Malformed or partial expressions are
/// silently kept from overwriting the preview until they parse cleanly.
pub fn live_preview(state: &mut AppState, buffer: &str) {
    let trimmed = buffer.trim_start();
    let expr_str = if let Some(rest) = trimmed.strip_prefix("filter ") {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix("f ") {
        rest.trim()
    } else if trimmed == "filter" || trimmed == "f" {
        ""
    } else {
        return;
    };

    if expr_str.is_empty() {
        // Preview clearing the user filter so they see the unfiltered list
        // (modulo hide_heartbeat) while they're about to type an expression.
        state.user_filter_text = None;
        recompute_effective_filter(state);
        return;
    }
    if parse_query(expr_str).is_ok() {
        state.user_filter_text = Some(expr_str.to_string());
        recompute_effective_filter(state);
    }
    // Malformed: leave previous preview alone — this is the "silent" rule.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quit_aliases() {
        assert_eq!(parse("q"), Command::Quit);
        assert_eq!(parse("quit"), Command::Quit);
        assert_eq!(parse("  q  "), Command::Quit);
    }

    #[test]
    fn parse_help() {
        assert_eq!(parse("help"), Command::Help);
        assert_eq!(parse("h"), Command::Help);
    }

    #[test]
    fn parse_filter_with_expression() {
        assert_eq!(parse("filter 35=D"), Command::SetFilter("35=D".to_string()));
        assert_eq!(
            parse("f 35=D AND 55=AAPL"),
            Command::SetFilter("35=D AND 55=AAPL".to_string())
        );
    }

    #[test]
    fn parse_filter_without_arg_clears() {
        assert_eq!(parse("filter"), Command::ClearFilter);
        assert_eq!(parse("filter   "), Command::ClearFilter);
    }

    #[test]
    fn parse_unknown() {
        assert_eq!(parse("wat"), Command::Unknown("wat".into()));
        assert_eq!(parse(""), Command::Unknown(String::new()));
    }
}
