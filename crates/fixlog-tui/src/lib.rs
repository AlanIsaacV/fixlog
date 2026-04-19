#![deny(unsafe_code)]

//! `fixlog-tui` — interactive terminal frontend for fixlog.
//!
//! Opens a FIX log in an alternate-screen TUI with a virtual message list,
//! a resolved detail panel, vim-like navigation, a live filter reusing
//! [`fixlog_core::query`], and optional `--follow` tailing that appends to
//! the in-memory [`fixlog_core::index::LogIndex`] as the file grows.
//!
//! Downstream code should go through [`run`] with a [`TuiConfig`] rather than
//! touching the internal modules directly — the event loop, terminal
//! setup/teardown, and re-mmap-on-growth invariants live inside `run` and are
//! not part of the public contract.

use std::path::PathBuf;

use anyhow::{Context, Result};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};

pub mod app;
pub mod clipboard;
pub mod command;
pub mod event;
pub mod export;
pub mod follow;
pub mod input;
pub mod io;
pub mod search;
pub mod state;
pub mod summary;
pub mod terminal;
pub mod theme;
pub mod view;

use crate::app::App;
use crate::follow::FollowWatcher;
use crate::terminal::TerminalGuard;

/// Configuration for a single TUI session.
///
/// Constructed by the caller (usually the `fixlog tui` CLI subcommand) and
/// handed to [`run`]. All fields are owned so the config can outlive argv.
#[derive(Debug, Clone)]
pub struct TuiConfig {
    /// Path to the FIX log to open. Read via `memmap2`; must be a regular file.
    pub path: PathBuf,
    /// When true, the TUI keeps watching the file for growth/rotation and
    /// appends new messages to the index in-place.
    pub follow: bool,
    /// Optional filter expression (same grammar as `fixlog grep --filter`),
    /// applied after bootstrap so the initial view is already filtered.
    pub initial_filter: Option<String>,
}

/// Errors surfaced from the TUI event loop.
///
/// Internal callers typically use `anyhow::Result` and `.context(...)` for
/// operational errors; this enum is reserved for failures callers may want to
/// branch on programmatically.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("failed to open log at {path}: {source}")]
    OpenLog {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to initialise the terminal: {0}")]
    Terminal(#[source] std::io::Error),

    #[error("invalid initial filter: {0}")]
    InvalidFilter(String),
}

/// Entry point for the TUI.
///
/// Bootstraps the app state from [`TuiConfig::path`], sets up the
/// alternate-screen terminal via [`TerminalGuard`], and drives the event
/// loop until the user quits with `q` or `Ctrl+C`. The guard's `Drop` impl
/// restores the terminal even if this function returns `Err`.
pub fn run(cfg: TuiConfig) -> Result<()> {
    let mut app = App::bootstrap(&cfg).context("bootstrapping app state")?;
    let mut watcher = cfg.follow.then(|| FollowWatcher::new(cfg.path.clone()));
    let mut guard = TerminalGuard::enter().context("entering terminal")?;

    loop {
        guard
            .terminal_mut()
            .draw(|frame| draw(frame, &mut app))
            .context("drawing frame")?;

        let ev = event::next()?;
        app.on_event(&ev);

        if let Some(w) = watcher.as_mut()
            && let Err(e) = w.poll(&mut app.state)
        {
            tracing::warn!(error = %e, "follow watcher error");
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

/// Per-frame layout.
///
/// ```text
/// +-----------------------------+  title (1 row)
/// | list (60%)   | detail (40%) |  body (rest)
/// +-----------------------------+  status (1 row)
/// | :filter …                   |  command (1 row, only when in Command mode)
/// ```
///
/// When `raw_detail_mode` is on the body collapses to a single full-width
/// detail panel so (a) the wrapped raw FIX bytes get the maximum viewport
/// for readability and (b) a terminal drag-select captures only raw text
/// without bleeding into list rows at the same y-coordinate.
///
/// The command bar only steals a row while `InputMode::Command` is active
/// so normal navigation keeps the full body height.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let bottom_rows = match app.state.input_mode {
        state::InputMode::Command | state::InputMode::Search => 1,
        state::InputMode::Normal => 0,
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),           // title
            Constraint::Min(1),              // body (list + detail)
            Constraint::Length(1),           // status
            Constraint::Length(bottom_rows), // command/search (possibly 0)
        ])
        .split(area);

    draw_title(frame, chunks[0], app);

    if app.state.raw_detail_mode {
        view::detail::render(frame, chunks[1], &mut app.state);
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(chunks[1]);
        view::list::render(frame, body[0], &mut app.state);
        view::detail::render(frame, body[1], &mut app.state);
    }
    view::status::render(frame, chunks[2], &app.state);
    view::command::render(frame, chunks[3], &app.state);
    view::search::render(frame, chunks[3], &app.state);

    // Overlays draw last so they sit on top of the main layout. Each view
    // paints a `Clear` over its centered rect so list/detail underneath
    // don't bleed through.
    if let Some(overlay) = app.state.overlay.clone() {
        match overlay {
            state::Overlay::Sessions { map, cursor } => {
                view::sessions::render(frame, area, &map, cursor);
            }
            state::Overlay::Orders { timeline, cursor } => {
                view::orders::render(frame, area, &timeline, cursor);
            }
            state::Overlay::Diff => {
                if let (Some(a), Some(b)) = (app.state.diff_slots[0], app.state.diff_slots[1]) {
                    view::diff::render(frame, area, &app.state, a, b);
                }
            }
            state::Overlay::Marks { cursor } => {
                view::marks::render(frame, area, &app.state, cursor);
            }
            state::Overlay::Histogram { histogram, width } => {
                view::histogram::render(frame, area, &histogram, width);
            }
            state::Overlay::Help { scroll } => {
                view::help::render(frame, area, scroll);
            }
        }
    }
}

fn draw_title(frame: &mut Frame, area: Rect, app: &App) {
    let total = app.state.index.len();
    let shown = app.state.visible.len();
    let cursor = app.state.cursor.saturating_add(1).min(shown.max(1));
    let title = Line::from(vec![
        Span::styled("fixlog", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::raw(app.state.path.display().to_string()),
        Span::raw(format!(" — {cursor}/{shown} ({total})")),
    ]);
    frame.render_widget(Block::default().title(title).borders(Borders::NONE), area);
}
