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
