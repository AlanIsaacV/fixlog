//! Parse and execute command-bar commands (`:q`, `:filter <expr>`, `:help`).
//!
//! Kept separate from `app.rs` so the parser/executor can be unit-tested
//! without constructing an `App`. The grammar is minimal by design — we
//! reuse `fixlog-query` for filter expressions, so there's nothing clever to
//! parse here.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fixlog_analysis::histogram::Histogram;
use fixlog_analysis::orders::OrderTimeline;
use fixlog_analysis::orders_consolidated::ConsolidatedBuilder;
use fixlog_analysis::sessions::SessionMap;
use fixlog_core::query::parse as parse_query;

use crate::export::{self, ExportFormat};
use crate::state::{AppState, Overlay, StatusMessage, recompute_effective_filter};
use crate::view::consolidated::ConsolidatedView;

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
    Consolidated,
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
        "consolidated" | "consolidate" => Command::Consolidated,
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
            state.open_overlay(Overlay::Orders {
                timeline: tl,
                cursor: 0,
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

/// `:consolidated` — stream the current mmap through
/// [`ConsolidatedBuilder`] and open an overlay with one row per order
/// family. Bounded by mmap size; for very large logs this may stall the
/// UI for a second or two (no caching yet — fase 5).
pub(crate) fn open_consolidated_overlay(state: &mut AppState) {
    let bytes: &[u8] = &state.mmap;
    let mut builder = ConsolidatedBuilder::new();
    match builder.push_source(std::io::Cursor::new(bytes), &state.format) {
        Ok(_stats) => {
            let rows = builder.finish();
            if rows.is_empty() {
                state.status = StatusMessage::warn("no orders found in current log");
                return;
            }
            let view = Arc::new(ConsolidatedView::from_rows(rows));
            let n = view.rows.len();
            state.open_overlay(Overlay::Consolidated {
                view,
                cursor: 0,
                viewport_top: 0,
            });
            state.status = StatusMessage::info(format!("consolidated: {n} orders"));
        }
        Err(e) => {
            state.status = StatusMessage::error(format!("consolidate failed: {e}"));
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
            state.open_overlay(Overlay::Help { scroll: 0 });
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
            state.open_overlay(Overlay::Sessions { map, cursor: 0 });
            state.status = StatusMessage::info(format!("sessions: {count}"));
            Outcome::Continue
        }
        Command::Orders(id) => {
            open_orders_overlay(state, id.as_deref());
            Outcome::Continue
        }
        Command::Consolidated => {
            open_consolidated_overlay(state);
            Outcome::Continue
        }
        Command::Marks => {
            state.open_overlay(Overlay::Marks { cursor: 0 });
            Outcome::Continue
        }
        Command::Histogram(bucket) => {
            let h = Histogram::build(&state.index, &state.mmap, &state.format, bucket);
            let total = h.total();
            state.open_overlay(Overlay::Histogram {
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
            state.close_overlay();
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

    #[test]
    fn parse_consolidated_aliases() {
        assert_eq!(parse("consolidated"), Command::Consolidated);
        assert_eq!(parse("consolidate"), Command::Consolidated);
    }

    /// Build a FIX message with a correct BodyLength + CheckSum.
    fn build_msg(body_fields: &str) -> Vec<u8> {
        let body_len = body_fields.len();
        let head = format!("8=FIX.4.4\x019={body_len}\x01");
        let payload: Vec<u8> = head.bytes().chain(body_fields.bytes()).collect();
        let sum: u8 = payload.iter().fold(0u8, |a, b| a.wrapping_add(*b));
        let trailer = format!("10={sum:03}\x01");
        payload.into_iter().chain(trailer.bytes()).collect()
    }

    fn synthetic_log_two_orders() -> Vec<u8> {
        let tail = "49=A\x0156=B\x01";
        let t = |sec| format!("52=20260417-12:34:{sec:02}\x01");
        let mut out = Vec::new();
        // Order ABC: D + fill 60@10.5
        out.extend(build_msg(&format!(
            "35=D\x0134=1\x01{tail}{}11=ABC\x0155=AAPL\x0154=1\x0138=100\x01",
            t(1)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=2\x01{tail}{}11=ABC\x0137=ord1\x0117=E1\x01150=F\x0139=1\x0114=60\x0131=10.5\x0132=60\x01",
            t(2)
        )));
        // Order DEF: D + fill 50@100
        out.extend(build_msg(&format!(
            "35=D\x0134=3\x01{tail}{}11=DEF\x0155=MSFT\x0154=2\x0138=50\x01",
            t(3)
        )));
        out.extend(build_msg(&format!(
            "35=8\x0134=4\x01{tail}{}11=DEF\x0137=ord2\x0117=E2\x01150=F\x0139=2\x0114=50\x0131=100\x0132=50\x01",
            t(4)
        )));
        out
    }

    #[test]
    fn open_consolidated_opens_overlay_with_rows() {
        use crate::state::{Overlay, bootstrap_with_sort};
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&synthetic_log_two_orders())
            .unwrap();

        let mut state = bootstrap_with_sort(&path, None, crate::state::SortKey::Natural).unwrap();
        open_consolidated_overlay(&mut state);

        let Some(Overlay::Consolidated { view, cursor, .. }) = state.overlay.as_ref() else {
            panic!("expected Consolidated overlay, got {:?}", state.overlay);
        };
        assert_eq!(view.rows.len(), 2, "two orders in the log");
        assert_eq!(*cursor, 0);

        // Highest-notional first (DEF: 5000 vs ABC: 630).
        assert_eq!(view.rows[0].root_clordid, b"DEF");
        assert_eq!(view.rows[1].root_clordid, b"ABC");
    }

    #[test]
    fn drill_into_consolidated_opens_orders_overlay() {
        use crate::state::{Overlay, bootstrap_with_sort};
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&synthetic_log_two_orders())
            .unwrap();
        let mut state = bootstrap_with_sort(&path, None, crate::state::SortKey::Natural).unwrap();
        open_consolidated_overlay(&mut state);

        // Simulate Enter: take the cursor row, open `:orders <id>`. This
        // matches what `App::drill_into_consolidated_selection` does.
        let clordid = match state.overlay.as_ref() {
            Some(Overlay::Consolidated { view, cursor, .. }) => {
                view.rows[*cursor].root_clordid.clone()
            }
            _ => panic!("consolidated overlay missing"),
        };
        let id = String::from_utf8(clordid).unwrap();
        open_orders_overlay(&mut state, Some(&id));

        match state.overlay.as_ref() {
            Some(Overlay::Orders { timeline, .. }) => {
                assert_eq!(timeline.clordid, b"DEF");
                assert!(timeline.events.len() >= 2, "D + fill at minimum");
            }
            other => panic!("expected Orders overlay after drill-down, got {other:?}"),
        }
    }
}
