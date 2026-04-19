//! Help overlay — static cheatsheet of keybindings, two-key sequences,
//! command-bar commands, and overlays. Opened by `:help` / `:h`; closed
//! with `Esc`. Content taller than the viewport scrolls with `j`/`k` /
//! `Ctrl+D`/`Ctrl+U` / `g`/`G` (handled in `app::overlay_intercept`).

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

/// Total number of content lines rendered inside the overlay body.
/// Kept in sync with [`lines`] so the scroll-clamp logic in
/// `app::overlay_intercept` knows the upper bound without measuring.
pub fn content_len() -> usize {
    lines().len()
}

pub fn render(frame: &mut Frame, area: Rect, scroll: u16) {
    let overlay_area = centered_rect(80, 80, area);
    frame.render_widget(ratatui::widgets::Clear, overlay_area);

    let body = Paragraph::new(lines())
        .block(
            Block::default()
                .title("help  (j/k scroll · Esc close)")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(body, overlay_area);
}

fn header(text: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        text,
        Style::default().add_modifier(Modifier::BOLD),
    ))
}

fn entry(key: &'static str, desc: &'static str) -> Line<'static> {
    // Two columns separated by spaces; the wrap is disabled for key/desc
    // pairs because most keys are short. A plain string would work too but
    // this gives us consistent column widths.
    let padding = 18usize.saturating_sub(key.len());
    let spaces: String = " ".repeat(padding);
    Line::from(vec![
        Span::raw("  "),
        Span::styled(key, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(spaces),
        Span::raw(desc),
    ])
}

fn blank() -> Line<'static> {
    Line::from("")
}

fn lines() -> Vec<Line<'static>> {
    vec![
        header("MODES"),
        entry(":", "enter command mode"),
        entry("/", "enter search mode"),
        entry(
            "Esc",
            "leave current mode / close overlay / clear pending prefix",
        ),
        blank(),
        header("NAVIGATION (list / detail, focused panel)"),
        entry("j / ↓", "cursor down"),
        entry("k / ↑", "cursor up"),
        entry("g", "jump to top (drops into Browse)"),
        entry("G", "jump to bottom (re-enters Follow)"),
        entry("Ctrl+D / PageDown", "half page down"),
        entry("Ctrl+U / PageUp", "half page up"),
        entry("F", "toggle Follow / Browse mode"),
        entry("Tab / Shift+Tab", "move focus between list and detail"),
        entry("← / →", "horizontal scroll of focused panel"),
        entry("0", "reset list + detail scroll offsets"),
        blank(),
        header("DETAIL PANEL (when focus = Detail)"),
        entry("j / k / g / G", "move per-field cursor inside the message"),
        entry("Ctrl+D / Ctrl+U", "half page of fields"),
        entry("f", "filter from field: AND tag=value"),
        entry("x", "filter from field: AND NOT tag=value"),
        blank(),
        header("SEARCH"),
        entry("/<expr>", "open search buffer (same DSL as :filter)"),
        entry("Enter", "jump to first forward match"),
        entry("n", "next match"),
        entry("N", "previous match"),
        blank(),
        header("YANK TO CLIPBOARD"),
        entry("yy", "raw bytes of message (SOH rendered as |)"),
        entry("yY", "pretty-printed resolved table"),
        blank(),
        header("DIFF (two messages)"),
        entry("dd", "set diff slot A to message under cursor"),
        entry(
            "D",
            "set diff slot B (after dd) and open diff overlay; dD also works",
        ),
        entry(":diff clear", "reset both slots"),
        blank(),
        header("BOOKMARKS"),
        entry("m<0-9>", "set bookmark (0..9) to message under cursor"),
        entry(
            "'<0-9>",
            "jump to bookmark (must be in current filtered view)",
        ),
        entry(":marks", "list all bookmarks in an overlay"),
        blank(),
        header("TOGGLES"),
        entry(
            "c",
            "hide / show common header+trailer tags (8,9,10,34,35,49,52,56) in detail",
        ),
        entry("H", "hide / show Heartbeat messages (composes NOT 35=0)"),
        entry("r", "raw FIX bytes ↔ resolved field table in detail panel"),
        blank(),
        header("ANALYSIS OVERLAYS"),
        entry(
            "O",
            "open order lifecycle overlay for tag 11 of cursor message",
        ),
        entry(
            ":sessions",
            "session map overlay (Enter applies 49=X AND 56=Y filter)",
        ),
        entry(
            ":orders [id]",
            "timeline for a ClOrdID (default: tag 11 of cursor)",
        ),
        entry(
            ":histogram [bucket]",
            "temporal histogram (bucket: 1s, 500ms, 100us, 2m …)",
        ),
        entry(":marks", "bookmark overlay"),
        blank(),
        header("COMMANDS"),
        entry(":q / :quit", "exit the TUI"),
        entry(":h / :help", "show this overlay"),
        entry(
            ":filter <expr>",
            "apply a filter (live preview as you type)",
        ),
        entry(":filter", "clear the active filter"),
        entry(":f <expr>", "shorthand for :filter"),
        entry(
            ":export <fmt> <path>",
            "export visible messages (fmt: csv, json, fix, pretty)",
        ),
        entry(":diff clear", "reset diff slots"),
        entry("↑ / ↓  (in :)", "browse command history"),
        blank(),
        header("EXIT"),
        entry("q / Ctrl+C", "quit without saving state"),
    ]
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
    fn content_len_matches_lines() {
        assert_eq!(content_len(), lines().len());
    }

    #[test]
    fn content_covers_main_shortcut_categories() {
        // Render all lines to plain text so we can assert substrings.
        let text: String = lines()
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect::<Vec<_>>()
            .join(" ");
        for needle in [
            "MODES",
            "NAVIGATION",
            "DETAIL PANEL",
            "SEARCH",
            "YANK",
            "DIFF",
            "BOOKMARKS",
            "TOGGLES",
            "ANALYSIS OVERLAYS",
            "COMMANDS",
            ":filter",
            ":export",
            ":sessions",
            ":histogram",
            ":marks",
            ":diff clear",
            "yy",
            "dd",
            "dD",
            "m<0-9>",
            "'<0-9>",
        ] {
            assert!(
                text.contains(needle),
                "help missing section/entry: {needle}"
            );
        }
    }
}
