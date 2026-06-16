use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    thread::available_parallelism,
};

fn new_command<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    shared::utils::sanitize_child_env(&mut cmd);
    cmd
}

use anyhow::{anyhow, bail, Context, Result};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HardwareEncoder {
    Nvidia,
    Amd,
    Intel,
    Apple,
    SoftwareFallback,
}

impl HardwareEncoder {
    pub fn detect_and_test() -> Self {
        Self::detect().test()
    }

    fn detect() -> Self {
        #[cfg(target_os = "windows")]
        {
            if let Ok(output) = new_command("powershell")
                .args(["-Command", "(Get-CimInstance Win32_VideoController).Name"])
                .output()
            {
                let gpu_name = String::from_utf8_lossy(&output.stdout).to_lowercase();
                if gpu_name.contains("nvidia") {
                    return Self::Nvidia;
                }
                if gpu_name.contains("amd") || gpu_name.contains("radeon") {
                    return Self::Amd;
                }
                if gpu_name.contains("intel") {
                    return Self::Intel;
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            return Self::Apple;
        }

        #[cfg(target_os = "linux")]
        {
            if let Ok(output) = new_command("lspci").output() {
                let pci_info = String::from_utf8_lossy(&output.stdout).to_lowercase();
                if pci_info.contains("nvidia") {
                    return Self::Nvidia;
                }
                if pci_info.contains("amd") || pci_info.contains("radeon") {
                    return Self::Amd;
                }
                if pci_info.contains("intel") {
                    return Self::Intel;
                }
            }
        }

        // If all checks fail, fallback to safe CPU encoding
        Self::SoftwareFallback
    }

    pub fn ffmpeg_args(&self) -> &[&'static str] {
        match self {
            Self::Nvidia => &[
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p4",
                "-cq",
                "23",
                "-b:v",
                "0",
            ],
            Self::Apple => &["-c:v", "h264_videotoolbox", "-q:v", "60"],
            Self::Intel => &["-c:v", "h264_qsv", "-global_quality:v", "23"],
            Self::Amd => &[
                "-c:v", "h264_amf", "-quality", "quality", "-rc", "cqp", "-qp_i", "23", "-qp_p",
                "23", "-qp_b", "23",
            ],
            Self::SoftwareFallback => &["-c:v", "libx264", "-crf", "23"],
        }
    }

    pub fn test(self) -> Self {
        if self != Self::SoftwareFallback {
            if new_command(get_ffmpeg_path())
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=black:s=128x128",
                    "-vframes",
                    "1",
                ])
                .args(self.ffmpeg_args())
                .args(["-f", "null", "-"])
                .status()
                .is_ok_and(|status| status.success())
            {
                return self;
            }
        }

        Self::SoftwareFallback
    }
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
        .filter(|e| e.path().is_file() && is_media_path(e.path()).unwrap_or(false))
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

    let ext = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
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

    let ext = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    format!("{}-{}-{}{}", base, arch, os, ext)
}

pub fn get_ffmpeg_path() -> PathBuf {
    let sidecar_name = get_ffmpeg_name();
    let direct_name = if cfg!(target_os = "windows") {
        "lewdware-ffmpeg.exe"
    } else {
        "lewdware-ffmpeg"
    };

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
    let direct_name = if cfg!(target_os = "windows") {
        "lewdware-ffprobe.exe"
    } else {
        "lewdware-ffprobe"
    };

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
        "-v",
        "error",
        "-count_packets",
        "-show_entries",
        "stream=codec_type,nb_read_packets,width,height,pix_fmt:format=duration",
        "-output_format",
        "json",
    ];

    let output = new_command(get_ffprobe_path())
        .args(args)
        .arg(path)
        .output()?;

    if !output.status.success() {
        return Ok(None);
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    Ok(parse_media_info(json))
}

pub fn encode_file(
    input: &Path,
    output: &Path,
    encoder: HardwareEncoder,
) -> Result<Option<EncodedFile>> {
    let info = match file_info(input)? {
        Some(x) => x,
        None => return Ok(None),
    };

    let output = match info {
        FileInfo::Image { .. } => output.with_extension("avif"),
        FileInfo::Video { .. } => output.with_extension("mp4"),
        FileInfo::Audio { .. } => output.with_extension("opus"),
    };

    let mut thumbnail = None;
    let info = match info {
        FileInfo::Image { width, height, .. } => {
            let (thumb, w, h, transparent) = encode_image(input, &output, width, height)?;
            thumbnail = Some(thumb);
            FileInfo::Image {
                width: w,
                height: h,
                transparent,
            }
        }
        FileInfo::Video {
            width,
            height,
            duration,
            audio,
            ..
        } => {
            let (thumb, w, h, transparent) =
                encode_video(input, &output, width, height, audio, encoder, false)?;
            thumbnail = Some(thumb);
            FileInfo::Video {
                width: w,
                height: h,
                duration,
                audio,
                transparent,
            }
        }
        FileInfo::Audio { .. } => {
            encode_audio(input, &output)?;
            info
        }
    };

    Ok(Some(EncodedFile {
        info,
        thumbnail,
        path: output,
    }))
}

