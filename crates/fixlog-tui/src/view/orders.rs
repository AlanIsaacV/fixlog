//! Order-lifecycle overlay — Gantt bar on top, per-event table below.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

use fixlog_analysis::orders::{OrderTimeline, render_gantt};

pub fn render(frame: &mut Frame, area: Rect, timeline: &OrderTimeline, _scroll: usize) {
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
                .title("order lifecycle  (Esc: close)")
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    let header = Row::new(vec!["ord", "type", "exec", "status", "cum-qty"])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<Row<'static>> = timeline
        .events
        .iter()
        .map(|e| {
            let style = color_for_exec_type(e.exec_type.as_deref())
                .map(|c| Style::default().fg(c))
                .unwrap_or_default();
            Row::new(vec![
                Cell::from(e.ordinal.to_string()),
                Cell::from(String::from_utf8_lossy(&e.msg_type).into_owned()),
                Cell::from(
                    e.exec_type
                        .as_ref()
                        .map(|v| String::from_utf8_lossy(v).into_owned())
                        .unwrap_or_else(|| "-".into()),
                ),
                Cell::from(
                    e.ord_status
                        .as_ref()
                        .map(|v| String::from_utf8_lossy(v).into_owned())
                        .unwrap_or_else(|| "-".into()),
                ),
                Cell::from(
                    e.cum_qty
                        .as_ref()
                        .map(|v| String::from_utf8_lossy(v).into_owned())
                        .unwrap_or_else(|| "-".into()),
                ),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(10),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(table, chunks[1]);
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
