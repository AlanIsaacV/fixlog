//! Input event loop wrapper.
//!
//! `crossterm::event::poll` blocks with a timeout; we use ~250 ms so the event
//! loop wakes up often enough to process file-growth ticks in `--follow` mode
//! without burning CPU. Anything crossterm emits that we care about is mapped
//! to a compact [`Event`] enum.

use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event as CtEvent, KeyEvent, KeyEventKind};

pub const POLL_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub enum Event {
    /// Keystroke (press only; repeats and releases are dropped).
    Key(KeyEvent),
    /// Terminal window resize.
    Resize(u16, u16),
    /// Poll timeout — used to drive periodic work (watcher heartbeat, status
    /// bar time-based clears, etc.).
    Tick,
}

/// Block up to [`POLL_TIMEOUT`] waiting for the next event. Returns `Tick` on
/// timeout so the loop can do periodic work without special-casing `None`.
pub fn next() -> Result<Event> {
    if event::poll(POLL_TIMEOUT).context("event::poll")? {
        match event::read().context("event::read")? {
            CtEvent::Key(k) if k.kind == KeyEventKind::Press => Ok(Event::Key(k)),
            CtEvent::Key(_) => Ok(Event::Tick),
            CtEvent::Resize(w, h) => Ok(Event::Resize(w, h)),
            _ => Ok(Event::Tick),
        }
    } else {
        Ok(Event::Tick)
    }
}
