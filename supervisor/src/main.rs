use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod backoff;
mod client;
mod control;
mod daemon;
mod engine;
mod engine_link;
mod ipc_server;
mod presence;
mod residuals;
mod schedule;
#[cfg(test)]
mod schedule_sim;
mod session;
mod shutdown;
mod state;
mod stop_key;
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
///
/// `DiagnoseSchedule` is the one exception and is handled in [`main`] rather than `client`: it
/// reads a file this machine wrote and needs no daemon at all.
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
        #[arg(long)]
        mode_path: Option<PathBuf>,
        #[arg(long)]
        dev: bool,
    },
    Stop,
    StopImmediately,
    /// Not a client command: reads the residual log this machine wrote and reports whether the
    /// rate model matches what actually happened. See `residuals.rs`.
    DiagnoseSchedule {
        /// Defaults to `schedule-residuals.jsonl` beside the schedule state.
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => daemon::run(),
        Some(Command::DiagnoseSchedule { path }) => diagnose_schedule(path),
        Some(command) => client::run(command),
    }
}

/// Local file analysis, deliberately not routed through `client::run`: it says nothing about a
/// running daemon and must work when none is.
fn diagnose_schedule(path: Option<PathBuf>) -> anyhow::Result<()> {
    let path = path
        .or_else(|| state::state_path().map(|p| p.with_file_name("schedule-residuals.jsonl")))
        .ok_or_else(|| anyhow::anyhow!("no state directory; pass --path"))?;

    if !path.exists() {
        println!(
            "no residual log at {}\nrun the supervisor with {}=1 set and let the schedule fire.",
            path.display(),
            residuals::ENABLE_ENV
        );
        return Ok(());
    }

    let records = residuals::read(&path)?;
    println!("{}\n", path.display());
    print!("{}", residuals::describe(&records));
    print!("{}", residuals::describe_by_rule(&records));
    Ok(())
}
