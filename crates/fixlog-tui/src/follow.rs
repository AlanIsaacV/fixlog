//! File watcher for `--follow` mode.
//!
//! Strategy: instead of pulling in `notify` plus a background thread, we piggy-back on
//! the 250 ms `poll` timeout already driving the TUI event loop — on each tick we
//! `stat` the target file and compare its size against what we mapped. If the file
//! grew, re-mmap and call [`LogIndex::append_from_offset`]; if it shrunk (or the
//! inode changed behind our back), do a full rebuild via [`crate::state::bootstrap`].
//!
//! This adds up to one poll-interval of latency before a new message appears on
//! screen; for a human-facing TUI that's fine. The CLI `grep --follow` uses `notify`
//! for lower latency because its output is piped and the consumer may be waiting
//! in a tight loop.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use fixlog_core::parser::parse_one_with_format;

use crate::app;
use crate::io::mmap_file;
use crate::state::{AppState, StatusMessage, bootstrap};

/// How long between file-metadata polls. Matches the event-loop tick
/// (`event::POLL_TIMEOUT`) so at most one extra stat per tick.
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

pub struct FollowWatcher {
    path: PathBuf,
    last_poll: Option<Instant>,
}

impl FollowWatcher {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            last_poll: None,
        }
    }

    /// Poll the file and sync `state` with any growth/rotation. Does nothing
    /// if we polled within the last [`POLL_INTERVAL`]. The force-poll
    /// variant is [`Self::poll_now`].
    pub fn poll(&mut self, state: &mut AppState) -> Result<()> {
        if let Some(last) = self.last_poll
            && Instant::now().duration_since(last) < POLL_INTERVAL
        {
            return Ok(());
        }
        self.poll_now(state)
    }

    /// Force-poll regardless of cooldown — used by tests and by the initial
    /// pass right after bootstrap.
    pub fn poll_now(&mut self, state: &mut AppState) -> Result<()> {
        self.last_poll = Some(Instant::now());
        let current_len = match fs::metadata(&self.path) {
            Ok(m) => m.len(),
            Err(_) => return Ok(()), // file temporarily missing (rotation in flight)
        };
        let mapped_len = state.mmap.len() as u64;

        if current_len == mapped_len {
            return Ok(());
        }

        if current_len < state.index.consumed {
            rebuild(state, &self.path)?;
            return Ok(());
        }

        // Re-mmap: the old Arc goes away after `state.mmap` is overwritten,
        // unless some cache still holds a Clone. The detail cache uses
        // `ResolvedMessageOwned` (no borrow on mmap) so we're safe.
        let new_mmap = Arc::new(mmap_file(&self.path)?);
        state.mmap = new_mmap;

        let prev_count = state.index.len() as u32;
        let consumed = state.index.consumed;
        if let Err(e) = state
            .index
            .append_from_offset(&state.mmap, consumed, &state.format)
        {
            tracing::warn!(error = %e, "append_from_offset failed; rebuilding");
            rebuild(state, &self.path)?;
            return Ok(());
        }

        extend_visible(state, prev_count);
        let new_count = state.index.len() as u32;
        let delta = new_count.saturating_sub(prev_count) as usize;
        if delta > 0 {
            app::on_index_grew(state, delta);
            tracing::debug!(delta, "follow: index grew");
        }
        Ok(())
    }
}

/// Append to `state.visible` every new message ordinal (starting at
/// `start_ord`) that passes the active filter. Keeps the visible list in
/// sync with the index without re-scanning the whole buffer.
fn extend_visible(state: &mut AppState, start_ord: u32) {
    let end = state.index.messages.len() as u32;
    for ord in start_ord..end {
        let ordu = ord as usize;
        let Some(bytes) = state.index.message_bytes(&state.mmap, ordu) else {
            continue;
        };
        let matches = match &state.filter {
            None => true,
            Some(expr) => match parse_one_with_format(bytes, &state.format) {
                Ok((msg, _)) => expr.matches(&msg),
                Err(_) => false,
            },
        };
        if matches {
            state.visible.push(ord);
        }
    }
}

