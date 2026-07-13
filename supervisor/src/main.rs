use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod backoff;
mod client;
mod control;
mod daemon;
mod engine;
mod engine_link;
mod ipc_server;
mod panic_key;
mod schedule;
mod session;
mod tray;
mod wallpaper;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// Diagnostic/manual-testing subcommands: a thin one-shot client against a *running* daemon.
/// Deliberately doesn't auto-spawn one if none is reachable -- that bootstrap logic belongs to
/// the real callers (config app, `lw dev`), so this stays a pure diagnostic tool.
#[derive(Subcommand)]
pub enum Command {
    Status,
    Start {
        #[arg(long)]
        mode_path: Option<PathBuf>,
        #[arg(long)]
        dev: bool,
    },
    Restart {
        mode_path: PathBuf,
        #[arg(long)]
        dev: bool,
    },
    Stop,
    Panic,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => daemon::run(),
        Some(command) => client::run(command),
    }
}
