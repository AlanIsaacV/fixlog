//! `fixlog tui <file> [--filter EXPR] [--follow]` — launch the interactive
//! terminal frontend.
//!
//! Thin wrapper over [`fixlog_tui::run`]; all the rendering and event-loop
//! logic lives in the `fixlog-tui` crate so the CLI binary stays a façade.

use std::path::Path;

use anyhow::Result;

use fixlog_tui::TuiConfig;

pub fn run(file: &Path, filter: Option<String>, follow: bool) -> Result<()> {
    fixlog_tui::run(TuiConfig {
        path: file.to_path_buf(),
        follow,
        initial_filter: filter,
    })
}
