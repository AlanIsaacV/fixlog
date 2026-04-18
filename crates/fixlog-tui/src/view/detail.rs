//! Detail panel. Renders the resolved representation of the message under
//! the cursor as a table of `tag | name | type | raw | decoded`. The resolve
//! step allocates per-field — cheap because it only runs when the cursor
//! moves; the cache is keyed on ordinal via [`AppState::refresh_detail_cache`].

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};

use crate::state::{AppState, ResolvedMessageOwned};

/// Session-layer header and trailer tags. Hidden when `state.skip_common`
/// is true so the payload is easier to scan.
const COMMON_TAGS: &[u32] = &[8, 9, 10, 34, 35, 49, 52, 56];

fn is_common(tag: u32) -> bool {
    COMMON_TAGS.contains(&tag)
}

pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 10 || area.height < 2 {
        return;
    }

    state.refresh_detail_cache();

    // Raw mode renders WITHOUT the leading border so the wrapped FIX bytes
    // don't have a vertical bar intersecting terminal-selected lines (which
    // would insert `|` between every wrap boundary in the pasted text).
    // Resolved mode keeps the left border as a visual panel divider.
    let (border_block, inner) = if state.raw_detail_mode {
        let block = Block::default().title(title_line(
            &state.detail_cache,
            state.skip_common,
            state.raw_detail_mode,
        ));
        let inner = block.inner(area);
        (block, inner)
    } else {
        let block = Block::default().borders(Borders::LEFT).title(title_line(
            &state.detail_cache,
            state.skip_common,
            state.raw_detail_mode,
        ));
        let inner = block.inner(area);
        (block, inner)
    };
    frame.render_widget(border_block, area);

    if state.raw_detail_mode {
        render_raw(frame, inner, state);
        return;
    }

    // Clone only what's `Copy` / cheap-owned up front so the match can
    // read `state.detail_cache` immutably and we can still feed scroll
    // state back via `render_fields_result` afterward without fighting
    // the borrow checker.
    let skip_common = state.skip_common;
    let detail_h_offset = state.detail_h_offset;
    let detail_v_offset = state.detail_v_offset;

    let result: Option<RenderFieldsOutcome> = match &state.detail_cache {
        None => {
            frame.render_widget(Paragraph::new("no message selected"), inner);
            None
        }
        Some((_, Err(msg))) => {
            let p = Paragraph::new(Line::from(vec![
                Span::styled(
                    "<parse error> ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw(msg.clone()),
            ]));
            frame.render_widget(p, inner);
            None
        }
        Some((_, Ok(resolved))) => Some(render_fields(
            frame,
            inner,
            resolved,
            skip_common,
            detail_h_offset,
            detail_v_offset,
        )),
    };
    if let Some(out) = result {
        state.detail_v_offset = out.clamped_v_offset;
        state.last_detail_height = out.viewport_rows;
    }
}

/// What `render_fields` needs to feed back to `AppState` after the
/// immutable borrow of `detail_cache` ends: the clamped vertical offset
/// (so `G` → `u16::MAX` settles on a real last row) and the viewport
/// row count (so `Ctrl+D`/`Ctrl+U` half-page steps are sized correctly).
struct RenderFieldsOutcome {
    clamped_v_offset: u16,
    viewport_rows: usize,
}

/// Render the raw FIX bytes of the cached message with `SOH → |` and
/// non-printable → `.`. Wraps at the viewport width via `Paragraph::wrap`
/// so the whole message is visible without horizontal scrolling — the
/// caller is expected to give `render_raw` the full body width (see
/// `lib::draw`) so a terminal drag-select captures only the raw bytes and
/// nothing from the list panel.
fn render_raw(frame: &mut Frame, area: Rect, state: &mut AppState) {
    state.last_detail_height = area.height as usize;

    let Some((ord, _)) = state.detail_cache else {
        frame.render_widget(Paragraph::new("no message selected"), area);
        return;
    };
    let Some(bytes) = state.index.message_bytes(&state.mmap, ord as usize) else {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "<offset out of range> ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("ordinal {ord}")),
            ])),
            area,
        );
        return;
    };
    let text = raw_value(bytes);
    // `trim: false` preserves whitespace inside the raw text; the FIX bytes
    // don't have leading whitespace issues, so either flag behaves the same
    // for us — `false` is the conservative default.
    // Vertical scroll via `.scroll((detail_v_offset, 0))` drops the first N
    // wrapped lines; ratatui's Paragraph recomputes wrapping internally so
    // we don't need to know the post-wrap line count in advance. Over-scroll
    // (caused by `G` which sets u16::MAX) leaves the panel empty — `g` or
    // `k` recovers.
    let p = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((state.detail_v_offset, 0));
    frame.render_widget(p, area);
}

