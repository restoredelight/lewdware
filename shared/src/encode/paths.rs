use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::OnceLock,
};

use crate::utils::sanitize_child_env;

static FFMPEG_PATH: OnceLock<PathBuf> = OnceLock::new();
static FFPROBE_PATH: OnceLock<PathBuf> = OnceLock::new();
static AVIFENC_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init_binary_paths(ffmpeg: PathBuf, ffprobe: PathBuf, avifenc: PathBuf) {
    let _ = FFMPEG_PATH.set(ffmpeg);
    let _ = FFPROBE_PATH.set(ffprobe);
    let _ = AVIFENC_PATH.set(avifenc);
}

fn ffmpeg_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "lewdware-ffmpeg.exe"
    } else {
        "lewdware-ffmpeg"
    }
}

fn ffprobe_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "lewdware-ffprobe.exe"
    } else {
        "lewdware-ffprobe"
    }
}

fn avifenc_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "lewdware-avifenc.exe"
    } else {
        "lewdware-avifenc"
    }
}

pub fn get_ffmpeg_path() -> PathBuf {
    FFMPEG_PATH
        .get_or_init(|| {
            if let Ok(path) = which::which(ffmpeg_filename()) {
                path
            } else if let Ok(path) = which::which("ffmpeg") {
                path
            } else {
                panic!(
                    "ffmpeg binary not found. Please install ffmpeg or set the FFMPEG_PATH environment variable."
                );
            }
        })
        .clone()
}

pub fn get_ffprobe_path() -> PathBuf {
    FFPROBE_PATH
        .get_or_init(|| {
            if let Ok(path) = which::which(ffprobe_filename()) {
                path
            } else if let Ok(path) = which::which("ffprobe") {
                path
            } else {
                panic!(
                    "ffprobe binary not found. Please install ffprobe or set the FFPROBE_PATH environment variable."
                );
            }
        })
        .clone()
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
