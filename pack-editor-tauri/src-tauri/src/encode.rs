use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    thread::available_parallelism,
};

use anyhow::{anyhow, bail, Result};
use futures::{stream, StreamExt};
use infer::MatcherType;
use shared::encode::FileInfo;
use tempfile::NamedTempFile;
use tokio::sync::{oneshot, Semaphore};
use uuid::Uuid;
use walkdir::WalkDir;

use tauri::Emitter;

use crate::pack::MediaFile;

pub struct EncodedFile {
    pub info: FileInfo,
    pub thumbnail: Option<Vec<u8>>,
    pub path: PathBuf,
}

#[derive(Debug)]
pub enum ProcessErrorKind {
    Skipped,
    EncodeError(anyhow::Error),
    PackError(anyhow::Error),
    HashError(io::Error),
    Other(anyhow::Error),
}

impl std::fmt::Display for ProcessErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skipped => write!(f, "Duplicate (skipped)"),
            Self::EncodeError(e) => write!(f, "Encode error: {e}"),
            Self::PackError(e) => write!(f, "Pack error: {e}"),
            Self::HashError(e) => write!(f, "Hash error: {e}"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

static ENCODE_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();

fn encode_semaphore() -> &'static Semaphore {
    ENCODE_SEMAPHORE.get_or_init(|| {
        let permits = available_parallelism()
            .map(|x| (x.get() / 4).max(2))
            .unwrap_or(2);
        Semaphore::new(permits)
    })
}

pub fn explore_folder(path: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut walkdir = WalkDir::new(path);
    if !recursive {
        walkdir = walkdir.max_depth(1);
    }
    walkdir
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().is_file() && is_media_path(e.path()).unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

fn is_media_path(path: &Path) -> anyhow::Result<bool> {
    let guess = mime_guess::from_path(path);
    if guess.iter().any(|m| {
        let t = m.type_();
        t == mime_guess::mime::IMAGE || t == mime_guess::mime::VIDEO || t == mime_guess::mime::AUDIO
    }) {
        return Ok(true);
    }
    if guess.first().is_some() {
        return Ok(false);
    }
    if let Some(g) = infer::get_from_path(path)? {
        let t = g.matcher_type();
        if t == MatcherType::Image || t == MatcherType::Audio || t == MatcherType::Video {
            return Ok(true);
        }
    }
    Ok(false)
}

fn get_ffmpeg_name() -> String {
    let base = "lewdware-ffmpeg";
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        return base.to_string();
    };

    let os = if cfg!(target_os = "windows") {
        "pc-windows-msvc"
    } else if cfg!(target_os = "macos") {
        "apple-darwin"
    } else if cfg!(target_os = "linux") {
        "unknown-linux-gnu"
    } else {
        return base.to_string();
    };

    let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
    format!("{}-{}-{}{}", base, arch, os, ext)
}

fn get_ffprobe_name() -> String {
    let base = "lewdware-ffprobe";
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        return base.to_string();
    };

    let os = if cfg!(target_os = "windows") {
        "pc-windows-msvc"
    } else if cfg!(target_os = "macos") {
        "apple-darwin"
    } else if cfg!(target_os = "linux") {
        "unknown-linux-gnu"
    } else {
        return base.to_string();
    };

    let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
    format!("{}-{}-{}{}", base, arch, os, ext)
}

pub fn get_ffmpeg_path() -> PathBuf {
    let sidecar_name = get_ffmpeg_name();
    let direct_name = if cfg!(target_os = "windows") { "lewdware-ffmpeg.exe" } else { "lewdware-ffmpeg" };

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // Check 1: Sibling direct name
            let path = exe_dir.join(direct_name);
            if path.exists() {
                return path;
            }

            // Check 2: Sibling sidecar name
            let path = exe_dir.join(&sidecar_name);
            if path.exists() {
                return path;
            }

            // Check 3: macOS bundle Resources directory
            let macos_resources = exe_dir.join("../Resources").join(&sidecar_name);
            if macos_resources.exists() {
                return macos_resources;
            }

            // Check 4: macOS bundle Resources direct
            let macos_resources_direct = exe_dir.join("../Resources").join(direct_name);
            if macos_resources_direct.exists() {
                return macos_resources_direct;
            }
        }
    }

    PathBuf::from(direct_name)
}

pub fn get_ffprobe_path() -> PathBuf {
    let sidecar_name = get_ffprobe_name();
    let direct_name = if cfg!(target_os = "windows") { "lewdware-ffprobe.exe" } else { "lewdware-ffprobe" };

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // Check 1: Sibling direct name
            let path = exe_dir.join(direct_name);
            if path.exists() {
                return path;
            }

            // Check 2: Sibling sidecar name
            let path = exe_dir.join(&sidecar_name);
            if path.exists() {
                return path;
            }

            // Check 3: macOS bundle Resources directory
            let macos_resources = exe_dir.join("../Resources").join(&sidecar_name);
            if macos_resources.exists() {
                return macos_resources;
            }

            // Check 4: macOS bundle Resources direct
            let macos_resources_direct = exe_dir.join("../Resources").join(direct_name);
            if macos_resources_direct.exists() {
                return macos_resources_direct;
            }
        }
    }

    PathBuf::from(direct_name)
}