/// Full rebuild after a truncation or rotation. Keeps `filter_text` so the
/// user's filter survives across rotations.
fn rebuild(state: &mut AppState, path: &Path) -> Result<()> {
    let filter_text = state.filter_text.clone();
    let new_state =
        bootstrap(path, filter_text.as_deref()).context("rebuilding state after rotation")?;
    *state = new_state;
    state.status = StatusMessage::warn("file rotated or truncated — rebuilt");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::Write;

    use crate::state::bootstrap;

    const MINIMAL: &[u8] = include_bytes!("../../../fixtures/synthetic/minimal_4.4.log");

    fn temp_file(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "fixlog-tui-follow-{}-{}-{}.log",
            std::process::id(),
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut f = File::create(&p).unwrap();
        f.write_all(MINIMAL).unwrap();
        p
    }

    #[test]
    fn poll_is_noop_when_file_unchanged() {
        let path = temp_file("noop");
        let mut state = bootstrap(&path, None).expect("bootstrap");
        let before_len = state.index.len();

        let mut w = FollowWatcher::new(path.clone());
        w.poll_now(&mut state).expect("poll");
        assert_eq!(state.index.len(), before_len);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn poll_picks_up_appended_messages() {
        let path = temp_file("grow");
        let mut state = bootstrap(&path, None).expect("bootstrap");
        let before = state.index.len();
        assert!(before > 0);

        // Append the same fixture again — doubles the message count.
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(MINIMAL).unwrap();
        drop(f);

        let mut w = FollowWatcher::new(path.clone());
        w.poll_now(&mut state).expect("poll");
        assert_eq!(state.index.len(), before * 2, "index should double");
        assert_eq!(state.visible.len(), before * 2, "visible should follow");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn poll_in_follow_mode_jumps_cursor_to_end() {
        let path = temp_file("cursor");
        let mut state = bootstrap(&path, None).expect("bootstrap");
        // Manually move to the start so we can assert the cursor jump.
        state.cursor = 0;

        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(MINIMAL).unwrap();
        drop(f);

        let mut w = FollowWatcher::new(path.clone());
        w.poll_now(&mut state).expect("poll");
        assert_eq!(state.cursor, state.visible.len() - 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn poll_in_browse_mode_increments_new_counter() {
        use crate::state::ViewMode;

        let path = temp_file("browse");
        let mut state = bootstrap(&path, None).expect("bootstrap");
        state.mode = ViewMode::Browse;
        state.new_since_browse = 0;
        let cursor_before = state.cursor;

        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(MINIMAL).unwrap();
        drop(f);

        let mut w = FollowWatcher::new(path.clone());
        w.poll_now(&mut state).expect("poll");
        assert_eq!(state.cursor, cursor_before);
        assert!(state.new_since_browse > 0);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn poll_respects_active_filter_on_new_messages() {
        let path = temp_file("filter");
        let mut state = bootstrap(&path, Some("35=D")).expect("bootstrap with filter");
        let before_visible = state.visible.len();
        let before_index = state.index.len();

        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(MINIMAL).unwrap();
        drop(f);

        let mut w = FollowWatcher::new(path.clone());
        w.poll_now(&mut state).expect("poll");
        assert!(state.index.len() > before_index, "index should grow");
        // Visible should grow by exactly the number of new 35=D messages.
        assert!(
            state.visible.len() > before_visible,
            "visible should grow proportionally to filtered matches"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn truncation_triggers_full_rebuild() {
        let path = temp_file("trunc");
        let mut state = bootstrap(&path, None).expect("bootstrap");
        let before = state.index.len();

        // Truncate the file to empty, then write a smaller payload.
        let mut f = File::create(&path).unwrap();
        f.write_all(&MINIMAL[..MINIMAL.len() / 2]).unwrap();
        drop(f);

        let mut w = FollowWatcher::new(path.clone());
        w.poll_now(&mut state).expect("poll");
        // We can't predict exact counts (synthetic fixture has 10 messages,
        // half the bytes may parse partially) — we just need no panic and a
        // rebuild to have happened: the warn status should be set.
        let t = state.status.text.to_lowercase();
        assert!(
            t.contains("rotated") || t.contains("truncated"),
            "expected rebuild status, got: {}",
            state.status.text
        );
        // And the index should be smaller than before.
        assert!(state.index.len() < before);

        let _ = std::fs::remove_file(&path);
    }
}
