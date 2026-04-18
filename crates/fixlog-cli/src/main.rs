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
}

#[derive(Clone, Copy, ValueEnum)]
enum ParseFormat {
    Pretty,
    Json,
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
