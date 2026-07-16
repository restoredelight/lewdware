use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::attribution::ExtractedAttribution;
use crate::utils::sanitize_child_env;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum FileInfo {
    #[serde(rename = "image")]
    Image {
        width: u64,
        height: u64,
        transparent: bool,
    },
    #[serde(rename = "video")]
    Video {
        width: u64,
        height: u64,
        duration: f64,
        audio: bool,
        transparent: bool,
    },
    #[serde(rename = "audio")]
    Audio { duration: f64 },
}

pub struct FileInfoParts {
    pub file_type: FileType,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub transparent: Option<bool>,
    pub duration: Option<f64>,
    pub audio: Option<bool>,
}

#[derive(PartialEq, Eq, Debug)]
pub enum FileType {
    Image,
    Video,
    Audio,
}

impl FileType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileType::Image => "image",
            FileType::Video => "video",
            FileType::Audio => "audio",
        }
    }
}

#[derive(Debug)]
pub struct InvalidFileType();

impl std::fmt::Display for InvalidFileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Invalid file type")
    }
}

impl std::error::Error for InvalidFileType {}

impl std::str::FromStr for FileType {
    type Err = InvalidFileType;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "image" => Ok(FileType::Image),
            "video" => Ok(FileType::Video),
            "audio" => Ok(FileType::Audio),
            _ => Err(InvalidFileType()),
        }
    }
}

#[derive(Debug)]
pub struct InvalidFileInfoParts();

impl std::fmt::Display for InvalidFileInfoParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Invalid file info parts")
    }
}

impl std::error::Error for InvalidFileInfoParts {}

impl FileInfo {
    pub fn to_parts(&self) -> FileInfoParts {
        match self {
            FileInfo::Image {
                width,
                height,
                transparent,
            } => FileInfoParts {
                file_type: FileType::Image,
                width: Some(*width),
                height: Some(*height),
                transparent: Some(*transparent),
                duration: None,
                audio: None,
            },
            FileInfo::Video {
                width,
                height,
                duration,
                audio,
                transparent,
            } => FileInfoParts {
                file_type: FileType::Video,
                width: Some(*width),
                height: Some(*height),
                duration: Some(*duration),
                audio: Some(*audio),
                transparent: Some(*transparent),
            },
            FileInfo::Audio { duration } => FileInfoParts {
                file_type: FileType::Audio,
                duration: Some(*duration),
                width: None,
                height: None,
                transparent: None,
                audio: None,
            },
        }
    }

    pub fn try_from_parts(value: &FileInfoParts) -> Result<Self, InvalidFileInfoParts> {
        Self::from_parts(value).ok_or_else(InvalidFileInfoParts)
    }

    fn from_parts(value: &FileInfoParts) -> Option<Self> {
        Some(match value.file_type {
            FileType::Image => FileInfo::Image {
                width: value.width?,
                height: value.height?,
                transparent: value.transparent?,
            },
            FileType::Video => FileInfo::Video {
                width: value.width?,
                height: value.height?,
                duration: value.duration?,
                audio: value.audio?,
                transparent: value.transparent?,
            },
            FileType::Audio => FileInfo::Audio {
                duration: value.duration?,
            },
        })
    }
}

fn new_command<S: AsRef<std::ffi::OsStr>>(program: S) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    sanitize_child_env(&mut cmd);
    cmd
}

pub struct EncodedFile {
    pub info: FileInfo,
    pub thumbnail: Option<Vec<u8>>,
    pub path: PathBuf,
    pub artists: Vec<String>,
    pub source_url: Option<String>,
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
        if self != Self::SoftwareFallback
            && new_command(get_ffmpeg_path())
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

        Self::SoftwareFallback
    }
}

