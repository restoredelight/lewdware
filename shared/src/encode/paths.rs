use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::OnceLock,
};

use crate::utils::sanitize_child_env;

static AVIFENC_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Points image encoding at a specific `avifenc`, such as the sidecar an installer ships.
/// Without it, `avifenc` is looked up on `PATH` (as `lewdware-avifenc`, then `avifenc`).
///
/// avifenc is the one external program encoding still runs: ffmpeg's libaom wrapper can't write
/// AVIF alpha correctly (see `encode_image`). Video and audio go through the linked ffmpeg
/// libraries.
pub fn init_avifenc_path(avifenc: PathBuf) {
    let _ = AVIFENC_PATH.set(avifenc);
}

fn avifenc_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "lewdware-avifenc.exe"
    } else {
        "lewdware-avifenc"
    }
}

pub fn get_avifenc_path() -> PathBuf {
    AVIFENC_PATH
        .get_or_init(|| {
            if let Ok(path) = which::which(avifenc_filename()) {
                path
            } else if let Ok(path) = which::which("avifenc") {
                path
            } else {
                panic!(
                    "avifenc binary not found. Please install avifenc or set the AVIFENC_PATH environment variable."
                );
            }
        })
        .clone()
}

pub fn new_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null());
    sanitize_child_env(&mut cmd);
    cmd
}
