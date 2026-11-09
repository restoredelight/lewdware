//! Asking the engine about the machine. Ported from `config/src-tauri/src/commands/hardware.rs`.
//!
//! The engine is the process that will actually open the windows and audio sinks, and it sees
//! devices differently from this one (it forces XWayland; audio ids carry the cpal host that
//! produced them). So it is run as a short-lived probe rather than measured here -- an id taken
//! from this process would be saved and then never match what the engine compares it against.

use std::process::Stdio;
use std::time::Duration;

use shared::audio::{self, AudioDeviceInfo, TestAudioResult};

/// Runs the engine binary with `args` and returns its stdout. The probe opens an event loop or an
/// audio device, so it is capped rather than allowed to hang the settings window.
async fn run_engine_probe(args: &[&str], timeout: Duration, what: &str) -> anyhow::Result<Vec<u8>> {
    let mut command = tokio::process::Command::from(
        shared::binaries::find_engine_binary()
            .ok_or_else(|| anyhow::anyhow!("could not find the lewdware-engine binary"))?,
    );
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| anyhow::anyhow!("timed out asking the engine to {what}"))?
        .map_err(|e| anyhow::anyhow!("could not run the engine to {what}: {e}"))?;

    if !output.status.success() {
        anyhow::bail!(
            "the engine could not {what}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

pub async fn audio_devices() -> anyhow::Result<Vec<AudioDeviceInfo>> {
    let stdout = run_engine_probe(
        &[audio::LIST_AUDIO_DEVICES_COMMAND],
        Duration::from_secs(5),
        "list the audio devices",
    )
    .await?;
    serde_json::from_slice(&stdout)
        .map_err(|e| anyhow::anyhow!("could not read the engine's audio device list: {e}"))
}

/// Plays the test chime on `device` (the system default when `None`) and resolves once it has
/// finished, reporting whether the engine had to fall back to another device.
pub async fn test_audio(device: Option<String>) -> anyhow::Result<TestAudioResult> {
    let device = device.unwrap_or_else(|| audio::TEST_AUDIO_DEFAULT.to_string());
    let stdout = run_engine_probe(
        &[audio::TEST_AUDIO_COMMAND, &device],
        Duration::from_secs(15),
        "play the test sound",
    )
    .await?;
    serde_json::from_slice(&stdout)
        .map_err(|e| anyhow::anyhow!("could not read the engine's test sound result: {e}"))
}

/// Whether the stop shortcut can be heard. Only macOS gates global key listening (Input
/// Monitoring); the prompt for it is not ported yet, so this reports granted everywhere.
pub fn input_monitoring_granted() -> bool {
    true
}

/// Lists the monitors, primary first. Asked of the engine because this process and the engine
/// disagree about monitor names and geometry (the engine forces XWayland; see `shared::monitor`),
/// and an id measured here would be saved into `disabled_monitors` and then never match. No
/// fallback to this process's own view: a visible error beats a control that quietly does nothing.
pub async fn monitors() -> anyhow::Result<Vec<shared::monitor::MonitorInfo>> {
    let stdout = run_engine_probe(
        &[shared::monitor::LIST_MONITORS_COMMAND],
        Duration::from_secs(15),
        "list monitors",
    )
    .await?;
    let mut monitors: Vec<shared::monitor::MonitorInfo> = serde_json::from_slice(&stdout)
        .map_err(|e| anyhow::anyhow!("could not read the engine's monitor list: {e}"))?;
    if let Some(primary) = monitors.iter().position(|m| m.primary) {
        monitors.swap(0, primary);
    }
    Ok(monitors)
}
