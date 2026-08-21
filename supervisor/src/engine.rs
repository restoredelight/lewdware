use std::path::Path;
use std::process::Stdio;

use anyhow::{Result, anyhow};
use shared::schedule::SessionOverrides;
use uuid::Uuid;

/// Builds the command to spawn one engine session. `--control-token` is always passed so the
/// engine knows which `engine_link` connection to open and identify itself with; `--mode-path`/
/// `--dev` are only added when the episode carries them (letting the engine load `AppConfig`
/// itself exactly as it does for an ordinary session otherwise).
pub fn build_command(
    seq: u64,
    mode_path: Option<&Path>,
    dev: bool,
    dev_stream_id: Option<Uuid>,
    overrides: &SessionOverrides,
) -> Result<tokio::process::Command> {
    let cmd = shared::binaries::find_engine_binary()
        .ok_or_else(|| anyhow!("could not find the lewdware-engine binary"))?;
    let mut cmd = tokio::process::Command::from(cmd);

    cmd.arg("--control-token").arg(seq.to_string());
    if let Some(path) = mode_path {
        cmd.arg("--mode-path").arg(path);
    }
    if dev {
        cmd.arg("--dev");
    }
    if let Some(stream_id) = dev_stream_id {
        cmd.arg("--dev-stream-id").arg(stream_id.to_string());
    }

    // A rule's own pack/mode, if it set one. Sparse: an unset field means "inherit whatever the
    // config file says", so nothing is passed and the engine reads it as usual.
    if let Some(pack) = &overrides.pack_path {
        cmd.arg("--pack-path").arg(pack);
    }
    if let Some(mode) = &overrides.mode {
        cmd.arg("--mode").arg(serde_json::to_string(mode)?);
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    Ok(cmd)
}
