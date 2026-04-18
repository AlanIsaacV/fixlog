//! Search bar — renders only when `AppState.input_mode == Search`. Same
//! shape as the command bar but prefixed with `/` for the vim convention.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::state::{AppState, InputMode};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    if area.height == 0 || state.input_mode != InputMode::Search {
        return;
    }
    let line = Line::from(vec![
        Span::styled("/", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(state.search_buffer.clone()),
        Span::styled("▏", Style::default().add_modifier(Modifier::SLOW_BLINK)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
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
    fn renders_slash_prefix_in_search_mode() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.input_mode = InputMode::Search;
        state.search_buffer.push_str("35=D");

        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &state)).unwrap();
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("/35=D"), "got: {rendered}");
    }

    #[test]
    fn hidden_in_normal_mode() {
        let state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &state)).unwrap();
        let rendered = terminal.backend().to_string();
        assert!(!rendered.trim_start().starts_with('/'), "got: {rendered}");
    }
}