fn file_info(path: &Path) -> Result<Option<FileInfo>> {
    let args = [
        "-v", "error",
        "-count_packets",
        "-show_entries",
        "stream=codec_type,nb_read_packets,width,height,pix_fmt:format=duration",
        "-output_format", "json",
    ];

    let output = Command::new(get_ffprobe_path()).args(args).arg(path).output()?;

    if !output.status.success() {
        return Ok(None);
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    Ok(parse_media_info(json))
}

pub fn encode_file(input: &Path, output: &Path) -> Result<Option<EncodedFile>> {
    let info = match file_info(input)? {
        Some(x) => x,
        None => return Ok(None),
    };

    let output = match info {
        FileInfo::Image { .. } => output.with_extension("avif"),
        FileInfo::Video { .. } => output.with_extension("webm"),
        FileInfo::Audio { .. } => output.with_extension("opus"),
    };

    let mut thumbnail = None;
    let info = match info {
        FileInfo::Image { width, height, .. } => {
            let (thumb, w, h) = encode_image(input, &output, width, height)?;
            thumbnail = Some(thumb);
            FileInfo::Image { width: w, height: h, transparent: false }
        }
        FileInfo::Video { width, height, duration, audio } => {
            let (thumb, w, h) = encode_video(input, &output, width, height, audio, false)?;
            thumbnail = Some(thumb);
            FileInfo::Video { width: w, height: h, duration, audio }
        }
        FileInfo::Audio { .. } => {
            encode_audio(input, &output)?;
            info
        }
    };

    Ok(Some(EncodedFile { info, thumbnail, path: output }))
}

fn encode_image(input: &Path, output: &Path, width: u64, height: u64) -> Result<(Vec<u8>, u64, u64)> {
    let (width, height) = resize_dimensions(width, height, 2560, true);

    let thumb_temp = NamedTempFile::new()?;
    let thumb_path = thumb_temp.path();

    let filter = format!(
        "[0:v]scale=w='{width}':h='{height}',format=yuv420p[main]; \
         [0:v]scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease[thumb]"
    );

    let result = Command::new(get_ffmpeg_path())
        .arg("-i").arg(input)
        .arg("-filter_complex").arg(filter)
        .arg("-map").arg("[main]")
        .args(["-y", "-c:v", "libaom-av1", "-cpu-used", "6", "-crf", "32", "-b:v", "0", "-still-picture", "1", "-f", "avif"])
        .arg(output)
        .arg("-map").arg("[thumb]")
        .args(["-frames:v", "1", "-f", "webp"])
        .arg(thumb_path)
        .output()?;

    if !result.status.success() {
        bail!("ffmpeg failed for {}", input.display());
    }

    let mut thumbnail = Vec::new();
    File::open(thumb_path)?.read_to_end(&mut thumbnail)?;
    Ok((thumbnail, width, height))
}

fn encode_video(
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
    audio: bool,
    fixed_fps: bool,
) -> Result<(Vec<u8>, u64, u64)> {
    let (width, height) = resize_dimensions(width, height, 1920, true);

    let thumb_temp = NamedTempFile::new()?;
    let thumb_path = thumb_temp.path();

    let filter = format!(
        "[0:v]scale=w='{width}':h='{height}'[main]; \
         [0:v]scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease[thumb]"
    );

    let mut cmd = Command::new(get_ffmpeg_path());
    cmd.arg("-i").arg(input)
        .arg("-filter_complex").arg(filter)
        .arg("-map").arg("[main]");

    if audio {
        cmd.args(["-map", "0:a?", "-c:a", "libopus", "-b:a", "64k"]);
    } else {
        cmd.arg("-an");
    }

    cmd.args(["-y", "-crf", "30", "-b:v", "0", "-c:v", "libvpx-vp9", "-f", "webm"]);

    if fixed_fps {
        cmd.arg("-r").arg("30");
    }

    cmd.arg(output)
        .arg("-map").arg("[thumb]")
        .args(["-frames:v", "1", "-f", "webp"])
        .arg(thumb_path);

    let result = cmd.output()?;

    if !result.status.success() {
        if !fixed_fps {
            if let Ok(r) = encode_video(input, output, width, height, audio, true) {
                return Ok(r);
            }
        }
        bail!("ffmpeg failed for {}", input.display());
    }

    let mut thumbnail = Vec::new();
    File::open(thumb_path)?.read_to_end(&mut thumbnail)?;
    Ok((thumbnail, width, height))
}

fn encode_audio(input: &Path, output: &Path) -> Result<()> {
    let status = Command::new(get_ffmpeg_path())
        .arg("-i").arg(input)
        .args(["-y", "-c:a", "libopus", "-b:a", "64k"])
        .arg(output)
        .status()?;

    if !status.success() {
        bail!("ffmpeg failed for {}", input.display());
    }
    Ok(())
}

fn parse_media_info(json: serde_json::Value) -> Option<FileInfo> {
    let streams = json.get("streams")?.as_array()?;

    let video_stream = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("video"));

    let has_audio = streams
        .iter()
        .any(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("audio"));

    let width = video_stream
        .and_then(|s| s.get("width"))
        .and_then(|v| v.as_u64());
    let height = video_stream
        .and_then(|s| s.get("height"))
        .and_then(|v| v.as_u64());
    let duration = json.get("format")?.get("duration")?.as_str()?.parse().ok();

    Some(match video_stream {
        Some(vs) => {
            if has_audio
                || vs.get("nb_read_packets")?
                    .as_str()
                    .and_then(|x| x.parse::<u32>().ok())
                    != Some(1)
            {
                FileInfo::Video { width: width?, height: height?, duration: duration?, audio: has_audio }
            } else {
                let transparent = vs.get("pix_fmt")?.as_str()?.contains('a');
                FileInfo::Image { width: width?, height: height?, transparent }
            }
        }
        None if has_audio => FileInfo::Audio { duration: duration? },
        None => return None,
    })
}

