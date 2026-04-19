//! Virtual message list. Renders only the rows currently inside the
//! viewport; each row lazily re-parses its message from the mmap so the
//! primary index stays content-free (offset-only).
//!
//! # Layout
//!
//! Four semantic columns, mirroring fixparser.targetcompid.com:
//! `TIME | MESSAGE | CLIENT ORDER ID | DETAIL`.
//!
//! # Performance contract
//!
//! For a viewport of height `H`, this function does `H` calls to `parse_one`
//! plus a handful of small String allocations per row. The mmap bytes are
//! never copied wholesale — only the tags we need (MsgType, BeginString,
//! SendingTime, ClOrdID).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use fixlog_core::dict::{CHAIN_FIX44, chain_for, chain_msg_type_label};
use fixlog_core::parser::{
    TAG_BEGIN_STRING, TAG_MSG_TYPE, TAG_SENDING_TIME, parse_one_with_format,
};

use crate::state::AppState;
use crate::summary::{self, lookup_tag};
use crate::theme;

/// Fixed column widths (in visible columns, i.e. `chars().count()`).
/// `DETAIL_MIN` is the minimum; the column can extend past the viewport
/// width when the row content is long, and horizontal scroll reveals the
/// overflow.
///
/// Exposed to `view::marks` so the bookmarks overlay renders the same
/// semantic columns as the main list.
pub(crate) const COL_TIME: usize = 9;
pub(crate) const COL_MESSAGE: usize = 22;
pub(crate) const COL_CLORDID: usize = 18;
pub(crate) const COL_STATUS: usize = 24;
/// Space between columns.
pub(crate) const COL_SEP: &str = " ";

pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.height < 2 || area.width < 10 {
        return;
    }

    // Reserve one row for the header.
    let content_height = (area.height as usize).saturating_sub(1);
    state.last_list_height = content_height;
    state.ensure_cursor_visible(content_height);

    if state.visible.is_empty() {
        frame.render_widget(Paragraph::new("no messages match"), area);
        return;
    }

    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let body_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height - 1,
    };

    let header_line = header_line();
    let header = Paragraph::new(header_line).scroll((0, state.list_h_offset));
    frame.render_widget(header, header_area);

    let end = (state.viewport_top + content_height).min(state.visible.len());
    let lines: Vec<Line<'static>> = (state.viewport_top..end)
        .map(|i| {
            let line = build_line_for_ord(state, state.visible[i]);
            if i == state.cursor {
                line.patch_style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                line
            }
        })
        .collect();

    let body = Paragraph::new(lines).scroll((0, state.list_h_offset));
    frame.render_widget(body, body_area);
}

pub(crate) fn header_line() -> Line<'static> {
    let style = Style::default().add_modifier(Modifier::BOLD);
    Line::from(vec![
        Span::styled(pad_cell("time", COL_TIME), style),
        Span::raw(COL_SEP),
        Span::styled(pad_cell("message", COL_MESSAGE), style),
        Span::raw(COL_SEP),
        Span::styled(pad_cell("client order id", COL_CLORDID), style),
        Span::raw(COL_SEP),
        Span::styled(pad_cell("status", COL_STATUS), style),
        Span::raw(COL_SEP),
        Span::styled("detail", style),
    ])
}

