//! Consolidated-orders overlay — one row per order family with
//! cum-qty, notional, average price, fill count and final status.
//!
//! Mirrors the CLI `orders consolidate` view. `Enter` on a row drops the
//! overlay and opens the existing [`crate::state::Overlay::Orders`] for
//! that row's root ClOrdID (wiring in `app.rs`).
//!
//! All display strings are precomputed once in [`ConsolidatedView::from_rows`]
//! when the overlay is opened; the render path only reads borrowed slices.
//! See [`crate::state::Overlay::Consolidated`] for the `Arc` wrapping that
//! keeps `app.state.overlay.clone()` (in the draw loop) cheap.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};

use fixlog_analysis::orders_consolidated::OrderConsolidated;
use fixlog_core::CHAIN_FIX44;
use fixlog_core::dict::chain_enum_value_label;

use crate::state::{AppState, Overlay};

/// Precomputed display state for the consolidated overlay. Built once in
/// [`Self::from_rows`] and stored behind `Arc` inside
/// [`crate::state::Overlay::Consolidated`] so the per-frame
/// `app.state.overlay.clone()` is a refcount bump instead of a deep copy
/// of the underlying row table.
#[derive(Debug)]
pub struct ConsolidatedView {
    pub rows: Vec<ConsolidatedDisplayRow>,
    pub summary: String,
}

/// One row, formatted for direct rendering. Each cell is `Cow<'static, str>`
/// so static placeholders (`"-"`), `side_label` constants, and dictionary
/// hits don't pay a `String` allocation; only fields that genuinely need
/// formatting (`fmt_int`, `fmt_money`, `fmt_price`, `lossy`) are owned.
#[derive(Debug)]
pub struct ConsolidatedDisplayRow {
    pub root_clordid: Vec<u8>,
    pub status_color: Option<Color>,
    pub family: Cow<'static, str>,
    pub side: &'static str,
    pub symbol: Cow<'static, str>,
    pub order_qty: Cow<'static, str>,
    pub cum_qty: Cow<'static, str>,
    pub notional: Cow<'static, str>,
    pub avg_px: Cow<'static, str>,
    pub status: Cow<'static, str>,
    pub fills: Cow<'static, str>,
}

impl ConsolidatedView {
    /// Consume the raw consolidated rows and produce a render-ready view.
    /// Runs all formatting (`fmt_int`, `fmt_money`, `family_display`,
    /// `status_label`, …) exactly once per row — never per frame.
    pub fn from_rows(rows: Vec<OrderConsolidated>) -> Self {
        let total_notional: f64 = rows.iter().map(|r| r.notional).sum();
        let total_fills: u32 = rows.iter().map(|r| r.fills).sum();
        let summary = format!(
            "orders: {}  fills: {}  notional: {}",
            rows.len(),
            total_fills,
            fmt_money(total_notional),
        );

        let display_rows: Vec<ConsolidatedDisplayRow> = rows
            .into_iter()
            .map(|r| {
                let status_color = color_for_status(r.final_ord_status.as_deref());
                let family = family_display(&r);
                let side = r.side.map(side_label).unwrap_or("-");
                let symbol = r
                    .symbol
                    .as_deref()
                    .map(|s| Cow::Owned(lossy(s)))
                    .unwrap_or(Cow::Borrowed("-"));
                let order_qty = r
                    .order_qty
                    .map(|q| Cow::Owned(fmt_int(q)))
                    .unwrap_or(Cow::Borrowed("-"));
                let cum_qty = Cow::Owned(fmt_int(r.cum_qty));
                let notional = Cow::Owned(fmt_money(r.notional));
                let avg_px = fmt_price(r.avg_px);
                let status = r
                    .final_ord_status
                    .as_deref()
                    .map(status_label)
                    .unwrap_or(Cow::Borrowed("-"));
                let fills = Cow::Owned(r.fills.to_string());
                ConsolidatedDisplayRow {
                    root_clordid: r.root_clordid,
                    status_color,
                    family,
                    side,
                    symbol,
                    order_qty,
                    cum_qty,
                    notional,
                    avg_px,
                    status,
                    fills,
                }
            })
            .collect();

        Self {
            rows: display_rows,
            summary,
        }
    }
}

