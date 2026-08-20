//! OS "start at login" registration for the supervisor binary (never the engine --
//! `design/scheduling.md`: "the only OS integration is autostart-at-login for the supervisor").
//! Thin wrapper around the `auto-launch` crate, which handles the three real platform mechanisms
//! (Windows registry run key, macOS LaunchAgent plist, Linux XDG autostart `.desktop` file).

use anyhow::{Context, Result, anyhow};
use auto_launch::{AutoLaunchBuilder, MacOSLaunchMode};

use shared::binaries::find_supervisor_binary_path;

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

pub fn enable() -> Result<()> {
    auto_launch()?
        .enable()
        .context("failed to register autostart")
}

pub fn disable() -> Result<()> {
    auto_launch()?
        .disable()
        .context("failed to deregister autostart")
}

#[allow(dead_code)]
pub fn is_enabled() -> Result<bool> {
    auto_launch()?
        .is_enabled()
        .context("failed to query autostart registration")
}