/// Render a single row by `visible`-index (used by the main list).
///
/// `i` is an index into `state.visible`; see
/// [`build_line_for_ord`] for the ordinal-driven variant used by the
/// bookmarks overlay where filtered-out ordinals must still render.
pub(crate) fn build_line_for_ord(state: &AppState, ord_u32: u32) -> Line<'static> {
    let ord = ord_u32 as usize;
    if state.index.messages.get(ord).is_none() {
        return Line::from(vec![
            Span::raw(pad_cell("-", COL_TIME)),
            Span::raw(COL_SEP),
            Span::raw(pad_cell("?", COL_MESSAGE)),
            Span::raw(COL_SEP),
            Span::raw(pad_cell("", COL_CLORDID)),
            Span::raw(COL_SEP),
            Span::raw(pad_cell("", COL_STATUS)),
            Span::raw(COL_SEP),
            Span::raw("<index out of range>"),
        ]);
    }

    let bytes = state.index.message_bytes(&state.mmap, ord);
    let parsed = bytes.and_then(|b| parse_one_with_format(b, &state.format).ok().map(|(m, _)| m));

    let (time, message, clordid, status, detail, msg_type_raw) = match parsed.as_ref() {
        Some(m) => {
            let mt_raw = lookup_tag(m, TAG_MSG_TYPE).map(|v| v.to_vec());
            let begin = lookup_tag(m, TAG_BEGIN_STRING);
            let chain = begin.map(|b| chain_for(b, None)).unwrap_or(CHAIN_FIX44);
            let message = mt_raw
                .as_deref()
                .and_then(|v| chain_msg_type_label(chain, v))
                .map(|s| s.to_string())
                .or_else(|| {
                    mt_raw
                        .as_deref()
                        .map(|v| String::from_utf8_lossy(v).into_owned())
                })
                .unwrap_or_else(|| "?".into());
            let time = lookup_tag(m, TAG_SENDING_TIME)
                .map(format_time)
                .unwrap_or_default();

            let s = summary::summarize(m, chain);
            let clordid = s.client_order_id.unwrap_or_default();
            let status = s
                .badges
                .iter()
                .map(|b| b.as_ref())
                .collect::<Vec<_>>()
                .join(" · ");
            let detail = s.detail.unwrap_or_default();
            (time, message, clordid, status, detail, mt_raw)
        }
        None => (
            String::new(),
            "?".into(),
            String::new(),
            String::new(),
            String::new(),
            None,
        ),
    };

    let row_style = msg_type_raw
        .as_deref()
        .and_then(theme::color_for_msg_type)
        .map(|c| Style::default().fg(c))
        .unwrap_or_default();

    Line::from(vec![
        Span::raw(pad_cell(&time, COL_TIME)),
        Span::raw(COL_SEP),
        Span::raw(pad_cell(&message, COL_MESSAGE)),
        Span::raw(COL_SEP),
        Span::raw(pad_cell(&clordid, COL_CLORDID)),
        Span::raw(COL_SEP),
        Span::raw(pad_cell(&status, COL_STATUS)),
        Span::raw(COL_SEP),
        Span::raw(detail),
    ])
    .style(row_style)
}

/// Pad (or truncate) `s` to exactly `width` display columns, counted by
/// `chars()`. Truncation prefers keeping the prefix.
pub(crate) fn pad_cell(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count == width {
        s.to_string()
    } else if count > width {
        s.chars().take(width).collect()
    } else {
        let mut out = String::with_capacity(s.len() + (width - count));
        out.push_str(s);
        for _ in 0..(width - count) {
            out.push(' ');
        }
        out
    }
}

