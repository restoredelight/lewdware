use std::path::Path;
use std::process::Stdio;

use anyhow::{Result, anyhow};
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
) -> Result<tokio::process::Command> {
    let cmd = shared::child::find_engine_binary()
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

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    Ok(cmd)
}
