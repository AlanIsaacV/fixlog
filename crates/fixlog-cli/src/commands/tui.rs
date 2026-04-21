//! `fixlog tui [file] [--filter EXPR] [--follow] [--sort …]` — launch the
//! interactive terminal frontend.
//!
//! Thin wrapper over [`fixlog_tui::run`]; all the rendering and event-loop
//! logic lives in the `fixlog-tui` crate so the CLI binary stays a façade.
//!
//! When `file` is omitted and stdin is a pipe (not a terminal), the stdin
//! stream is drained into a temporary file and the TUI mmaps that file.
//! FD 0 is then redirected to `/dev/tty` so crossterm's raw-mode keyboard
//! reader stays wired to the user, not to the (already consumed) pipe.

use std::io::{self, IsTerminal};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use tempfile::NamedTempFile;

use fixlog_tui::TuiConfig;
use fixlog_tui::state::SortKey;

pub fn run(
    file: Option<&Path>,
    filter: Option<String>,
    follow: bool,
    sort_key: SortKey,
) -> Result<()> {
    // `_tmp` keeps the temp file alive for the lifetime of the TUI; its
    // `Drop` deletes the backing file once the user quits. The TUI mmaps
    // the path, so the file must outlive the session.
    let (path, _tmp) = match file {
        Some(p) => (p.to_path_buf(), None),
        None => {
            let tmp = drain_stdin_to_tmpfile(follow)?;
            (tmp.path().to_path_buf(), Some(tmp))
        }
    };

    fixlog_tui::run(TuiConfig {
        path,
        follow,
        initial_filter: filter,
        sort_key,
    })
}

/// Drain stdin into a temp file and redirect fd 0 to `/dev/tty` so the TUI
/// can read keyboard events after stdin is consumed. Errors early if stdin
/// is a terminal (no piped input) or if `--follow` is requested (pipes are
/// one-shot; there's nothing to watch).
fn drain_stdin_to_tmpfile(follow: bool) -> Result<NamedTempFile> {
    if io::stdin().is_terminal() {
        return Err(anyhow!(
            "no input file and stdin is a terminal\n\
             hint: `fixlog tui <file>` or pipe data in (e.g. `rg ... | fixlog tui`)"
        ));
    }
    if follow {
        return Err(anyhow!(
            "--follow is not supported when reading from stdin (pipes don't grow)"
        ));
    }
    let mut tmp = NamedTempFile::new().context("creating temp file for stdin input")?;
    let written =
        io::copy(&mut io::stdin().lock(), &mut tmp).context("draining stdin to temp file")?;
    if written == 0 {
        return Err(anyhow!("stdin is empty"));
    }
    tmp.as_file_mut()
        .sync_all()
        .context("flushing temp file to disk")?;
    redirect_stdin_to_tty().context("redirecting stdin to /dev/tty")?;
    Ok(tmp)
}

/// Replace fd 0 (stdin) with a handle on `/dev/tty` so crossterm's raw-mode
/// reader still gets keystrokes after we consumed the piped input. Unix
/// only — fixlog already targets darwin/linux.
#[cfg(unix)]
fn redirect_stdin_to_tty() -> Result<()> {
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;

    let tty = OpenOptions::new()
        .read(true)
        .write(false)
        .open("/dev/tty")
        .context("opening /dev/tty (no controlling terminal?)")?;

    // SAFETY: `libc::dup2` takes two raw fds: the source (`tty`, just
    // opened, guaranteed valid) and the target (STDIN_FILENO = 0, always
    // valid). It atomically closes fd 0 if open, then duplicates `tty` onto
    // fd 0. On error it returns -1 and sets errno; we surface that via
    // `std::io::Error::last_os_error`. No Rust invariants are violated: no
    // references or owned resources straddle the call. `tty` is dropped
    // after the call, closing its original fd — fd 0 remains a valid
    // duplicate of the tty until the process exits.
    let rc = unsafe { libc::dup2(tty.as_raw_fd(), libc::STDIN_FILENO) };
    if rc == -1 {
        let err = std::io::Error::last_os_error();
        return Err(err).context("dup2(tty, stdin)");
    }
    Ok(())
}

#[cfg(not(unix))]
fn redirect_stdin_to_tty() -> Result<()> {
    Err(anyhow!(
        "reading from stdin requires /dev/tty (Unix only); pass a file path instead"
    ))
}
