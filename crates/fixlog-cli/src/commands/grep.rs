//! `fixlog grep <file> --filter "<expr>" [--follow]` — post-mortem and live filtering.
//!
//! Two modes:
//! - **Post-mortem** (default): stream-parse the whole file, print matches, exit with a
//!   grep(1)-style code. 0 if anything matched, 1 otherwise.
//! - **Follow** (`--follow`): do a post-mortem pass, then watch the file for growth and
//!   re-parse only the new bytes as they arrive. Runs until SIGINT.
//!
//! The filter is compiled once; the streaming loop allocates nothing per message thanks
//! to zero-copy parsing and the pre-compiled regex inside each `Op::Re` predicate.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fixlog_core::{LogFormat, QueryExpr, parse_all_with_format, parse_query, resolve, sniff};
use memmap2::Mmap;
use notify::{EventKind, RecursiveMode, Watcher, event::ModifyKind};

use crate::ParseFormat;
use crate::commands::parse::{write_jsonl, write_pretty};
use crate::io::{head, mmap_file};

const SNIFF_WINDOW: usize = 64 * 1024;
/// How long we wait on the notify channel before doing a polling fallback `stat`. Catches
/// the case where the OS collapsed events or the watcher missed a write.
const WATCH_POLL_TIMEOUT: Duration = Duration::from_millis(500);

/// Outcome of a `grep` run. Only meaningful in post-mortem mode: follow mode runs until
/// SIGINT and the process terminates without returning.
pub struct GrepOutcome {
    pub matched: usize,
}

pub fn run(path: &Path, filter: &str, format: ParseFormat, follow: bool) -> Result<GrepOutcome> {
    let expr: QueryExpr =
        parse_query(filter).with_context(|| format!("parsing filter: {filter:?}"))?;

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    // Initial pass is the same in both modes: mmap, sniff, stream matches.
    let initial = open_and_sniff(path)?;
    let matched = stream_matches(
        &initial.mmap[..],
        0,
        &expr,
        &initial.log_format,
        format,
        &mut out,
    )?;
    let outcome = GrepOutcome { matched: matched.0 };
    out.flush()?;
    tracing::info!(
        matched = matched.0,
        scanned = matched.1,
        "initial pass done"
    );

    if !follow {
        return Ok(outcome);
    }

    run_follow_loop(
        path,
        &expr,
        &initial.log_format,
        format,
        &mut out,
        matched.0,
    )
}

/// Body of `--follow`: watch the file path and re-scan new bytes on every event.
///
/// Runs until SIGINT. The returned `GrepOutcome` carries the running total of matches so
/// far (including the initial post-mortem pass); in practice the caller ignores it because
/// the process has already been terminated by the signal.
fn run_follow_loop<W: Write>(
    path: &Path,
    expr: &QueryExpr,
    initial_format: &LogFormat,
    format: ParseFormat,
    out: &mut W,
    seed_matched: usize,
) -> Result<GrepOutcome> {
    // Watch the parent directory non-recursively. Parent-level watches catch
    // remove/create events that fire when logrotate swaps the inode; a file-level watch
    // can lose track after rename on some platforms.
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let target = path.to_path_buf();

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .context("initializing file watcher")?;
    watcher
        .watch(&parent, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", parent.display()))?;

    // Follow state holds the current mmap + how far we've parsed. We re-create it from
    // scratch on truncation/rotation.
    let mut state = FollowState::open(&target, initial_format.clone())?;
    // Seed `consumed` so the first post-event catch-up only processes new bytes.
    state.consumed = state.mmap.len() as u64;
    tracing::info!(
        path = %target.display(),
        seed_matched,
        "follow mode active; waiting for file events",
    );

    loop {
        let should_catch_up = match rx.recv_timeout(WATCH_POLL_TIMEOUT) {
            Ok(Ok(event)) => event_is_relevant(&event, &target),
            Ok(Err(err)) => {
                tracing::warn!(error = %err, "notify error (continuing)");
                false
            }
            // Timeout: fall through to a polling `catch_up` so we don't miss writes that
            // the backend coalesced away.
            Err(mpsc::RecvTimeoutError::Timeout) => true,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(anyhow!("file watcher channel disconnected"));
            }
        };

        if should_catch_up {
            let added = state.catch_up(&target, expr, format, out)?;
            if added > 0 {
                tracing::debug!(added, "follow pass emitted matches");
            }
            out.flush()?;
        }
    }
}

