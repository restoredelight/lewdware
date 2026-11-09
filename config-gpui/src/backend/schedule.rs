//! Scheduling: OS autostart for the supervisor, and telling a running supervisor about changes.
//! Ported from `config/src-tauri/src/{autostart.rs,commands/schedule.rs}`.

use anyhow::{anyhow, Context, Result};
use auto_launch::{AutoLaunchBuilder, MacOSLaunchMode};
use shared::binaries::find_supervisor_binary_path;
use shared::ipc::control::{self, Request, Response};

/// The supervisor's "start at login" registration -- the only OS integration scheduling has
/// (`design/scheduling.md`). Never the engine.
fn auto_launch() -> Result<auto_launch::AutoLaunch> {
    let path = find_supervisor_binary_path().ok_or_else(|| {
        anyhow!("could not find an installed lewdware-supervisor binary to register")
    })?;
    AutoLaunchBuilder::new()
        .set_app_name("Lewdware")
        .set_app_path(&path.to_string_lossy())
        .set_macos_launch_mode(MacOSLaunchMode::LaunchAgent)
        .build()
        .context("failed to build autostart registration")
}

/// Registers or deregisters autostart. Blocking.
pub fn set_autostart(enabled: bool) -> Result<()> {
    let launcher = auto_launch()?;
    if enabled {
        launcher.enable().context("failed to register autostart")
    } else {
        launcher.disable().context("failed to deregister autostart")
    }
}

/// Activates an enable/disable in the supervisor: starts one if enabling, but never spawns one
/// just to tell it to stop. Best-effort.
pub async fn apply_enabled(enabled: bool) {
    if enabled {
        let _ = control::ensure_supervisor_running().await;
    }
    let _ = control::request(&Request::ReloadConfig).await;
}

/// Passes an already-saved schedule change to the supervisor, starting one if scheduling is
/// enabled. Best-effort: the save already happened, and a failure here mustn't undo it.
pub async fn reload() {
    if control::ensure_supervisor_running().await.is_ok() {
        let _ = control::request(&Request::ReloadConfig).await;
    }
}

/// Ends a running schedule pause early. No reachable supervisor means nothing is paused.
pub async fn resume() -> Result<()> {
    match control::request(&Request::ResumeSchedule).await? {
        Response::Error { message } => Err(anyhow!(message)),
        _ => Ok(()),
    }
}