fn title_line(
    cache: &Option<(u32, Result<ResolvedMessageOwned, String>)>,
    skip_common: bool,
    raw_mode: bool,
) -> Line<'static> {
    let mut suffix = String::new();
    if raw_mode {
        suffix.push_str(" · raw");
    }
    if skip_common {
        suffix.push_str(" · common skipped");
    }
    match cache {
        None => Line::from(Span::raw(format!(" detail{suffix} "))),
        Some((_, Err(_))) => Line::from(Span::raw(format!(" detail (error){suffix} "))),
        Some((_, Ok(r))) => Line::from(vec![
            Span::raw(" detail · offset "),
            Span::raw(r.offset.to_string()),
            Span::raw(" · "),
            Span::styled(
                r.msg_type_name.unwrap_or("?").to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(suffix, Style::default().add_modifier(Modifier::DIM)),
            Span::raw(" "),
        ]),
    }
}

fn render_fields(
    frame: &mut Frame,
    area: Rect,
    resolved: &ResolvedMessageOwned,
    skip_common: bool,
    h_offset: u16,
    requested_v_offset: u16,
) -> RenderFieldsOutcome {
    let header = Row::new(vec!["tag", "name", "type", "raw", "decoded"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    // The Table eats one row for its header, so the data viewport is
    // `area.height - 1`.
    let viewport_rows = (area.height as usize).saturating_sub(1);
    let offset = h_offset as usize;

    let all: Vec<&_> = resolved
        .fields
        .iter()
        .filter(|f| !skip_common || !is_common(f.tag))
        .collect();

    // Clamp the requested vertical offset to the real last-page-start so
    // `G` (dispatched as `u16::MAX`) lands on the last row exactly and a
    // subsequent `k` walks back through real content instead of empty
    // rows.
    let max_v = all.len().saturating_sub(viewport_rows);
    let max_v_u16 = u16::try_from(max_v).unwrap_or(u16::MAX);
    let clamped_v_offset = requested_v_offset.min(max_v_u16);
    let v_offset = clamped_v_offset as usize;

    let rows: Vec<Row<'static>> = all
        .iter()
        .skip(v_offset)
        .take(viewport_rows.max(1))
        .map(|f| {
            Row::new(vec![
                Cell::from(f.tag.to_string()),
                Cell::from(f.name.unwrap_or("?")),
                Cell::from(f.field_type.unwrap_or("-")),
                Cell::from(h_scroll_str(&raw_value(&f.value), offset)),
                Cell::from(h_scroll_str(f.value_label.unwrap_or(""), offset)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(6),  // tag
        Constraint::Length(18), // name
        Constraint::Length(9),  // type
        Constraint::Length(20), // raw
        Constraint::Min(6),     // decoded
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, area);

    RenderFieldsOutcome {
        clamped_v_offset,
        viewport_rows,
    }
}

/// Drop the first `offset` characters from `s`. When `offset` exceeds the
/// string's length the result is empty. Called on the `raw` and `decoded`
/// detail columns when `h_offset > 0` so long values can slide into view.
fn h_scroll_str(s: &str, offset: usize) -> String {
    if offset == 0 {
        return s.to_string();
    }
    s.chars().skip(offset).collect()
}

/// ASCII-safe rendering of a value: non-printables become `.`, SOH becomes
/// `|`. Values are usually short (<40 bytes) so the allocation cost is low.
fn raw_value(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            0x01 => out.push('|'),
            c if c.is_ascii_graphic() || c == b' ' => out.push(c as char),
            _ => out.push('.'),
        }
    }
    out
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
    fn raw_value_replaces_soh_and_controls() {
        assert_eq!(raw_value(b"D\x01buy"), "D|buy");
        assert_eq!(raw_value(b"\x00\x1F"), "..");
    }

    #[test]
    fn render_shows_msg_type_name() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 0;
        state.refresh_detail_cache();

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();

        let rendered = terminal.backend().to_string();
        // The header shows `msg_type_name` for the resolved message. fix44-om
        // starts with a Logon (35=A).
        assert!(
            rendered.contains("Logon") || rendered.contains("35"),
            "expected MsgType in header, got: {rendered}"
        );
        assert!(
            rendered.contains("tag"),
            "expected column header: {rendered}"
        );
    }

    #[test]
    fn cache_is_reused_for_same_ordinal() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 5;
        state.refresh_detail_cache();
        let key1 = state.detail_cache.as_ref().unwrap().0;

        // Call again without moving cursor — cache should not rebuild.
        let ptr1 = state
            .detail_cache
            .as_ref()
            .unwrap()
            .1
            .as_ref()
            .ok()
            .map(|r| r as *const _);
        state.refresh_detail_cache();
        let ptr2 = state
            .detail_cache
            .as_ref()
            .unwrap()
            .1
            .as_ref()
            .ok()
            .map(|r| r as *const _);
        assert_eq!(ptr1, ptr2, "cache should not rebuild for same ordinal");
        let key2 = state.detail_cache.as_ref().unwrap().0;
        assert_eq!(key1, key2);
    }

    #[test]
    fn cache_rebuilds_when_cursor_moves() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 0;
        state.refresh_detail_cache();
        let k0 = state.detail_cache.as_ref().unwrap().0;

        state.cursor = 10;
        state.refresh_detail_cache();
        let k1 = state.detail_cache.as_ref().unwrap().0;

        assert_ne!(k0, k1);
    }

    #[test]
    fn skip_common_filters_session_layer_tags() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 0;
        state.skip_common = true;
        state.refresh_detail_cache();

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();
        let rendered = terminal.backend().to_string();

        // Common tag numbers should not appear as a leading cell when
        // skip_common is on. The easiest signal is the title suffix; also
        // check that "BeginString" (tag 8) name is gone.
        assert!(
            rendered.contains("common skipped"),
            "expected title marker: {rendered}"
        );
        assert!(
            !rendered.contains("BeginString"),
            "BeginString should be hidden: {rendered}"
        );
    }

    #[test]
    fn skip_common_off_shows_everything() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 0;
        state.skip_common = false;
        state.refresh_detail_cache();

        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();
        let rendered = terminal.backend().to_string();

        assert!(
            rendered.contains("BeginString"),
            "BeginString should be visible by default: {rendered}"
        );
    }

    #[test]
    fn raw_detail_mode_renders_bytes_with_pipe_separator() {
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 0;
        state.raw_detail_mode = true;
        state.refresh_detail_cache();

        let backend = TestBackend::new(120, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();
        let rendered = terminal.backend().to_string();

        // A raw FIX message renders with `|` instead of SOH, so at minimum
        // we should see `8=FIX.4.4|` somewhere — the BeginString field.
        assert!(
            rendered.contains("8=FIX.4.4|"),
            "raw detail should show `8=FIX.4.4|`: {rendered}"
        );
        assert!(
            rendered.contains("raw"),
            "title should include `· raw`: {rendered}"
        );
    }

    #[test]
    fn raw_detail_mode_wraps_on_narrow_width() {
        // A real FIX message is typically 80–200 bytes. A 40-column
        // viewport must show the full message across multiple rows —
        // Paragraph::wrap does the work.
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 0;
        state.raw_detail_mode = true;
        state.refresh_detail_cache();

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();
        let rendered = terminal.backend().to_string();

        // The message starts with `8=FIX.4.4|` and ends with `10=NNN|`.
        // Wrapping means both must be visible somewhere in the output.
        assert!(
            rendered.contains("8=FIX.4.4|"),
            "BeginString must be visible: {rendered}"
        );
        // The trailer tag 10 (Checksum) is always last; if wrap works it
        // should be visible.
        assert!(
            rendered.contains("10="),
            "trailer Checksum tag must be visible when wrapping: {rendered}"
        );
    }

    #[test]
    fn raw_detail_mode_omits_left_border() {
        // The left border (`│`) would get sucked into terminal drag-select
        // at the start of every wrap boundary, so raw mode drops it.
        let mut state = bootstrap(&fixture("fix44-om.log"), None).expect("bootstrap");
        state.cursor = 0;
        state.raw_detail_mode = true;
        state.refresh_detail_cache();

        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();
        let rendered = terminal.backend().to_string();
        assert!(
            !rendered.contains('│'),
            "raw mode must not render a vertical border (sullies clipboard): {rendered}"
        );
    }

    #[test]
    fn h_scroll_str_skips_leading_chars() {
        assert_eq!(h_scroll_str("hello", 0), "hello");
        assert_eq!(h_scroll_str("hello", 2), "llo");
        assert_eq!(h_scroll_str("hello", 5), "");
        assert_eq!(h_scroll_str("hello", 10), "");
    }

    #[test]
    fn empty_visible_clears_cache() {
        let mut state = bootstrap(&fixture("fix44-om.log"), Some("35=ZZZ")).expect("bootstrap");
        assert!(state.detail_cache.is_none());

        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut state)).unwrap();

        let rendered = terminal.backend().to_string();
        assert!(
            rendered.contains("no message selected"),
            "expected empty placeholder, got: {rendered}"
        );
    }
}
