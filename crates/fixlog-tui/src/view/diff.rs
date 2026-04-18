//! Diff overlay — two messages side by side, one row per tag in the union.

use std::collections::BTreeMap;

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Clear, Row, Table};

use fixlog_core::{parse_one_with_format, resolve};

use crate::state::AppState;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState, a_ord: u32, b_ord: u32) {
    let overlay_area = centered_rect(90, 80, area);
    frame.render_widget(Clear, overlay_area);

    let a_fields = resolve_fields(state, a_ord);
    let b_fields = resolve_fields(state, b_ord);

    // Union of tags, BTreeMap for stable sorted order.
    let mut all_tags: BTreeMap<u32, (Option<String>, Option<String>)> = BTreeMap::new();
    for (tag, val) in &a_fields {
        all_tags.insert(*tag, (Some(val.clone()), None));
    }
    for (tag, val) in &b_fields {
        all_tags
            .entry(*tag)
            .and_modify(|e| e.1 = Some(val.clone()))
            .or_insert((None, Some(val.clone())));
    }

    let header =
        Row::new(vec!["tag", "A", "B"]).style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<Row<'static>> = all_tags
        .iter()
        .map(|(tag, (a, b))| {
            let style = match (a, b) {
                (Some(x), Some(y)) if x == y => Style::default().add_modifier(Modifier::DIM),
                (Some(_), Some(_)) => Style::default().fg(Color::Red),
                (Some(_), None) => Style::default().fg(Color::Yellow),
                (None, Some(_)) => Style::default().fg(Color::Cyan),
                (None, None) => Style::default(),
            };
            Row::new(vec![
                Cell::from(tag.to_string()),
                Cell::from(a.clone().unwrap_or_else(|| "—".into())),
                Cell::from(b.clone().unwrap_or_else(|| "—".into())),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(6),
        Constraint::Percentage(47),
        Constraint::Percentage(47),
    ];
    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .title(format!(
                "diff  A=#{a_ord}  B=#{b_ord}  (Esc close · :diff clear)"
            ))
            .borders(Borders::ALL),
    );
    frame.render_widget(table, overlay_area);
}

fn resolve_fields(state: &AppState, ord: u32) -> Vec<(u32, String)> {
    let Some(bytes) = state.index.message_bytes(&state.mmap, ord as usize) else {
        return Vec::new();
    };
    let Ok((msg, _)) = parse_one_with_format(bytes, &state.format) else {
        return Vec::new();
    };
    let r = resolve(&msg);
    r.fields
        .iter()
        .map(|f| (f.tag, String::from_utf8_lossy(f.value).into_owned()))
        .collect()
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
