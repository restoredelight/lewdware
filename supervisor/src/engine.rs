use std::path::Path;
use std::process::Stdio;

use anyhow::{Result, anyhow};
use shared::schedule::SessionOverrides;
use shared::user_config::Mode;
use uuid::Uuid;

/// Builds the command to spawn one engine session. `--control-token` is always passed so the
/// engine knows which `engine_link` connection to open and identify itself with; `--dev` is only
/// added when the episode carries it (letting the engine load `AppConfig` itself exactly as it
/// does for an ordinary session otherwise).
///
/// The pack and mode do not go on the command line -- they travel in
/// `shared::schedule::SESSION_OVERRIDES_ENV`, which is not world-readable the way argv is. See
/// that constant.
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
    if dev {
        cmd.arg("--dev");
    }
    if let Some(stream_id) = dev_stream_id {
        cmd.arg("--dev-stream-id").arg(stream_id.to_string());
    }

    // A rule's own pack/mode, if it set one. Sparse: an unset field means "inherit whatever the
    // config file says", so the variable stays unset and the engine reads the config as usual.
    //
    // The episode's `mode_path` (a `lw dev` session pointed at a mode file on disk) is folded in
    // here rather than sent down a second channel, so that only one of the two can win and the
    // engine never has to arbitrate between them.
    let overrides = SessionOverrides {
        mode: overrides.mode.clone().or_else(|| {
            mode_path.map(|path| Mode::File {
                path: path.to_path_buf(),
            })
        }),
        pack_path: overrides.pack_path.clone(),
    };

    if let Some(value) = overrides.to_env_value()? {
        cmd.env(shared::schedule::SESSION_OVERRIDES_ENV, value);
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    Ok(cmd)
}
