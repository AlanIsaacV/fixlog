//! Bookmarks overlay — table of `letter → ordinal → preview`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Clear, Row, Table};

use fixlog_core::parse_one_with_format;

use crate::state::AppState;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let overlay_area = centered_rect(60, 60, area);
    frame.render_widget(Clear, overlay_area);

    let mut entries: Vec<_> = state.bookmarks.iter().collect();
    entries.sort_by_key(|(c, _)| *c);

    let header = Row::new(vec!["mark", "ordinal", "preview"])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<Row<'static>> = entries
        .into_iter()
        .map(|(c, ord)| {
            let preview = state
                .index
                .message_bytes(&state.mmap, *ord as usize)
                .and_then(|b| {
                    parse_one_with_format(b, &state.format)
                        .ok()
                        .map(|(m, _)| preview_msg(&m))
                })
                .unwrap_or_else(|| "-".into());
            Row::new(vec![
                Cell::from(c.to_string()),
                Cell::from(ord.to_string()),
                Cell::from(preview),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Min(20),
    ];
    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .title("bookmarks  (Esc close · m<letter> to set · '<letter> to jump)")
            .borders(Borders::ALL),
    );
    frame.render_widget(table, overlay_area);
}

fn preview_msg(msg: &fixlog_core::RawMessage<'_>) -> String {
    let mt = msg
        .tags
        .iter()
        .find(|(t, _)| *t == 35)
        .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
        .unwrap_or_else(|| "?".into());
    let sender = msg
        .tags
        .iter()
        .find(|(t, _)| *t == 49)
        .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
        .unwrap_or_else(|| "?".into());
    let target = msg
        .tags
        .iter()
        .find(|(t, _)| *t == 56)
        .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
        .unwrap_or_else(|| "?".into());
    format!("{mt}  {sender}→{target}")
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
