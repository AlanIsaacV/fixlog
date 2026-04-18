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

use fixlog_core::dict::{
    CHAIN_FIX44, DictChain, chain_enum_value_label, chain_for, chain_msg_type_label,
};
use fixlog_core::parser::{
    RawMessage, TAG_BEGIN_STRING, TAG_MSG_TYPE, TAG_SENDING_TIME, parse_one_with_format,
};

use crate::state::AppState;
use crate::theme;

/// Fixed column widths (in visible columns, i.e. `chars().count()`).
/// `DETAIL_MIN` is the minimum; the column can extend past the viewport
/// width when the row content is long, and horizontal scroll reveals the
/// overflow.
const COL_TIME: usize = 9;
const COL_MESSAGE: usize = 22;
const COL_CLORDID: usize = 18;
const COL_STATUS: usize = 24;
/// Space between columns.
const COL_SEP: &str = " ";

/// ClOrdID tag. Not re-exported from `parser` because it's an application-
/// layer tag, not a session-layer one. Declared here to keep the list view
/// self-contained.
const TAG_CL_ORD_ID: u32 = 11;
/// OrderQty tag.
const TAG_ORDER_QTY: u32 = 38;
/// OrdStatus tag (ExecutionReport/OrderCancelReject).
const TAG_ORD_STATUS: u32 = 39;
/// Price tag.
const TAG_PRICE: u32 = 44;
/// Side tag (BUY/SELL/…).
const TAG_SIDE: u32 = 54;
/// Symbol tag.
const TAG_SYMBOL: u32 = 55;
/// ExecType tag (ExecutionReport).
const TAG_EXEC_TYPE: u32 = 150;

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
            let line = build_line(state, i);
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

fn header_line() -> Line<'static> {
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

fn build_line(state: &AppState, i: usize) -> Line<'static> {
    let ord = state.visible[i] as usize;
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
            let mt_raw = lookup_tag_bytes(m, TAG_MSG_TYPE);
            let begin = lookup_tag_bytes(m, TAG_BEGIN_STRING);
            let chain = begin
                .as_deref()
                .map(|b| chain_for(b, None))
                .unwrap_or(CHAIN_FIX44);
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
            let time = lookup_tag_bytes(m, TAG_SENDING_TIME)
                .map(|b| format_time(&b))
                .unwrap_or_default();
            let clordid = lookup_tag_string(m, TAG_CL_ORD_ID).unwrap_or_default();
            let status = status_chips(m, mt_raw.as_deref(), chain);
            let detail = detail_summary(m, chain);
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
fn pad_cell(s: &str, width: usize) -> String {
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

/// Build the DETAIL column from Side (54), OrderQty (38), Symbol (55) and
/// Price (44) when present. Format `"SIDE QTY SYMBOL @ PRICE"` — missing
/// fields are omitted. `Side` resolves through the dictionary when
/// possible (`BUY`, `SELL`, …); numeric quantities get a thousands
/// separator.
fn detail_summary(msg: &RawMessage<'_>, chain: DictChain) -> String {
    let side = lookup_tag_bytes(msg, TAG_SIDE).map(|v| label_or_raw(chain, TAG_SIDE, &v));
    let qty = lookup_tag_bytes(msg, TAG_ORDER_QTY).map(|v| format_number(&v));
    let symbol = lookup_tag_string(msg, TAG_SYMBOL);
    let price = lookup_tag_bytes(msg, TAG_PRICE).map(|v| String::from_utf8_lossy(&v).into_owned());

    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = side {
        parts.push(s);
    }
    if let Some(q) = qty {
        parts.push(q);
    }
    if let Some(s) = symbol {
        parts.push(s);
    }
    let mut out = parts.join(" ");
    if let Some(p) = price {
        if out.is_empty() {
            out = format!("@ {p}");
        } else {
            out = format!("{out} @ {p}");
        }
    }
    out
}

/// Insert thousands separators in an ASCII integer string. Leaves the
/// decimal portion (if any) untouched. Non-digit leading characters
/// (`-`, `+`) are preserved. Falls back to the raw text when bytes
/// aren't valid UTF-8 digits.
fn format_number(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes).into_owned();
    // Split optional decimal.
    let (int_part, frac) = match raw.find('.') {
        Some(i) => (&raw[..i], Some(&raw[i..])),
        None => (raw.as_str(), None),
    };
    // Preserve a leading sign.
    let (sign, digits) = match int_part.strip_prefix('-') {
        Some(r) => ("-", r),
        None => match int_part.strip_prefix('+') {
            Some(r) => ("+", r),
            None => ("", int_part),
        },
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return raw;
    }
    let len = digits.len();
    let mut grouped = String::with_capacity(len + len / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i != 0 && (len - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let mut out =
        String::with_capacity(sign.len() + grouped.len() + frac.map(|s| s.len()).unwrap_or(0));
    out.push_str(sign);
    out.push_str(&grouped);
    if let Some(f) = frac {
        out.push_str(f);
    }
    out
}

/// Status chips for ExecutionReport (35=8): `ExecType · OrdStatus`, each
/// rendered as its dictionary label (falling back to the raw byte value
/// when the label is unknown).
///
/// Other message types get an empty string for now — OrderCancelReject (35=9)
/// also carries `OrdStatus` but is out of scope for this iteration.
fn status_chips(msg: &RawMessage<'_>, msg_type: Option<&[u8]>, chain: DictChain) -> String {
    if msg_type != Some(b"8") {
        return String::new();
    }
    let exec_type =
        lookup_tag_bytes(msg, TAG_EXEC_TYPE).map(|v| label_or_raw(chain, TAG_EXEC_TYPE, &v));
    let ord_status =
        lookup_tag_bytes(msg, TAG_ORD_STATUS).map(|v| label_or_raw(chain, TAG_ORD_STATUS, &v));
    match (exec_type, ord_status) {
        (Some(e), Some(o)) => format!("{e} · {o}"),
        (Some(e), None) => e,
        (None, Some(o)) => o,
        (None, None) => String::new(),
    }
}

fn label_or_raw(chain: DictChain, tag: u32, value: &[u8]) -> String {
    chain_enum_value_label(chain, tag, value)
        .map(|s| s.to_string())
        .unwrap_or_else(|| String::from_utf8_lossy(value).into_owned())
}

fn lookup_tag_bytes(msg: &RawMessage<'_>, tag: u32) -> Option<Vec<u8>> {
    msg.tags
        .iter()
        .find(|(t, _)| *t == tag)
        .map(|(_, v)| v.to_vec())
}

fn lookup_tag_string(msg: &RawMessage<'_>, tag: u32) -> Option<String> {
    msg.tags
        .iter()
        .find(|(t, _)| *t == tag)
        .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
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
    fn format_number_inserts_thousands_separators() {
        assert_eq!(format_number(b"10000"), "10,000");
        assert_eq!(format_number(b"1000000"), "1,000,000");
        assert_eq!(format_number(b"100"), "100");
        assert_eq!(format_number(b"1"), "1");
        assert_eq!(format_number(b"-12345"), "-12,345");
    }

    #[test]
    fn format_number_preserves_decimal_portion() {
        assert_eq!(format_number(b"10000.25"), "10,000.25");
        assert_eq!(format_number(b"1234.5"), "1,234.5");
    }

    #[test]
    fn format_number_passes_through_non_numeric() {
        assert_eq!(format_number(b"abc"), "abc");
        assert_eq!(format_number(b""), "");
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
