//! Rendering modules. Each submodule owns one visual region and exposes a
//! single `render(frame, area, ...)` function. View code must not mutate
//! state beyond per-frame bookkeeping like `viewport_top` (driven by
//! cursor visibility). Anything load-bearing happens in `input.rs`/`state.rs`.

pub mod command;
pub mod consolidated;
pub mod detail;
pub mod diff;
pub mod help;
pub mod histogram;
pub mod list;
pub mod marks;
pub mod orders;
pub mod search;
pub mod sessions;
pub mod status;
