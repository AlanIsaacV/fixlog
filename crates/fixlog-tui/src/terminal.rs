//! Terminal setup/teardown with a RAII guard.
//!
//! `TerminalGuard::enter()` claims the alternate screen and enables raw mode.
//! Its `Drop` impl restores the terminal — so even if the event loop panics or
//! returns `Err`, the user's shell is left in a usable state.
//!
//! A one-shot panic hook is installed on first `enter()` that calls `leave()`
//! before delegating to the previous hook; this catches panics from anywhere
//! in the process, not just inside the loop.

use std::io::{Stdout, stdout};
use std::sync::Once;

use anyhow::{Context, Result};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

static PANIC_HOOK: Once = Once::new();

/// RAII wrapper around the crossterm alternate-screen state. Restores the
/// terminal on drop so the caller never has to remember a manual `leave()`.
pub struct TerminalGuard {
    terminal: Option<Tui>,
}

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        install_panic_hook();
        enable_raw_mode().context("enable_raw_mode")?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen).context("EnterAlternateScreen")?;
        let backend = CrosstermBackend::new(out);
        let terminal = Terminal::new(backend).context("Terminal::new")?;
        Ok(Self {
            terminal: Some(terminal),
        })
    }

    pub fn terminal_mut(&mut self) -> &mut Tui {
        self.terminal
            .as_mut()
            .expect("TerminalGuard used after drop")
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = leave();
    }
}

/// Restore the terminal without needing a [`TerminalGuard`]. Idempotent — safe
/// to call from the panic hook even if raw mode was never enabled.
pub fn leave() -> Result<()> {
    let mut out = stdout();
    let _ = execute!(out, LeaveAlternateScreen);
    let _ = disable_raw_mode();
    Ok(())
}

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = leave();
            previous(info);
        }));
    });
}
