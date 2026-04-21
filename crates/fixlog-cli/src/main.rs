use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

mod commands;
mod io;

/// `fixlog` — inspect FIX log files.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Increase log verbosity (`-v` info, `-vv` debug).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Detect the layout (separator, line prefix, line ending) of a log file.
    Sniff {
        /// Path to the log file.
        file: PathBuf,
    },
    /// Parse messages from a log file and print them.
    Parse {
        /// Path to the log file.
        file: PathBuf,
        /// Stop after the first `N` messages.
        #[arg(long)]
        first: Option<usize>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = ParseFormat::Pretty)]
        format: ParseFormat,
    },
    /// Print summary statistics for a log file (placeholder; full implementation in T13).
    Stats {
        /// Path to the log file.
        file: PathBuf,
    },
    /// Filter messages with a simple DSL, grep-style.
    ///
    /// Examples:
    ///   fixlog grep file.log --filter "35=D"
    ///   fixlog grep file.log --filter "35=8 AND 55=AAPL"
    ///   fixlog grep file.log --filter "55~^MS" --format json
    ///   fixlog grep live.log --filter "35=3" --follow  # tail -f style
    Grep {
        /// Path to the log file.
        file: PathBuf,
        /// Filter expression (see crate fixlog-query for the grammar).
        #[arg(long)]
        filter: String,
        /// Output format.
        #[arg(long, value_enum, default_value_t = ParseFormat::Pretty)]
        format: ParseFormat,
        /// Stream new messages as the file grows, like `tail -f`. Terminates on Ctrl+C.
        #[arg(short = 'F', long)]
        follow: bool,
    },
    /// Open the log in an interactive TUI (ratatui + crossterm).
    ///
    /// Examples:
    ///   fixlog tui file.log
    ///   fixlog tui file.log --filter "35=D"
    ///   fixlog tui live.log --follow
    ///   fixlog tui file.log --sort seq            # list by MsgSeqNum (34)
    ///   rg "35=D" logs/*.log | fixlog tui         # pipe input via stdin
    Tui {
        /// Path to the log file. Omit to read from stdin (pipe).
        file: Option<PathBuf>,
        /// Optional initial filter (same grammar as `grep --filter`).
        #[arg(long)]
        filter: Option<String>,
        /// Watch the file for growth/rotation and append new messages live.
        /// Not available with stdin input.
        #[arg(short = 'F', long)]
        follow: bool,
        /// Initial sort criterion for the list (toggled at runtime with
        /// `o`). `natural` keeps file order; useful when resend requests
        /// create duplicate SendingTimes and you want to see messages in
        /// their generated order (34 / 60).
        #[arg(long, value_enum, default_value_t = SortArg::Natural)]
        sort: SortArg,
    },
    /// Aggregate messages by `(SenderCompID, TargetCompID)` session pair.
    ///
    /// Examples:
    ///   fixlog sessions file.log
    ///   fixlog sessions file.log --format json | jq .
    Sessions {
        /// Path to the log file.
        file: PathBuf,
        /// Output format.
        #[arg(long, value_enum, default_value_t = ParseFormat::Pretty)]
        format: ParseFormat,
    },
    /// Reconstruct order lifecycles by ClOrdID.
    ///
    /// Without `--id`: list the N ClOrdIDs with the most events.
    /// With `--id`: print the full timeline + Gantt row.
    Orders {
        /// Path to the log file.
        file: PathBuf,
        /// ClOrdID (tag 11) to reconstruct. If omitted, list top-N by event count.
        #[arg(long)]
        id: Option<String>,
        /// Number of ClOrdIDs to list when `--id` is absent.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output format.
        #[arg(long, value_enum, default_value_t = ParseFormat::Pretty)]
        format: ParseFormat,
    },
    /// Temporal histogram of SendingTime (tag 52).
    ///
    /// Examples:
    ///   fixlog histogram file.log
    ///   fixlog histogram file.log --bucket 500ms --width 120 --peaks 3
    Histogram {
        /// Path to the log file.
        file: PathBuf,
        /// Bucket width: `<N>ms`, `<N>s`, or `<N>m`.
        #[arg(long, default_value = "1s")]
        bucket: String,
        /// Sparkline width in columns.
        #[arg(long, default_value_t = 80)]
        width: usize,
        /// Number of top-k peaks to highlight below the sparkline.
        #[arg(long, default_value_t = 5)]
        peaks: usize,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ParseFormat {
    Pretty,
    Json,
}

#[derive(Clone, Copy, ValueEnum)]
enum SortArg {
    /// File order — the order messages were written.
    Natural,
    /// Tag 34 MsgSeqNum, numerically.
    Seq,
    /// Tag 60 TransactTime, chronologically.
    Transact,
    /// Tag 52 SendingTime, chronologically.
    Sending,
}

impl From<SortArg> for fixlog_tui::state::SortKey {
    fn from(s: SortArg) -> Self {
        use fixlog_tui::state::SortKey;
        match s {
            SortArg::Natural => SortKey::Natural,
            SortArg::Seq => SortKey::MsgSeqNum,
            SortArg::Transact => SortKey::TransactTime,
            SortArg::Sending => SortKey::SendingTime,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    match cli.command {
        Command::Sniff { file } => commands::sniff::run(&file),
        Command::Parse {
            file,
            first,
            format,
        } => commands::parse::run(&file, first, format),
        Command::Stats { file } => commands::stats::run(&file),
        Command::Grep {
            file,
            filter,
            format,
            follow,
        } => {
            let outcome = commands::grep::run(&file, &filter, format, follow)?;
            // grep(1) exit convention: 0 if anything matched, 1 if nothing did. In follow
            // mode the loop never returns normally (the process dies on SIGINT), so this
            // path only runs when `--follow` wasn't passed.
            if outcome.matched == 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Tui {
            file,
            filter,
            follow,
            sort,
        } => commands::tui::run(file.as_deref(), filter, follow, sort.into()),
        Command::Sessions { file, format } => commands::sessions::run(&file, format),
        Command::Orders {
            file,
            id,
            limit,
            format,
        } => commands::orders::run(&file, id.as_deref(), limit, format),
        Command::Histogram {
            file,
            bucket,
            width,
            peaks,
        } => commands::histogram::run(&file, &bucket, width, peaks),
    }
}

fn init_tracing(verbose: u8) {
    use tracing_subscriber::{EnvFilter, fmt};
    let level = match verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