fn hash_file(path: &Path) -> Result<blake3::Hash, io::Error> {
    let file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(file)?;
    Ok(hasher.finalize())
}

fn resize_dimensions(w: u64, h: u64, max: u64, truncate: bool) -> (u64, u64) {
    let (mut fw, mut fh) = (w as f64, h as f64);
    let long = fw.max(fh);
    if long > max as f64 {
        let scale = max as f64 / long;
        fw *= scale;
        fh *= scale;
    }
    if truncate {
        fw = (fw / 2.0).floor() * 2.0;
        fh = (fh / 2.0).floor() * 2.0;
    }
    (fw.round() as u64, fh.round() as u64)
}

// Called from Tauri commands via AppHandle
pub async fn process_files(
    pack_state: crate::PackState,
    paths: Vec<PathBuf>,
    skip_duplicates: bool,
    app: tauri::AppHandle,
) {
    let dir = {
        let lock = pack_state.lock().await;
        match lock.as_ref() {
            Some(pack) => pack.dir().to_path_buf(),
            None => return,
        }
    };

    let limit = available_parallelism().map(|x| x.get()).ok();

    stream::iter(paths)
        .for_each_concurrent(limit, |path| {
            let pack_state = pack_state.clone();
            let app = app.clone();
            let dir = dir.clone();
            async move {
                let _ = app.emit("upload:processing", path.to_string_lossy().as_ref());
                match process_one_file(&pack_state, &path, &dir, skip_duplicates).await {
                    Ok(Some(media_file)) => {
                        let _ = app.emit("upload:added", &media_file);
                    }
                    Ok(None) => {}
                    Err(ProcessErrorKind::Skipped) => {
                        let _ = app.emit(
                            "upload:skipped",
                            path.to_string_lossy().as_ref(),
                        );
                    }
                    Err(err) => {
                        let _ = app.emit(
                            "upload:error",
                            serde_json::json!({
                                "path": path.to_string_lossy(),
                                "error": err.to_string()
                            }),
                        );
                    }
                }
                let _ = app.emit("upload:file-done", ());
            }
        })
        .await;

    let _ = app.emit("upload:done", ());
}

async fn process_one_file(
    pack_state: &crate::PackState,
    path: &Path,
    dir: &Path,
    skip_duplicates: bool,
) -> Result<Option<MediaFile>, ProcessErrorKind> {
    let path_owned = path.to_path_buf();
    let hash = tokio::task::spawn_blocking(move || hash_file(&path_owned))
        .await
        .map_err(|e| ProcessErrorKind::Other(e.into()))?
        .map_err(ProcessErrorKind::HashError)?;

    if skip_duplicates {
        let lock = pack_state.lock().await;
        if let Some(pack) = lock.as_ref() {
            if pack.check_hash(&hash).await.map_err(|e| ProcessErrorKind::Other(e))? {
                return Err(ProcessErrorKind::Skipped);
            }
        }
    }

    let _permit = encode_semaphore()
        .acquire()
        .await
        .map_err(|e| ProcessErrorKind::Other(anyhow!("{e}")))?;

    let id = Uuid::new_v4();
    let output_path = dir.join("media").join(id.to_string());
    let path_owned = path.to_path_buf();

    let (tx, rx) = oneshot::channel();
    rayon::spawn(move || {
        let _ = tx.send(encode_file(&path_owned, &output_path));
    });

    let encoded = rx
        .await
        .map_err(|e| ProcessErrorKind::Other(e.into()))?
        .map_err(ProcessErrorKind::EncodeError)?;

    let encoded = match encoded {
        Some(e) => e,
        None => return Ok(None),
    };

    let mut lock = pack_state.lock().await;
    if let Some(pack) = lock.as_mut() {
        let media = pack.add_file(encoded, path, hash).await.map_err(ProcessErrorKind::PackError)?;
        Ok(Some(media))
    } else {
        Err(ProcessErrorKind::Other(anyhow!("Pack was closed")))
    }
}
