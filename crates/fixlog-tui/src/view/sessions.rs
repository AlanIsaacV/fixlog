//! Sessions overlay — a table of `(SenderCompID, TargetCompID)` canonical
//! pairs with msg counts, seq range, and gap badge.
//!
//! Navigation is local to the overlay: `j`/`k` scroll the cursor; `Enter`
//! applies a filter `49=<sender> AND 56=<target>` and closes the overlay;
//! `Esc` closes without applying.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Clear, Row, Table, TableState};

use fixlog_analysis::sessions::SessionMap;

pub fn render(frame: &mut Frame, area: Rect, map: &SessionMap, cursor: usize) {
    let overlay_area = centered_rect(80, 70, area);
    frame.render_widget(Clear, overlay_area);

    let mut rows: Vec<_> = map.by_key.iter().collect();
    rows.sort_by(|(a, _), (b, _)| a.sender.cmp(&b.sender).then(a.target.cmp(&b.target)));

    let widths = [
        Constraint::Length(30),
        Constraint::Length(8),
        Constraint::Length(16),
        Constraint::Length(16),
        Constraint::Length(6),
    ];

    let header = Row::new(vec!["session", "msgs", "seq-range", "top types", "gaps"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let body: Vec<Row<'static>> = rows
        .iter()
        .map(|(key, stats)| {
            let session = format!(
                "{} ↔ {}",
                String::from_utf8_lossy(&key.sender),
                String::from_utf8_lossy(&key.target),
            );
            let msgs = (stats.in_count + stats.out_count).to_string();
            let seq = match (stats.seq_min, stats.seq_max) {
                (Some(mn), Some(mx)) => format!("{mn}..{mx}"),
                _ => "-".into(),
            };
            let mut types: Vec<_> = stats.by_msg_type.iter().collect();
            types.sort_by(|a, b| b.1.cmp(a.1));
            let types_s = types
                .iter()
                .take(3)
                .map(|(mt, c)| format!("{}={c}", String::from_utf8_lossy(mt)))
                .collect::<Vec<_>>()
                .join(" ");
            let gaps = stats.gaps.len();
            let gap_cell = Cell::from(gaps.to_string()).style(if gaps > 0 {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            });
            Row::new(vec![
                Cell::from(session),
                Cell::from(msgs),
                Cell::from(seq),
                Cell::from(types_s),
                gap_cell,
            ])
        })
        .collect();

    let table = Table::new(body, widths)
        .header(header)
        .block(
            Block::default()
                .title("sessions  (Enter: apply filter · Esc: close)")
                .borders(Borders::ALL),
        )
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut ts = TableState::default();
    if !rows.is_empty() {
        ts.select(Some(cursor.min(rows.len() - 1)));
    }
    frame.render_stateful_widget(table, overlay_area, &mut ts);
}

/// Return the session at `cursor`, sorted the same way we render.
pub fn session_at(map: &SessionMap, cursor: usize) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut rows: Vec<_> = map.by_key.iter().collect();
    rows.sort_by(|(a, _), (b, _)| a.sender.cmp(&b.sender).then(a.target.cmp(&b.target)));
    rows.get(cursor)
        .map(|(k, _)| (k.sender.clone(), k.target.clone()))
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
