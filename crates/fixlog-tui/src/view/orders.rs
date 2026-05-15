//! Order-lifecycle overlay — Gantt bar on top, per-event table below.
//!
//! The table is stateful: `cursor` (stored on the `Overlay::Orders`
//! variant) selects a row and drives the ratatui `TableState`, so the
//! viewport scrolls automatically when the selection moves past the
//! visible rows. `Enter` on a row jumps the main list to that message
//! (handled in `app.rs` → `overlay_intercept`).

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};

use fixlog_analysis::orders::{OrderTimeline, render_gantt};

pub fn render(frame: &mut Frame, area: Rect, timeline: &OrderTimeline, cursor: usize) {
    let overlay_area = centered_rect(90, 70, area);
    frame.render_widget(Clear, overlay_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header + gantt
            Constraint::Min(1),    // event table
        ])
        .split(overlay_area);

    let clord = String::from_utf8_lossy(&timeline.clordid);
    let order_ids = timeline
        .order_ids
        .iter()
        .map(|o| String::from_utf8_lossy(o).into_owned())
        .collect::<Vec<_>>()
        .join(", ");
    let gantt = render_gantt(timeline, chunks[0].width.saturating_sub(4) as usize);
    let top = format!(
        "ClOrdID: {clord}   OrderID(s): [{order_ids}]   events: {}\n{gantt}",
        timeline.events.len()
    );
    frame.render_widget(
        Paragraph::new(top).block(
            Block::default()
                .title("order lifecycle  (j/k: move, Enter: jump, Esc: close)")
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    // Header order groups per-fill values (LastQty/LastPx) next to the
    // cumulative ones (CumQty/AvgPx) so the distinction reads naturally:
    // "this fill" → "running total".
    let header = Row::new(vec![
        "time", "type", "exec", "status", "LastQty", "LastPx", "CumQty", "AvgPx",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<Row<'static>> = timeline
        .events
        .iter()
        .map(|e| {
            let style = color_for_exec_type(e.exec_type.as_deref())
                .map(|c| Style::default().fg(c))
                .unwrap_or_default();
            Row::new(vec![
                Cell::from(format_time(e.sending_time)),
                Cell::from(String::from_utf8_lossy(&e.msg_type).into_owned()),
                Cell::from(fmt_bytes_or_dash(e.exec_type.as_deref())),
                Cell::from(fmt_bytes_or_dash(e.ord_status.as_deref())),
                Cell::from(fmt_bytes_or_dash(e.last_qty.as_deref())),
                Cell::from(fmt_bytes_or_dash(e.last_px.as_deref())),
                Cell::from(fmt_bytes_or_dash(e.cum_qty.as_deref())),
                Cell::from(fmt_bytes_or_dash(e.avg_px.as_deref())),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(19), // "YYYY-MM-DD HH:MM:SS"
        Constraint::Length(6),  // type
        Constraint::Length(6),  // exec
        Constraint::Length(8),  // status
        Constraint::Length(9),  // LastQty
        Constraint::Length(11), // LastPx
        Constraint::Length(9),  // CumQty
        Constraint::Length(11), // AvgPx
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .block(Block::default().borders(Borders::ALL));

    let mut ts = TableState::default();
    if !timeline.events.is_empty() {
        ts.select(Some(cursor.min(timeline.events.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, chunks[1], &mut ts);
}

/// Format an optional FIX byte field as `String`, falling back to "-"
/// when absent. Used for the per-fill / cumulative columns where the
/// underlying value is already a numeric string in the wire bytes.
fn fmt_bytes_or_dash(v: Option<&[u8]>) -> String {
    v.map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_else(|| "-".into())
}

/// Format a `SystemTime` as `YYYY-MM-DD HH:MM:SS` (UTC). We don't pull in
/// a date crate for this: the timestamp comes from our own
/// `parse_sending_time`, so we can invert the civil-from-days algorithm
/// locally. Missing timestamps render as `-`.
fn format_time(t: Option<SystemTime>) -> String {
    let Some(t) = t else {
        return "-".into();
    };
    let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) else {
        return "-".into();
    };
    let total_secs = d.as_secs();
    let days = total_secs / 86_400;
    let secs_of_day = total_secs % 86_400;
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    let (y, m, day) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// Inverse of Howard Hinnant's `days_from_civil`. Input: days since
/// 1970-01-01 (unix epoch). Output: `(year, month, day)` in the
/// proleptic Gregorian calendar.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i32 + (era as i32) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp.wrapping_sub(9) };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn color_for_exec_type(t: Option<&[u8]>) -> Option<Color> {
    match t? {
        b"A" => Some(Color::DarkGray),    // PendingNew
        b"0" | b"5" => Some(Color::Blue), // New / Replaced
        b"1" => Some(Color::Yellow),      // Partial
        b"F" => Some(Color::Green),       // Trade (Fill)
        b"4" | b"8" => Some(Color::Red),  // Canceled / Rejected
        _ => None,
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_roundtrips_known_dates() {
        // 1970-01-01 is day 0.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-04-17 = 20560 days since 1970-01-01 (matches util.rs test).
        assert_eq!(civil_from_days(20_560), (2026, 4, 17));
        // 2000-02-29 — leap year, day 10_956.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn format_time_matches_iso_like_shape() {
        // 2026-04-17 12:34:56 UTC = 20560 * 86400 + 45296 seconds.
        let secs = 20_560u64 * 86_400 + 45_296;
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        assert_eq!(format_time(Some(t)), "2026-04-17 12:34:56");
        assert_eq!(format_time(None), "-");
    }
}
