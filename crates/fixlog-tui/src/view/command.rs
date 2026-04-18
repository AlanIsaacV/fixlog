//! Command bar — renders only when `AppState.input_mode == Command`. Shows
//! `:` + the current buffer. A trailing block character marks the caret.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::state::{AppState, InputMode};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    if area.height == 0 || state.input_mode != InputMode::Command {
        return;
    }
    let line = Line::from(vec![
        Span::styled(":", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(state.command_buffer.clone()),
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
    fn renders_nothing_in_normal_mode() {
        let state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        assert_eq!(state.input_mode, InputMode::Normal);

        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &state)).unwrap();
        let rendered = terminal.backend().to_string();
        // No colon prefix should appear.
        assert!(!rendered.trim_start().starts_with(':'), "got: {rendered}");
    }

    #[test]
    fn renders_buffer_in_command_mode() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.input_mode = InputMode::Command;
        state.command_buffer.push_str("filter 35=D");

        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &state)).unwrap();
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains(":filter 35=D"), "got: {rendered}");
    }
}
