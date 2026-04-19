//! Bookmarks overlay — same semantic columns as the main list
//! (`time | message | client order id | status | detail`) prefixed by
//! a `mark` column. Navigable with `j`/`k`; `Enter` jumps the main
//! cursor to the selected bookmark's ordinal and closes the overlay.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::state::AppState;
use crate::view::list::{
    COL_CLORDID, COL_MESSAGE, COL_SEP, COL_STATUS, COL_TIME, build_line_for_ord, header_line,
    pad_cell,
};

/// Width of the leading "mark" column — one digit plus padding.
const COL_MARK: usize = 4;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState, cursor: usize) {
    let overlay_area = centered_rect(90, 60, area);
    frame.render_widget(Clear, overlay_area);

    let mut entries: Vec<(char, u32)> = state.bookmarks.iter().map(|(c, o)| (*c, *o)).collect();
    entries.sort_by_key(|(c, _)| *c);

    let block = Block::default()
        .title("bookmarks  (j/k: move · Enter: jump · Esc: close · m<0-9> set · '<0-9> jump)")
        .borders(Borders::ALL);

    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new("no marks set — press m<0-9> on a message in the list").block(block),
            overlay_area,
        );
        return;
    }

    let inner = block.inner(overlay_area);
    frame.render_widget(block, overlay_area);

    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let header_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    let body_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: inner.height - 1,
    };

    // Header: "mark" prefix + the standard list columns.
    let hstyle = Style::default().add_modifier(Modifier::BOLD);
    let mut header_spans = vec![
        Span::styled(pad_cell("mark", COL_MARK), hstyle),
        Span::raw(COL_SEP),
    ];
    header_spans.extend(header_line().spans);
    frame.render_widget(Paragraph::new(Line::from(header_spans)), header_area);

    // Clamp cursor inside the current entries — the caller may have
    // deleted bookmarks between renders.
    let selected = cursor.min(entries.len() - 1);

    let lines: Vec<Line<'static>> = entries
        .iter()
        .enumerate()
        .map(|(i, (c, ord))| {
            let base = build_line_for_ord(state, *ord);
            let mut spans = vec![
                Span::raw(pad_cell(&c.to_string(), COL_MARK)),
                Span::raw(COL_SEP),
            ];
            spans.extend(base.spans);
            let line = Line::from(spans).style(base.style);
            if i == selected {
                line.patch_style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                line
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), body_area);
}

/// Total width of the data columns (minus the trailing `detail`, which
/// extends). Used for horizontal layout sanity; kept private to avoid
/// making the overlay any more brittle to upstream column tweaks.
#[allow(dead_code)]
fn fixed_columns_width() -> usize {
    COL_MARK + 1 + COL_TIME + 1 + COL_MESSAGE + 1 + COL_CLORDID + 1 + COL_STATUS + 1
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Percentage((100 - percent_y) / 2),
            ratatui::layout::Constraint::Percentage(percent_y),
            ratatui::layout::Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints([
            ratatui::layout::Constraint::Percentage((100 - percent_x) / 2),
            ratatui::layout::Constraint::Percentage(percent_x),
            ratatui::layout::Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