/// A file we're tailing: the mmap we last saw and the offset up to which we've evaluated.
struct FollowState {
    mmap: Mmap,
    format: LogFormat,
    consumed: u64,
}

impl FollowState {
    fn open(path: &Path, format: LogFormat) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        // SAFETY: Mmap is read-only; external writers appending to the file may surface
        // inconsistent bytes at the tail, which the parser handles by emitting
        // UnexpectedEof and skipping. Same tradeoff as the post-mortem path.
        let mmap =
            unsafe { Mmap::map(&file) }.with_context(|| format!("mmapping {}", path.display()))?;
        Ok(Self {
            mmap,
            format,
            consumed: 0,
        })
    }

    /// Bring the state up to the current end-of-file, printing any matches discovered
    /// since the last catch-up. Returns the number of new matches.
    fn catch_up<W: Write>(
        &mut self,
        path: &Path,
        expr: &QueryExpr,
        format: ParseFormat,
        out: &mut W,
    ) -> Result<usize> {
        // Stat via the file handle behind the mmap isn't possible (we dropped it) — use
        // metadata on the path. If the path now points at a different inode (logrotate
        // moved the old file away and created a new one) we'll detect it below when the
        // size drops or the mmap is stale.
        let meta = std::fs::metadata(path).ok();
        let current_len = meta.map(|m| m.len()).unwrap_or(0);

        // Truncation/rotation detection. If the new file is smaller than what we've
        // already consumed, something discontinuous happened: rebuild from zero.
        if current_len < self.consumed {
            tracing::warn!(
                previous = self.consumed,
                now = current_len,
                "file shrank; assuming rotation/truncation and restarting"
            );
            *self = FollowState::open(path, self.format.clone())?;
            return self.process_from(0, expr, format, out);
        }

        // Re-mmap only if the on-disk file is larger than what we already mapped.
        if current_len > self.mmap.len() as u64 {
            let file = File::open(path).with_context(|| format!("reopening {}", path.display()))?;
            self.mmap = unsafe { Mmap::map(&file) }
                .with_context(|| format!("remmapping {}", path.display()))?;
        }

        self.process_from(self.consumed, expr, format, out)
    }

    /// Parse `self.mmap[from..]` and emit matches. Returns matches produced.
    fn process_from<W: Write>(
        &mut self,
        from: u64,
        expr: &QueryExpr,
        format: ParseFormat,
        out: &mut W,
    ) -> Result<usize> {
        let (matched, new_consumed) =
            stream_matches(&self.mmap[..], from, expr, &self.format, format, out)?;
        self.consumed = new_consumed;
        Ok(matched)
    }
}

/// True if the notify event concerns the file we're following.
///
/// We accept any path equality *and* any event kind that can plausibly imply file growth
/// or replacement: Modify (write/append), Create (rotation re-creating the path), Remove
/// (we'll then try to reopen on the next iteration and get back bytes or give up).
fn event_is_relevant(event: &notify::Event, target: &Path) -> bool {
    if !event.paths.iter().any(|p| p == target) {
        return false;
    }
    matches!(
        event.kind,
        EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Name(_))
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Create(_)
            | EventKind::Remove(_)
    )
}

/// Initial `open + sniff` pass. Separated so the follow mode can reuse the sniffer
/// result for subsequent re-mmaps: we only sniff once per run because resniffing a
/// growing file would waste time and risk oscillating format detection across reads.
struct InitialOpen {
    mmap: Mmap,
    log_format: LogFormat,
}

fn open_and_sniff(path: &Path) -> Result<InitialOpen> {
    let mmap = mmap_file(path)?;
    let log_format =
        sniff(head(&mmap, SNIFF_WINDOW)).with_context(|| format!("sniffing {}", path.display()))?;
    Ok(InitialOpen { mmap, log_format })
}

