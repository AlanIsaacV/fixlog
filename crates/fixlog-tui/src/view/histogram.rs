//! Histogram overlay — sparkline + top peaks table.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

use fixlog_analysis::histogram::Histogram;

pub fn render(frame: &mut Frame, area: Rect, histogram: &Histogram, _width_hint: usize) {
    let overlay_area = centered_rect(80, 60, area);
    frame.render_widget(Clear, overlay_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(1)])
        .split(overlay_area);

    let inner_w = chunks[0].width.saturating_sub(4) as usize;
    let spark = if histogram.bins.is_empty() {
        String::from("(no timestamped messages)")
    } else {
        histogram.render_sparkline(inner_w.max(10))
    };
    let header = format!(
        "bucket: {} ns   total: {}   no-time: {}\n{spark}",
        histogram.bucket_ns,
        histogram.total(),
        histogram.dropped_no_time,
    );
    frame.render_widget(
        Paragraph::new(header).block(
            Block::default()
                .title("histogram  (Esc close)")
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    let header_row = Row::new(vec!["count", "start_ns", "end_ns"])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<Row<'static>> = histogram
        .peaks(20)
        .into_iter()
        .map(|b| {
            Row::new(vec![
                Cell::from(b.count.to_string()),
                Cell::from(b.start_ns.to_string()),
                Cell::from(b.end_ns.to_string()),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(8),
        Constraint::Length(22),
        Constraint::Length(22),
    ];
    let table = Table::new(rows, widths)
        .header(header_row)
        .block(Block::default().title("top peaks").borders(Borders::ALL));
    frame.render_widget(table, chunks[1]);
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