fn encode_image(
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
) -> Result<(Vec<u8>, u64, u64, bool)> {
    let (width, height) = resize_dimensions(width, height, 2560, true);

    let thumb_temp = NamedTempFile::new()?;
    let thumb_path = thumb_temp.path();

    let filter = format!(
        "[0:v]scale=w='{width}':h='{height}',format=yuva420p[main]; \
         [0:v]scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease[thumb]; \
         [0:v]format=rgba,alphaextract,format=gray,signalstats,metadata=print:key=lavfi.signalstats.YMIN[alpha]"
    );

    let mut cmd = new_command(get_ffmpeg_path());
    cmd.arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-filter_complex")
        .arg(&filter);

    cmd.args([
        "-map",
        "[main]",
        "-c:v",
        "libaom-av1",
        "-cpu-used",
        "6",
        "-crf",
        "32",
        "-b:v",
        "0",
        "-still-picture",
        "1",
        "-f",
        "avif",
    ])
    .arg(output);

    cmd.args(["-map", "[thumb]", "-frames:v", "1", "-f", "webp"])
        .arg(thumb_path);

    cmd.args(["-map", "[alpha]", "-f", "null", "-"]);

    let mut child = cmd.stderr(Stdio::piped()).spawn()?;
    let stderr = child.stderr.take().context("Failed to take stderr")?;
    let reader = BufReader::new(stderr);

    let mut transparent = false;
    let mut stderr_buf = String::new();
    for line in reader.lines() {
        let line = line?;
        stderr_buf.push_str(&line);
        stderr_buf.push('\n');

        if line.contains("lavfi.signalstats.YMIN=") {
            if let Some(val_str) = line.split('=').last() {
                if let Ok(y_min) = val_str.trim().parse::<f64>() {
                    if y_min < 255.0 {
                        transparent = true;
                    }
                }
            }
        }
    }

    let result = child.wait()?;

    if !result.success() {
        eprintln!("{stderr_buf}");
        bail!("ffmpeg failed for {}", input.display());
    }

    let mut thumbnail = Vec::new();
    File::open(thumb_path)?.read_to_end(&mut thumbnail)?;
    Ok((thumbnail, width, height, transparent))
}

fn encode_video(
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
    audio: bool,
    encoder: HardwareEncoder,
    fixed_fps: bool,
) -> Result<(Vec<u8>, u64, u64, bool)> {
    let (width, height) = resize_dimensions(width, height, 1280, true);

    let thumb_temp = NamedTempFile::new()?;
    let thumb_path = thumb_temp.path();

    let filter = format!(
        "[0:v]scale=w='{width}':h='{height}',format=yuv420p[main]; \
         [0:v]scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease[thumb]; \
         [0:v]format=rgba,alphaextract,format=gray,signalstats,metadata=print:key=lavfi.signalstats.YMIN[alpha]"
    );

    let mut cmd = new_command(get_ffmpeg_path());
    cmd.arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-filter_complex")
        .arg(filter);

    cmd.args(["-map", "[main]"]);
    if audio {
        cmd.args(["-map", "0:a?", "-c:a", "libopus", "-b:a", "64k"]);
    } else {
        cmd.arg("-an");
    }

    cmd.args(encoder.ffmpeg_args()).args(["-f", "mp4"]);

    if fixed_fps {
        cmd.arg("-r").arg("30");
    }

    cmd.arg(output);

    cmd.args(["-map", "[thumb]", "-frames:v", "1", "-f", "webp"])
        .arg(thumb_path);

    cmd.args(["-map", "[alpha]", "-f", "null", "-"]);

    let mut child = cmd.stderr(Stdio::piped()).spawn()?;
    let stderr = child.stderr.take().context("Failed to take stderr")?;
    let reader = BufReader::new(stderr);

    let mut stderr_buf = String::new();
    for line in reader.lines() {
        let line = line?;
        stderr_buf.push_str(&line);
        stderr_buf.push('\n');

        if line.contains("lavfi.signalstats.YMIN=") {
            if let Some(val_str) = line.split('=').last() {
                if let Ok(y_min) = val_str.trim().parse::<f64>() {
                    if y_min < 255.0 {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = std::fs::remove_file(output);
                        return encode_video_with_transparency(
                            input, output, width, height, audio, false,
                        );
                    }
                }
            }
        }
    }

    let result = child.wait()?;

    if !result.success() {
        eprintln!("{stderr_buf}");

        if !fixed_fps {
            eprintln!("Encoding with non-fixed FPS failed; trying fixed FPS");

            if let Ok(r) = encode_video(
                input,
                output,
                width,
                height,
                audio,
                HardwareEncoder::SoftwareFallback,
                true,
            ) {
                return Ok(r);
            }
        }

        bail!("ffmpeg failed for {}", input.display());
    }

    let mut thumbnail = Vec::new();
    File::open(thumb_path)?.read_to_end(&mut thumbnail)?;
    Ok((thumbnail, width, height, false))
}