static FFMPEG_PATH: OnceLock<PathBuf> = OnceLock::new();
static FFPROBE_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init_binary_paths(ffmpeg: PathBuf, ffprobe: PathBuf) {
    let _ = FFMPEG_PATH.set(ffmpeg);
    let _ = FFPROBE_PATH.set(ffprobe);
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

pub fn get_ffmpeg_path() -> PathBuf {
    if let Some(p) = FFMPEG_PATH.get() {
        return p.clone();
    }

    let name = ffmpeg_filename();
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        let path = exe_dir.join(name);
        if path.exists() {
            return path;
        }
        // macOS .app bundle
        let resources = exe_dir.join("../Resources").join(name);
        if resources.exists() {
            return resources;
        }
    }

    PathBuf::from(name)
}

pub fn get_ffprobe_path() -> PathBuf {
    if let Some(p) = FFPROBE_PATH.get() {
        return p.clone();
    }

    let name = ffprobe_filename();
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        let path = exe_dir.join(name);
        if path.exists() {
            return path;
        }
        // macOS .app bundle
        let resources = exe_dir.join("../Resources").join(name);
        if resources.exists() {
            return resources;
        }
    }

    PathBuf::from(name)
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

/// Best-effort artist/source-url extraction from a video/audio container's own metadata tags
/// (`artist`/`album_artist`/`composer`, `comment`) -- a separate, lightweight ffprobe call (just
/// `format_tags`, no stream/packet counting) rather than folding into `file_info`'s probe, so a
/// failure here can never affect the required width/height/duration detection. Never fails --
/// any error (missing ffprobe, unreadable file, no tags) just yields an empty result.
fn extract_container_attribution(path: &Path) -> ExtractedAttribution {
    let mut out = ExtractedAttribution::default();
    let Ok(output) = new_command(get_ffprobe_path())
        .args(["-v", "error", "-show_entries", "format_tags", "-output_format", "json"])
        .arg(path)
        .output()
    else {
        return out;
    };
    if !output.status.success() {
        return out;
    }
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return out;
    };
    let Some(tags) = json
        .get("format")
        .and_then(|f| f.get("tags"))
        .and_then(|t| t.as_object())
    else {
        return out;
    };
    for key in ["artist", "album_artist", "composer", "performer"] {
        if let Some(v) = tags.get(key).and_then(|v| v.as_str()) {
            out.add_artist(v);
        }
    }
    if let Some(v) = tags.get("comment").and_then(|v| v.as_str()) {
        out.set_source_url(v);
    }
    out
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

    let attribution = match info {
        FileInfo::Image { .. } => crate::attribution::extract_image_attribution(input),
        FileInfo::Video { .. } | FileInfo::Audio { .. } => extract_container_attribution(input),
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
        artists: attribution.artists,
        source_url: attribution.source_url,
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

        if line.contains("lavfi.signalstats.YMIN=")
            && let Some(val_str) = line.split('=').next_back()
            && let Ok(y_min) = val_str.trim().parse::<f64>()
            && y_min < 255.0
        {
            transparent = true;
        }
    }

    let result = child.wait()?;

    if !result.success() {
        tracing::error!("{stderr_buf}");
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

        if line.contains("lavfi.signalstats.YMIN=")
            && let Some(val_str) = line.split('=').next_back()
            && let Ok(y_min) = val_str.trim().parse::<f64>()
            && y_min < 255.0
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(output);
            return encode_video_with_transparency(input, output, width, height, audio, false);
        }
    }

    let result = child.wait()?;

    if !result.success() {
        tracing::error!("{stderr_buf}");

        if !fixed_fps {
            tracing::error!("Encoding with non-fixed FPS failed; trying fixed FPS");

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
        tracing::error!("{}", String::from_utf8_lossy(&result.stderr));
        if !fixed_fps {
            tracing::error!("Encoding with non-fixed FPS failed; trying fixed FPS");

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
        tracing::error!("{}", String::from_utf8_lossy(&output.stderr));
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
    // A still image's `format` section has no `duration` key at all (there's no timeline) --
    // `.and_then` chains here (not `?`) so that absence degrades to `None` for this one field
    // instead of aborting classification of the whole file; only video/audio actually require it
    // (see the `duration?` uses below), and images never reference it.
    let duration = json
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(|d| d.as_str())
        .and_then(|s| s.parse::<f64>().ok());

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

pub fn hash_file(path: &Path) -> std::result::Result<blake3::Hash, io::Error> {
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
