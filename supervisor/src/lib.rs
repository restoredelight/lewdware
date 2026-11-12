//! The supervisor daemon: owns the session lifecycle, the schedule, the tray, the stop shortcut
//! and the wallpaper. Run through the `lewdware-supervisor` executable (`src/main.rs` in
//! development, a launcher loading the shared core in releases; see `core/`).

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
/// `DiagnoseSchedule` is the one exception and is handled in [`run`] rather than `client`: it
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
    /// Exits 0 if this process may listen for the stop shortcut, 1 if not. Run by a daemon
    /// waiting for Input Monitoring: a process's answer never changes once asked, so it asks a
    /// new one. See `stop_key.rs`.
    #[command(hide = true)]
    CheckInputMonitoring,
}

/// Runs the daemon, or the one-shot command on this process's command line.
pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // macOS: send notifications as Lewdware.app. Otherwise mac-notification-sys looks the sender
    // up with an AppleScript `id of application "use_default"`, which names no app, so AppleScript
    // asks the user where it is: a dialog this process can't show, which blocks it forever,
    // bouncing in the Dock. (An engine that failed to start hung there, its session stuck at
    // "Starting".) Fails only if already set.
    #[cfg(target_os = "macos")]
    let _ = notify_rust::set_application(shared::MACOS_BUNDLE_ID);

    match cli.command {
        None => daemon::run(),
        Some(Command::DiagnoseSchedule { path }) => diagnose_schedule(path),
        Some(Command::CheckInputMonitoring) => {
            std::process::exit(if shared::input_monitoring::granted() {
                0
            } else {
                1
            })
        }
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