/// Extract `HH:MM:SS` from a `SendingTime` (tag 52) value formatted as
/// `YYYYMMDD-HH:MM:SS[.sss]`. Returns the input unchanged if the shape is
/// not recognised — the list stays readable even on exotic timestamps.
fn format_time(value: &[u8]) -> String {
    if let Some(dash) = value.iter().position(|&b| b == b'-')
        && dash + 9 <= value.len()
    {
        let time = &value[dash + 1..dash + 9];
        if time.iter().all(|b| b.is_ascii()) {
            return String::from_utf8_lossy(time).into_owned();
        }
    }
    String::from_utf8_lossy(value).into_owned()
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
    fn format_time_extracts_hhmmss_from_utc_timestamp() {
        assert_eq!(format_time(b"20121105-23:25:12"), "23:25:12");
        assert_eq!(format_time(b"20121105-23:25:12.345"), "23:25:12");
    }

    #[test]
    fn format_time_falls_back_for_unknown_shape() {
        assert_eq!(format_time(b"12:34:56"), "12:34:56");
        assert_eq!(format_time(b"weird"), "weird");
    }

    #[test]
    fn render_shows_semantic_columns_from_fix44_om() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        // Put cursor at the first message so viewport_top is stable at 0.
        state.cursor = 0;
        state.viewport_top = 0;

        let backend = TestBackend::new(140, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();

        let rendered = terminal.backend().to_string();
        assert!(
            rendered.contains("time"),
            "should show time header: {rendered}"
        );
        assert!(
            rendered.contains("message"),
            "should show message header: {rendered}"
        );
        assert!(
            rendered.contains("status"),
            "should show status header: {rendered}"
        );
        // fix44-om.log starts with a Logon (35=A); the dict resolves the
        // msg type label — look for a substring of "Logon".
        assert!(
            rendered.contains("Logon")
                || rendered.contains("Heartbeat")
                || rendered.contains("NewOrderSingle"),
            "should resolve some MsgType label: {rendered}"
        );
    }

    #[test]
    fn render_shows_side_qty_symbol_price_in_detail_column() {
        // A NewOrderSingle (35=D) in fix44-om should populate 54/38/55/44.
        let mut state = bootstrap(&fixture("fix44-om.log"), Some("35=D")).expect("bootstrap");
        assert!(!state.visible.is_empty(), "fixture must contain 35=D");
        state.cursor = 0;
        state.viewport_top = 0;

        let backend = TestBackend::new(160, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();

        let rendered = terminal.backend().to_string();
        // At least one of BUY/SELL must appear in the detail column; the
        // fixture's orders carry tag 54.
        assert!(
            rendered.contains("BUY") || rendered.contains("SELL"),
            "expected Side label (BUY/SELL) in detail: {rendered}"
        );
    }

    #[test]
    fn render_shows_exec_type_and_ord_status_for_execution_report() {
        // Navigate to an ExecutionReport. fix44-om.log starts with Logon then
        // has various messages; we scan the visible list for the first 35=8
        // ordinal and set the cursor there.
        let mut state = bootstrap(&fixture("fix44-om.log"), Some("35=8")).expect("bootstrap");
        assert!(!state.visible.is_empty(), "fixture must contain 35=8");
        state.cursor = 0;
        state.viewport_top = 0;

        let backend = TestBackend::new(140, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();

        let rendered = terminal.backend().to_string();
        // FIX 4.4 dictionary: ExecType 150 enum values include "NEW", "FILL",
        // "PARTIAL_FILL", etc; OrdStatus 39 includes "NEW", "FILLED", etc.
        // We assert at least one recognizable label shows up.
        let has_status_chip = [
            "NEW",
            "FILL",
            "FILLED",
            "PARTIAL_FILL",
            "CANCELED",
            "REJECTED",
        ]
        .iter()
        .any(|s| rendered.contains(s));
        assert!(
            has_status_chip,
            "expected ExecType/OrdStatus label in 35=8 row: {rendered}"
        );
    }

    #[test]
    fn render_survives_tiny_viewport() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        let backend = TestBackend::new(5, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();
    }

    #[test]
    fn render_empty_visible_shows_placeholder() {
        let mut state =
            bootstrap(&fixture("fix44-om.log"), Some("35=ZZZ")).expect("bootstrap with filter");
        assert!(state.visible.is_empty(), "filter should match nothing");

        let backend = TestBackend::new(60, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();
        let rendered = terminal.backend().to_string();
        assert!(
            rendered.contains("no messages"),
            "should show placeholder: {rendered}"
        );
    }

    #[test]
    fn list_h_offset_slides_row_content_left() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 0;
        state.viewport_top = 0;

        let backend = TestBackend::new(120, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();
        let baseline = terminal.backend().to_string();
        assert!(
            baseline.contains("time") && baseline.contains("message"),
            "baseline should show full header: {baseline}"
        );

        // Scroll right past the entire header row (len ≈ 80+). The "time"
        // label lives in the first 4 columns, so a large scroll guarantees
        // it slides off the left edge.
        state.list_h_offset = 40;
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();
        let scrolled = terminal.backend().to_string();
        assert!(
            !scrolled.contains("time") && !scrolled.contains("message"),
            "after list_h_offset=40 header labels should be scrolled off: {scrolled}"
        );
    }

    #[test]
    fn detail_h_offset_does_not_affect_list_render() {
        // Cross-check: bumping detail_h_offset must leave the list rendering
        // identical to the zero-offset baseline.
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 0;
        state.viewport_top = 0;

        let backend = TestBackend::new(120, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();
        let baseline = terminal.backend().to_string();

        state.detail_h_offset = 40;
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();
        let after = terminal.backend().to_string();

        assert_eq!(baseline, after, "detail_h_offset must not touch the list");
    }

    #[test]
    fn render_scrolls_to_keep_cursor_visible() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 1000;
        state.viewport_top = 0;

        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();

        // viewport_height = 19 (20 - 1 header), so viewport_top should be
        // 1000 + 1 - 19 = 982 to keep cursor 1000 as the last row.
        assert_eq!(state.viewport_top, 982);
    }
}
