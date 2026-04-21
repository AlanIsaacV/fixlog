//! Status bar — one row at the bottom of the screen. Left: format version
//! summary. Center: current filter. Right: cursor/visible/total counters.
//!
//! Transient messages in `AppState.status` temporarily replace the full
//! layout with a single centred message; `StatusMessage::is_active` gates
//! the override so the bar reverts automatically on expiry.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::state::{AppState, Focus, SortKey, StatusLevel, ViewMode};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    if area.height == 0 {
        return;
    }

    if state.status.is_active(Instant::now()) {
        let color = match state.status.level {
            StatusLevel::Info => Color::Cyan,
            StatusLevel::Warn => Color::Yellow,
            StatusLevel::Error => Color::Red,
        };
        let p = Paragraph::new(Line::from(Span::styled(
            state.status.text.clone(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )));
        frame.render_widget(p, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(45),
            Constraint::Percentage(25),
        ])
        .split(area);

    frame.render_widget(Paragraph::new(left_text(state)), chunks[0]);
    frame.render_widget(Paragraph::new(middle_text(state)), chunks[1]);
    frame.render_widget(
        Paragraph::new(right_text(state)).alignment(ratatui::layout::Alignment::Right),
        chunks[2],
    );
}

fn left_text(state: &AppState) -> Line<'static> {
    let mode = match state.mode {
        ViewMode::Follow => "[follow]",
        ViewMode::Browse => "[browse]",
    };
    let mode_color = match state.mode {
        ViewMode::Follow => Color::Green,
        ViewMode::Browse => Color::Yellow,
    };
    // Raw mode hides the list entirely so the nav dispatch always treats
    // the focus as Detail; reflect that in the indicator so the user
    // isn't confused by a stale "focus: list" after pressing `r`.
    let effective_focus = if state.raw_detail_mode {
        Focus::Detail
    } else {
        state.focus
    };
    let (focus_label, focus_color) = match effective_focus {
        Focus::List => ("[list]", Color::Magenta),
        Focus::Detail => ("[detail]", Color::Cyan),
    };
    let mut spans = vec![
        Span::styled(mode, Style::default().fg(mode_color)),
        Span::raw(" "),
        Span::styled(
            focus_label,
            Style::default()
                .fg(focus_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    if state.mode == ViewMode::Browse && state.new_since_browse > 0 {
        spans.push(Span::styled(
            format!("⬇ {} new ", state.new_since_browse),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if state.hide_heartbeat {
        spans.push(Span::styled("[no hb] ", Style::default().fg(Color::Yellow)));
    }
    if state.sort_key != SortKey::Natural {
        spans.push(Span::styled(
            format!("[sort:{}] ", state.sort_key.label()),
            Style::default().fg(Color::Magenta),
        ));
    }
    spans.push(Span::styled(
        "sep:",
        Style::default().add_modifier(Modifier::DIM),
    ));
    spans.push(Span::raw(format!("{:?}", state.format.separator)));
    Line::from(spans)
}

fn middle_text(state: &AppState) -> Line<'static> {
    match &state.filter_text {
        None => Line::from(Span::styled(
            "no filter",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Some(text) => Line::from(vec![
            Span::styled("filter:", Style::default().add_modifier(Modifier::DIM)),
            Span::raw(" "),
            Span::raw(text.clone()),
        ]),
    }
}

fn right_text(state: &AppState) -> Line<'static> {
    let shown = state.visible.len();
    let total = state.index.len();
    let cursor = if shown == 0 {
        0
    } else {
        state.cursor.saturating_add(1).min(shown)
    };
    Line::from(format!("{cursor}/{shown} ({total})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::state::bootstrap;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/real")).join(name)
    }

    #[test]
    fn right_text_shows_cursor_over_shown_over_total() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 41;
        let rendered = format!("{:?}", right_text(&state));
        assert!(rendered.contains("42/5419"), "got: {rendered}");
    }

    #[test]
    fn middle_text_reflects_filter_presence() {
        let state_no = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        assert!(format!("{:?}", middle_text(&state_no)).contains("no filter"));

        let state_yes = bootstrap(&fixture("fix44-om.log"), Some("35=D")).expect("bootstrap");
        assert!(format!("{:?}", middle_text(&state_yes)).contains("35=D"));
    }

    #[test]
    fn render_status_bar_draws_counters() {
        let state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &state)).unwrap();
        let rendered = terminal.backend().to_string();
        assert!(
            rendered.contains("5419"),
            "expected total in render: {rendered}"
        );
    }
}