/// Evaluate `expr` against every message in `buf[from..]` and write matches to `out`.
///
/// Returns `(matched_count, scanned_count_or_new_consumed_offset)` — the second element is
/// the absolute offset past the last successfully parsed message, i.e. the value the
/// caller should store as its next `from` for follow-mode continuation. For the top-level
/// post-mortem path we also record it as `scanned_count` via logging.
fn stream_matches<W: Write>(
    buf: &[u8],
    from: u64,
    expr: &QueryExpr,
    log_format: &LogFormat,
    out_format: ParseFormat,
    out: &mut W,
) -> Result<(usize, u64)> {
    if from >= buf.len() as u64 {
        return Ok((0, from));
    }
    let tail = &buf[from as usize..];
    let mut matched = 0usize;
    let mut last_end = from;
    for msg in parse_all_with_format(tail, log_format).filter_map(Result::ok) {
        let abs_start = from + msg.offset;
        last_end = abs_start + msg.raw.len() as u64;
        if !expr.matches(&msg) {
            continue;
        }
        matched += 1;
        let resolved = resolve(&msg);
        match out_format {
            ParseFormat::Pretty => write_pretty(out, &resolved, msg.raw.len())?,
            ParseFormat::Json => write_jsonl(out, &resolved, msg.raw.len())?,
        }
    }
    Ok((matched, last_end))
}

#[cfg(test)]
mod tests {
    //! Unit tests for the streaming + follow-state logic. They exercise the pure
    //! byte-level behavior without touching the filesystem or `notify`; the e2e
    //! integration tests in `tests/grep.rs` cover the process-level surface.

    use super::*;
    use fixlog_core::sniff;

    const MINIMAL: &[u8] = include_bytes!("../../../../fixtures/synthetic/minimal_4.4.log");

    fn compile_expr(src: &str) -> QueryExpr {
        parse_query(src).expect("valid filter")
    }

    #[test]
    fn stream_matches_counts_and_advances_consumed() {
        let fmt = sniff(MINIMAL).expect("sniffable");
        let expr = compile_expr("35=D");
        let mut out = Vec::<u8>::new();
        let (matched, consumed) =
            stream_matches(MINIMAL, 0, &expr, &fmt, ParseFormat::Json, &mut out).unwrap();
        assert!(matched > 0, "fixture contains NewOrderSingle");
        // `consumed` should land on the end of the last successfully parsed message.
        // This can be slightly less than `MINIMAL.len()` if the fixture ends with a
        // trailing newline or similar separator — we only need monotonic growth to keep
        // advancing the follow-mode cursor.
        assert!(consumed <= MINIMAL.len() as u64);
        assert!(consumed > 0);
    }

    #[test]
    fn stream_matches_respects_from_offset() {
        let fmt = sniff(MINIMAL).expect("sniffable");
        let expr = compile_expr("35=D");
        // First pass from 0 to get the "full" answer.
        let mut out_full = Vec::<u8>::new();
        let (full_count, full_end) =
            stream_matches(MINIMAL, 0, &expr, &fmt, ParseFormat::Json, &mut out_full).unwrap();
        // Now re-run starting at full_end: must produce zero new matches and return
        // (0, full_end) — this is the invariant the follow loop relies on.
        let mut out_empty = Vec::<u8>::new();
        let (zero, end) = stream_matches(
            MINIMAL,
            full_end,
            &expr,
            &fmt,
            ParseFormat::Json,
            &mut out_empty,
        )
        .unwrap();
        assert_eq!(zero, 0);
        assert_eq!(end, full_end);
        assert!(out_empty.is_empty());
        assert!(full_count > 0);
    }

    #[test]
    fn event_is_relevant_matches_target_file_only() {
        let target = Path::new("/tmp/fixlog-target.log");
        let other = Path::new("/tmp/fixlog-other.log");
        let modify = notify::Event {
            kind: EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            paths: vec![target.to_path_buf()],
            attrs: Default::default(),
        };
        assert!(event_is_relevant(&modify, target));
        let modify_other = notify::Event {
            kind: EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            paths: vec![other.to_path_buf()],
            attrs: Default::default(),
        };
        assert!(!event_is_relevant(&modify_other, target));
        let access = notify::Event {
            kind: EventKind::Access(notify::event::AccessKind::Read),
            paths: vec![target.to_path_buf()],
            attrs: Default::default(),
        };
        assert!(!event_is_relevant(&access, target));
    }
}