fn encode_video_with_transparency(
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
    audio: bool,
    fixed_fps: bool,
) -> anyhow::Result<(Vec<u8>, u64, u64, bool)> {
    let (width, height) = resize_dimensions(width, height, 1280, true);

    let thumb_temp = NamedTempFile::new()?;
    let thumb_path = thumb_temp.path();

    let mut command = new_command(get_ffmpeg_path());

    // Pack color (top) and alpha-as-luma (bottom) into a single 2H-tall NV12 video.
    // Both parts are encoded full-range so the shader can read alpha directly (0→transparent, 1→opaque).
    let filter = format!(
        "[0:v]scale=w='{width}':h='{height}':out_range=pc,format=yuv420p[color]; \
         [0:v]scale=w='{width}':h='{height}',format=rgba,alphaextract,scale=out_range=pc,format=yuv420p[alpha_yuv]; \
         [0:v]scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease[thumb]; \
         [color][alpha_yuv]vstack=inputs=2[out]"
    );

    command
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-filter_complex")
        .arg(filter)
        .arg("-map")
        .arg("[out]");

    if audio {
        command.args(["-map", "0:a?", "-c:a", "libopus", "-b:a", "64k"]);
    } else {
        command.arg("-an");
    }

    command.args([
        "-c:v",
        "libx264",
        "-crf",
        "23",
        "-color_range",
        "pc",
        "-pix_fmt",
        "yuv420p",
    ]);

    if fixed_fps {
        command.arg("-r").arg("30");
    }

    command
        .arg(output)
        .args(["-map", "[thumb]", "-frames:v", "1", "-f", "webp"])
        .arg(thumb_path);

    let result = command.output()?;

    if !result.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&result.stderr));
        if !fixed_fps {
            eprintln!("Encoding with non-fixed FPS failed; trying fixed FPS");

            if let Ok(res) =
                encode_video_with_transparency(input, output, width, height, audio, true)
            {
                return Ok(res);
            }
        }

        bail!("ffmpeg failed for {}", input.display());
    }

    let mut thumbnail = Vec::new();
    File::open(thumb_path)?.read_to_end(&mut thumbnail)?;

    Ok((thumbnail, width, height, true))
}

fn encode_audio(input: &Path, output: &Path) -> Result<()> {
    let mut command = new_command(get_ffmpeg_path());
    command
        .arg("-y")
        .arg("-i")
        .arg(input)
        .args(["-c:a", "libopus", "-b:a", "64k"])
        .arg(output);

    let output = command.output()?;

    if !output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
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
                || vs
                    .get("nb_read_packets")?
                    .as_str()
                    .and_then(|x| x.parse::<u32>().ok())
                    != Some(1)
            {
                FileInfo::Video {
                    width: width?,
                    height: height?,
                    duration: duration?,
                    audio: has_audio,
                    transparent: false,
                }
            } else {
                FileInfo::Image {
                    width: width?,
                    height: height?,
                    transparent: false,
                }
            }
        }
        None if has_audio => FileInfo::Audio {
            duration: duration?,
        },
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
    encoder: HardwareEncoder,
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
            let encoder = encoder.clone();
            async move {
                let _ = app.emit("upload:processing", path.to_string_lossy().as_ref());
                match process_one_file(&pack_state, &path, &dir, skip_duplicates, encoder).await {
                    Ok(Some(media_file)) => {
                        let _ = app.emit("upload:added", &media_file);
                    }
                    Ok(None) => {}
                    Err(ProcessErrorKind::Skipped) => {
                        let _ = app.emit("upload:skipped", path.to_string_lossy().as_ref());
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
    encoder: HardwareEncoder,
) -> Result<Option<MediaFile>, ProcessErrorKind> {
    let path_owned = path.to_path_buf();
    let hash = tokio::task::spawn_blocking(move || hash_file(&path_owned))
        .await
        .map_err(|e| ProcessErrorKind::Other(e.into()))?
        .map_err(ProcessErrorKind::HashError)?;

    if skip_duplicates {
        let lock = pack_state.lock().await;
        if let Some(pack) = lock.as_ref() {
            if pack
                .check_hash(&hash)
                .await
                .map_err(|e| ProcessErrorKind::Other(e))?
            {
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
        let _ = tx.send(encode_file(&path_owned, &output_path, encoder));
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
        let media = pack
            .add_file(encoded, path, hash)
            .await
            .map_err(ProcessErrorKind::PackError)?;
        Ok(Some(media))
    } else {
        Err(ProcessErrorKind::Other(anyhow!("Pack was closed")))
    }
}