/// Render the consolidated overlay. Mutably borrows `state` so the sticky
/// `viewport_top` can be re-clamped against the live body height before we
/// build the visible row slice. We never iterate `view.rows` beyond
/// `[viewport_top .. viewport_top+h)` — that's what fixes the mouse-wheel
/// lag on logs with thousands of orders (the previous code allocated one
/// `Row` per order per frame).
pub fn render(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let overlay_area = centered_rect(95, 80, area);
    frame.render_widget(Clear, overlay_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(overlay_area);

    // Body height inside chunks[1]: subtract the two `Borders::ALL` rows
    // and the one header row. Floor at 1 so tiny terminals still render
    // something instead of dividing by zero downstream.
    let viewport_height = (chunks[1].height as usize).saturating_sub(3).max(1);

    // Update `viewport_top` and snapshot the data needed below. We exit
    // early when the overlay variant doesn't match (defence in depth —
    // the caller in `lib.rs` already dispatches here only for
    // Consolidated, but the alternative is a panic we don't want).
    let (view, cursor, vt) = {
        let Some(Overlay::Consolidated {
            view,
            cursor,
            viewport_top,
        }) = state.overlay.as_mut()
        else {
            return;
        };
        *viewport_top =
            clamp_viewport_top(*viewport_top, *cursor, viewport_height, view.rows.len());
        (view.clone(), *cursor, *viewport_top)
    };

    frame.render_widget(
        Paragraph::new(view.summary.as_str()).block(
            Block::default()
                .title("consolidated orders  (j/k: move, Enter: open timeline, Esc: close)")
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    let total = view.rows.len();
    let end = (vt + viewport_height).min(total);
    let visible_slice = &view.rows[vt..end];

    let header = Row::new(vec![
        "ClOrdID", "Side", "Symbol", "OrderQty", "CumQty", "Notional", "AvgPx", "Status", "Fills",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let body: Vec<Row<'_>> = visible_slice
        .iter()
        .map(|r| {
            let style = r
                .status_color
                .map(|c| Style::default().fg(c))
                .unwrap_or_default();
            Row::new(vec![
                Cell::from(r.family.as_ref()),
                Cell::from(r.side),
                Cell::from(r.symbol.as_ref()),
                Cell::from(r.order_qty.as_ref()),
                Cell::from(r.cum_qty.as_ref()),
                Cell::from(r.notional.as_ref()),
                Cell::from(r.avg_px.as_ref()),
                Cell::from(r.status.as_ref()),
                Cell::from(r.fills.as_ref()),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(20), // ClOrdID (family display)
        Constraint::Length(5),  // Side
        Constraint::Length(10), // Symbol
        Constraint::Length(10), // OrderQty
        Constraint::Length(10), // CumQty
        Constraint::Length(16), // Notional
        Constraint::Length(11), // AvgPx
        Constraint::Length(14), // Status
        Constraint::Length(6),  // Fills
    ];

    let table = Table::new(body, widths)
        .header(header)
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .block(Block::default().borders(Borders::ALL));

    let mut ts = TableState::default();
    if !visible_slice.is_empty() {
        // Cursor index is global; ratatui's TableState wants it relative
        // to the rows we actually pass in.
        let rel = cursor
            .saturating_sub(vt)
            .min(visible_slice.len().saturating_sub(1));
        ts.select(Some(rel));
    }
    frame.render_stateful_widget(table, chunks[1], &mut ts);
}

/// Same shape as [`crate::state::AppState::ensure_cursor_visible`]: keep
/// the existing `hint` when the cursor still fits in
/// `[hint, hint+height)`; otherwise scroll the minimum amount needed so
/// the cursor becomes the top or bottom row.
fn clamp_viewport_top(hint: usize, cursor: usize, height: usize, total: usize) -> usize {
    if total <= height {
        return 0;
    }
    let max_top = total - height;
    let candidate = if cursor < hint {
        cursor
    } else if cursor >= hint + height {
        cursor + 1 - height
    } else {
        hint
    };
    candidate.min(max_top)
}

fn family_display(r: &OrderConsolidated) -> Cow<'static, str> {
    if r.family.len() <= 1 {
        return Cow::Owned(lossy(&r.root_clordid));
    }
    let mut fam: Vec<String> = r.family.iter().map(|c| lossy(c)).collect();
    fam.sort();
    Cow::Owned(fam.join("→"))
}

fn status_label(bytes: &[u8]) -> Cow<'static, str> {
    chain_enum_value_label(CHAIN_FIX44, 39, bytes)
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned(String::from_utf8_lossy(bytes).into_owned()))
}

fn side_label(b: u8) -> &'static str {
    match b {
        b'1' => "BUY",
        b'2' => "SELL",
        b'3' => "BUYM",
        b'4' => "SELLS",
        b'5' => "SELLSE",
        b'6' => "SELLSX",
        b'7' => "UNDISC",
        b'8' => "CROSS",
        b'9' => "CROSSS",
        _ => "?",
    }
}

fn color_for_status(s: Option<&[u8]>) -> Option<Color> {
    match s? {
        b"2" => Some(Color::Green),       // Filled
        b"1" => Some(Color::Yellow),      // PartiallyFilled
        b"4" => Some(Color::Red),         // Canceled
        b"8" => Some(Color::Red),         // Rejected
        b"C" => Some(Color::DarkGray),    // Expired
        b"0" | b"A" => Some(Color::Blue), // New / PendingNew
        b"5" | b"E" => Some(Color::Cyan), // Replaced / PendingReplace
        _ => None,
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}

fn fmt_money(n: f64) -> String {
    let int_part = n.trunc() as i64;
    let frac = (n.fract().abs() * 100.0).round() as u64;
    format!("{}.{:02}", fmt_int(int_part.unsigned_abs()), frac)
}

fn fmt_price(n: f64) -> Cow<'static, str> {
    if n == 0.0 {
        Cow::Borrowed("-")
    } else {
        Cow::Owned(format!("{n:.4}"))
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
