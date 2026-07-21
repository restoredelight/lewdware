use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    thread::available_parallelism,
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

pub fn get_avifenc_path() -> PathBuf {
    if let Some(p) = AVIFENC_PATH.get() {
        return p.clone();
    }

    let name = avifenc_filename();
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

/// Probes both the stream/format info needed to classify and size the file, and (for the
/// video/audio case) its container metadata tags, in one ffprobe call -- `parse_media_info` and
/// `extract_container_attribution` each just read a different part of the same JSON.
fn file_info(path: &Path) -> Result<Option<(ProbedMedia, serde_json::Value)>> {
    let args = [
        "-v",
        "error",
        "-count_frames",
        "-show_entries",
        "stream=index,codec_type,nb_read_frames,nb_frames,width,height,coded_width,\
         coded_height,duration,r_frame_rate:stream_disposition=attached_pic:format=duration\
         :format_tags",
        "-output_format",
        "json",
    ];

    let output = new_command(get_ffprobe_path())
        .args(args)
        .arg(path)
        .output()?;

    if !output.status.success() {
        tracing::error!("{}", String::from_utf8_lossy(&output.stderr));
        return Ok(None);
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    Ok(parse_media_info(&json).map(|probed| (probed, json)))
}

/// Best-effort artist/source-url extraction from a video/audio container's own metadata tags
/// (`artist`/`album_artist`/`composer`, `comment`), read from `file_info`'s own probe JSON.
/// Never fails -- missing/malformed tags just yield an empty result.
fn extract_container_attribution(json: &serde_json::Value) -> ExtractedAttribution {
    let mut out = ExtractedAttribution::default();
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
    let (probed, probe_json) = match file_info(input)? {
        Some(x) => x,
        None => return Ok(None),
    };
    let ProbedMedia { info, video, audio } = probed;

    let attribution = match info {
        FileInfo::Image { .. } => crate::attribution::extract_image_attribution(input),
        FileInfo::Video { .. } | FileInfo::Audio { .. } => {
            extract_container_attribution(&probe_json)
        }
    };

    let output = match info {
        FileInfo::Image { .. } => output.with_extension("avif"),
        FileInfo::Video { .. } => output.with_extension("mp4"),
        FileInfo::Audio { .. } => output.with_extension("opus"),
    };

    let mut thumbnail = None;
    let info = match info {
        FileInfo::Image { width, height, .. } => {
            let video = video.context("classified as image without a video stream")?;
            let (thumb, w, h, transparent) = encode_image(input, &output, width, height, video)?;
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
            audio: has_audio,
            ..
        } => {
            let streams = Streams {
                video: video.context("classified as video without a video stream")?,
                audio,
            };
            let (thumb, w, h, transparent) =
                encode_video(input, &output, width, height, streams, encoder, false)?;
            thumbnail = Some(thumb);
            FileInfo::Video {
                width: w,
                height: h,
                duration,
                audio: has_audio,
                transparent,
            }
        }
        FileInfo::Audio { .. } => {
            let audio = audio.context("classified as audio without an audio stream")?;
            encode_audio(input, &output, audio)?;
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

/// avifenc defaults to using every core for a single encode (`-j all`). Both `encode.rs`'s
/// upload path and `import.rs`'s pack-import path already rate-limit how many `encode_file`
/// calls run at once via a semaphore sized at `available_parallelism()/4` (floored at 2) --
/// mirrored here, since this crate can't see that semaphore directly -- so an unconstrained
/// avifenc multiplies past the machine's actual core count as soon as more than one image
/// encodes concurrently. Capping each invocation to its "fair share" of cores instead measured
/// ~1.75x faster for a concurrent batch than leaving it on `all`.
fn avifenc_jobs() -> usize {
    let cores = available_parallelism().map(|x| x.get()).unwrap_or(4);
    let permits = (cores / 4).max(2);
    (cores / permits).max(1)
}

fn encode_image(
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
    video: u64,
) -> Result<(Vec<u8>, u64, u64, bool)> {
    let (width, height) = resize_dimensions(width, height, 2560, true);

    let thumb_temp = NamedTempFile::new()?;
    let thumb_path = thumb_temp.path();

    // ffmpeg's libaom-av1 wrapper can't produce a working alpha auxiliary image for AVIF: it
    // silently drops a yuva420p input (falls back to yuv420p), and a manual dual-stream encode
    // hits an upstream libaom bitstream-conformance rejection (monochrome + identity-matrix
    // output requires 4:4:4, but ffmpeg's wrapper still configures 4:2:0 for it). So ffmpeg here
    // only resizes to a lossless intermediate PNG (keeping any real alpha) plus the thumbnail;
    // `avifenc` below -- which sets up the alpha-plane encode correctly itself, and is faster
    // than ffmpeg's wrapper besides -- does the actual AVIF encode.
    let main_temp = NamedTempFile::with_suffix(".png")?;
    let main_path = main_temp.path();

    // `0:{video}` rather than `0:v`: the latter is whichever video stream comes first, which is
    // not necessarily the one `parse_media_info` sized and classified. See `ProbedMedia`.
    let filter = format!(
        "[0:{video}]scale=w='{width}':h='{height}',format=rgba[main]; \
         [0:{video}]scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease[thumb]; \
         [0:{video}]format=rgba,alphaextract,format=gray,signalstats,metadata=print:key=lavfi.signalstats.YMIN[alpha]"
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
        "-frames:v",
        "1",
        "-f",
        "image2",
        "-vcodec",
        "png",
        // This PNG is a throwaway intermediate that avifenc immediately re-encodes, so its own
        // size doesn't matter -- only write speed does. zlib-compressing it is pure waste: on a
        // 2560x1440 frame this alone was ~60% of the whole ffmpeg step (more under high-entropy
        // content), for a file avifenc reads once and discards.
        "-compression_level",
        "0",
        "-pred",
        "none",
    ])
    .arg(main_path);

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

    // -y 420: matches the chroma subsampling ffmpeg's libaom wrapper used previously (smaller
    // and faster than aom/avifenc's 444 default, with no visible quality cost at this quality
    // level).
    //
    // -q 60 is the knee of the size/quality curve (and avifenc's own default): measured across
    // detailed, smooth-gradient and heavy-grain 2560px sources, a 10-point step below it buys
    // ~0.0018 SSIM while the same step above it buys ~0.0007 for comparable bytes. Encode time is
    // flat across the whole 40..80 range, so this trades only size against quality. Note this is
    // deliberately a step up from the old -crf 32 (~q49), and costs ~30% more per image for it.
    //
    // --qalpha is held above -q on purpose: alpha is typically a near-binary mask where
    // quantization shows as visible halos on cutout edges, and protecting it is cheap (+6% file
    // size at worst, on a pathological smooth-gradient alpha). Not 100 -- lossless alpha is ~6x.
    //
    // -j: see avifenc_jobs.
    let jobs = avifenc_jobs().to_string();
    let avifenc_result = new_command(get_avifenc_path())
        .args([
            "-c", "aom", "-y", "420", "-s", "6", "-q", "60", "--qalpha", "80", "-j",
        ])
        .arg(&jobs)
        .arg(main_path)
        .arg(output)
        .output()?;

    if !avifenc_result.status.success() {
        tracing::error!("{}", String::from_utf8_lossy(&avifenc_result.stderr));
        bail!("avifenc failed for {}", input.display());
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
    streams: Streams,
    encoder: HardwareEncoder,
    fixed_fps: bool,
) -> Result<(Vec<u8>, u64, u64, bool)> {
    let Streams { video, audio } = streams;
    let (width, height) = resize_dimensions(width, height, 1280, true);

    let thumb_temp = NamedTempFile::new()?;
    let thumb_path = thumb_temp.path();

    // See `ProbedMedia` for why these address one specific stream rather than using `0:v`.
    let filter = format!(
        "[0:{video}]scale=w='{width}':h='{height}',format=yuv420p[main]; \
         [0:{video}]scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease[thumb]; \
         [0:{video}]format=rgba,alphaextract,format=gray,signalstats,metadata=print:key=lavfi.signalstats.YMIN[alpha]"
    );

    let mut cmd = new_command(get_ffmpeg_path());
    cmd.arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-filter_complex")
        .arg(filter);

    cmd.args(["-map", "[main]"]);
    if let Some(audio) = audio {
        cmd.args([
            "-map",
            &format!("0:{audio}"),
            "-c:a",
            "libopus",
            "-b:a",
            "64k",
        ]);
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
            return encode_video_with_transparency(input, output, width, height, streams, false);
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
                streams,
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
    streams: Streams,
    fixed_fps: bool,
) -> anyhow::Result<(Vec<u8>, u64, u64, bool)> {
    let Streams { video, audio } = streams;
    let (width, height) = resize_dimensions(width, height, 1280, true);

    let thumb_temp = NamedTempFile::new()?;
    let thumb_path = thumb_temp.path();

    let mut command = new_command(get_ffmpeg_path());

    // Pack color (top) and alpha-as-luma (bottom) into a single 2H-tall NV12 video.
    // Both parts are encoded full-range so the shader can read alpha directly (0→transparent, 1→opaque).
    let filter = format!(
        "[0:{video}]scale=w='{width}':h='{height}':out_range=pc,format=yuv420p[color]; \
         [0:{video}]scale=w='{width}':h='{height}',format=rgba,alphaextract,scale=out_range=pc,format=yuv420p[alpha_yuv]; \
         [0:{video}]scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease[thumb]; \
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

    if let Some(audio) = audio {
        command.args([
            "-map",
            &format!("0:{audio}"),
            "-c:a",
            "libopus",
            "-b:a",
            "64k",
        ]);
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
                encode_video_with_transparency(input, output, width, height, streams, true)
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

fn encode_audio(input: &Path, output: &Path, audio: u64) -> Result<()> {
    let mut command = new_command(get_ffmpeg_path());
    command
        .arg("-y")
        .arg("-i")
        .arg(input)
        // Without an explicit map, ffmpeg picks the "best" audio stream on its own, which need
        // not be the one that was probed, and a multi-track file would be reduced arbitrarily.
        .args(["-map", &format!("0:{audio}")])
        .args(["-c:a", "libopus", "-b:a", "64k"])
        .arg(output);

    let output = command.output()?;

    if !output.status.success() {
        tracing::error!("{}", String::from_utf8_lossy(&output.stderr));
        bail!("ffmpeg failed for {}", input.display());
    }

    Ok(())
}

/// ffprobe reports numbers inconsistently -- stream dimensions come back as JSON numbers while
/// durations and frame counts come back as strings -- and uses the string "N/A" for anything it
/// couldn't determine. Parsing through the text form handles all three uniformly, with "N/A"
/// falling out as `None` rather than as a bogus value.
fn json_number<T: std::str::FromStr>(value: &serde_json::Value) -> Option<T> {
    match value.as_str() {
        Some(s) => s.trim().parse().ok(),
        None => value.as_f64()?.to_string().parse().ok(),
    }
}

fn stream_number<T: std::str::FromStr>(stream: &serde_json::Value, key: &str) -> Option<T> {
    stream.get(key).and_then(json_number)
}

fn stream_type(stream: &serde_json::Value) -> Option<&str> {
    stream.get("codec_type").and_then(|v| v.as_str())
}

/// ffprobe surfaces embedded cover art -- an MP3's album art, an m4a's poster frame -- as an
/// ordinary video stream, so classifying on stream presence alone turns every tagged music file
/// into a one-frame "video" and sends it down the video encode path. `attached_pic` is the
/// disposition flag that separates cover art from real video content.
fn is_attached_pic(stream: &serde_json::Value) -> bool {
    stream
        .get("disposition")
        .and_then(|d| d.get("attached_pic"))
        .and_then(|v| v.as_u64())
        == Some(1)
}

/// Frames in a video stream, or `None` when it can't be established -- which must not be
/// conflated with "one frame". `nb_read_frames` is ffprobe's own count from `-count_frames` and
/// is what we normally go on; `nb_frames` is the container's claim, and only covers streams
/// ffprobe declined to decode.
fn frame_count(stream: &serde_json::Value) -> Option<u64> {
    stream_number(stream, "nb_read_frames").or_else(|| stream_number(stream, "nb_frames"))
}

/// Last-resort duration for a stream whose container declares none: frame count over
/// `r_frame_rate` (itself a "num/den" string).
fn frames_over_frame_rate(stream: &serde_json::Value) -> Option<f64> {
    let frames = frame_count(stream)? as f64;
    let rate = stream.get("r_frame_rate").and_then(|v| v.as_str())?;
    let (num, den) = rate.split_once('/')?;
    let (num, den): (f64, f64) = (num.parse().ok()?, den.parse().ok()?);
    (num > 0.0 && den > 0.0).then(|| frames * den / num)
}

/// Duration in seconds, preferring the container's own value, then the stream's, then a value
/// derived from the frame count. Plenty of real files are muxed without a duration header --
/// APNG and piped matroska have none at the format level at all -- and treating that as
/// "unreadable file" drops media that is otherwise perfectly encodable.
fn media_duration(json: &serde_json::Value, stream: Option<&serde_json::Value>) -> Option<f64> {
    let candidates = [
        json.get("format")
            .and_then(|f| f.get("duration"))
            .and_then(json_number),
        stream.and_then(|s| stream_number(s, "duration")),
        stream.and_then(frames_over_frame_rate),
    ];

    candidates
        .into_iter()
        .flatten()
        .find(|d: &f64| d.is_finite() && *d > 0.0)
}

/// `coded_*` is the padded macroblock-aligned size -- a 600x800 h264 frame is coded at 608x800 --
/// so it is only ever a fallback for streams that don't report a display size, never a
/// preference.
const VIDEO_WIDTH: [&str; 2] = ["width", "coded_width"];
const VIDEO_HEIGHT: [&str; 2] = ["height", "coded_height"];

/// A stream ffprobe couldn't decode is still reported, just with every dimension zeroed. Zero is
/// never a real dimension, so refusing it here is what keeps such files out rather than admitting
/// them as 0x0 media that only fails later, during the encode.
///
/// Which formats land here is a property of the sidecar build, not of this code, so the check
/// keys on the zeroed dimensions rather than on any format in particular -- the deploy scripts
/// fetch FFmpeg master snapshots, so decoder coverage grows between releases on its own.
/// Animated WebP is the worked example: it probed 0x0 until upstream's `webp_anim` decoder
/// (a3d8ba6613) reached the sidecars, and now classifies as video through the ordinary path with
/// nothing here having changed.
fn dimension(stream: &serde_json::Value, keys: [&str; 2]) -> Option<u64> {
    keys.into_iter()
        .filter_map(|key| stream_number::<u64>(stream, key))
        .find(|d| *d > 0)
}

/// A classified file, together with the container stream indices the classification was drawn
/// from.
///
/// The indices are the whole point of this type: `parse_media_info` deliberately picks *one*
/// video stream (the largest, ignoring cover art) and *one* audio stream, so the encode has to be
/// aimed at those exact two. ffmpeg's own shorthands would re-decide independently and can pick
/// differently -- `[0:v]` is "the first video stream", which for a .ico is the first icon size
/// rather than the largest, and `-map 0:a?` is *every* audio stream, which turns a
/// multi-language video into a multi-track one. They are container indices, so they're only
/// meaningful against the file that was probed.
pub struct ProbedMedia {
    pub info: FileInfo,
    /// The stream backing `Image`/`Video`; `None` for `Audio`.
    pub video: Option<u64>,
    /// The stream backing `Audio`, or a `Video`'s soundtrack; `None` when there is no audio.
    pub audio: Option<u64>,
}

/// The streams a video encode is aimed at, carried together so they can't drift apart. See
/// `ProbedMedia`.
#[derive(Debug, Clone, Copy)]
struct Streams {
    video: u64,
    audio: Option<u64>,
}

/// The stream's own index within the container. ffprobe lists streams in index order, so the
/// array position is an accurate fallback for the rare stream that reports no index.
fn stream_index(stream: &serde_json::Value, position: usize) -> u64 {
    stream_number(stream, "index").unwrap_or(position as u64)
}

fn parse_media_info(json: &serde_json::Value) -> Option<ProbedMedia> {
    let streams = json.get("streams")?.as_array()?;
    let indexed = || streams.iter().enumerate();

    let video_stream = indexed()
        .filter(|(_, s)| stream_type(s) == Some("video") && !is_attached_pic(s))
        .max_by_key(|(_, s)| {
            // Moving pictures outrank stills. `attached_pic` already removes cover art in every
            // container that bothers to set it, but it is only a hint and plenty of files carry
            // an unflagged still as an ordinary second video stream -- and cover art is often
            // far larger than the video it decorates, so size alone would hand a 2000x2000
            // album cover the win over the 320x240 video that is the actual content. A stream
            // of exactly one frame is never the moving picture; an unknown count might be, so
            // it ranks with the movers.
            let animated = frame_count(s) != Some(1);
            // Among equals, take the largest: multi-resolution containers expose one video
            // stream per size -- a .ico holds every icon size it was built with -- in no
            // guaranteed order, and there the biggest is the one worth keeping.
            let area =
                dimension(s, VIDEO_WIDTH).unwrap_or(0) * dimension(s, VIDEO_HEIGHT).unwrap_or(0);
            (animated, area)
        });

    let audio_stream = indexed().find(|(_, s)| stream_type(s) == Some("audio"));
    let audio = audio_stream.map(|(position, s)| stream_index(s, position));

    let (info, video) = match video_stream {
        Some((position, vs)) => {
            let (width, height) = (dimension(vs, VIDEO_WIDTH)?, dimension(vs, VIDEO_HEIGHT)?);

            // A lone frame with no soundtrack is a still image. Everything else -- multiple
            // frames, an audio track, or a frame count that couldn't be established -- is
            // treated as video, since encoding a still as a one-frame video loses nothing
            // whereas the reverse throws away every frame but the first.
            let info = if audio.is_none() && frame_count(vs) == Some(1) {
                FileInfo::Image {
                    width,
                    height,
                    transparent: false,
                }
            } else {
                FileInfo::Video {
                    width,
                    height,
                    duration: media_duration(json, Some(vs)).unwrap_or(0.0),
                    audio: audio.is_some(),
                    transparent: false,
                }
            };

            (info, Some(stream_index(vs, position)))
        }
        None if audio.is_some() => (
            FileInfo::Audio {
                duration: media_duration(json, audio_stream.map(|(_, s)| s)).unwrap_or(0.0),
            },
            None,
        ),
        None => return None,
    };

    Some(ProbedMedia { info, video, audio })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every fixture below is real `ffprobe` output, captured with `file_info`'s own argument
    /// list from a file of the named kind -- trimmed only of the empty `programs`/`stream_groups`
    /// arrays and of long metadata tags, neither of which `parse_media_info` reads.
    fn probe(json: &str) -> Option<ProbedMedia> {
        parse_media_info(&serde_json::from_str(json).expect("fixture is valid JSON"))
    }

    fn parse(json: &str) -> Option<FileInfo> {
        probe(json).map(|p| p.info)
    }

    /// An MP3 carrying album art probes with a real video stream alongside the audio. It is a
    /// music file, not a 600x800 video.
    #[test]
    fn cover_art_audio_is_audio() {
        let info = parse(
            r#"{"streams":[
                {"codec_type":"audio","r_frame_rate":"0/0","duration":"7.029841",
                 "nb_read_frames":"271","disposition":{"attached_pic":0}},
                {"codec_type":"video","width":600,"height":800,"coded_width":600,
                 "coded_height":800,"r_frame_rate":"90000/1","duration":"7.029844",
                 "nb_read_frames":"1","disposition":{"attached_pic":1}}],
              "format":{"duration":"7.029841"}}"#,
        );

        assert!(
            matches!(info, Some(FileInfo::Audio { duration }) if (duration - 7.029841).abs() < 1e-6),
            "{info:?}"
        );
    }

    /// APNG declares no duration at either the format or the stream level, so the only source
    /// left is its 5 frames at 20fps. Previously the missing duration dropped the file entirely.
    #[test]
    fn apng_without_duration_is_video() {
        let info = parse(
            r#"{"streams":[
                {"codec_type":"video","width":425,"height":106,"coded_width":425,
                 "coded_height":106,"r_frame_rate":"20/1","nb_read_frames":"5",
                 "disposition":{"attached_pic":0}}],
              "format":{}}"#,
        );

        assert!(
            matches!(
                info,
                Some(FileInfo::Video { width: 425, height: 106, duration, audio: false, .. })
                    if (duration - 0.25).abs() < 1e-6
            ),
            "{info:?}"
        );
    }

    /// A matroska muxed to a pipe carries no duration header at all, and no frame rate we'd
    /// rather trust over the frames it actually has.
    #[test]
    fn matroska_without_duration_is_video() {
        let info = parse(
            r#"{"streams":[
                {"codec_type":"video","width":64,"height":64,"coded_width":64,"coded_height":64,
                 "r_frame_rate":"25/1","nb_read_frames":"50","disposition":{"attached_pic":0}}],
              "format":{"tags":{"ENCODER":"Lavf62.12.102"}}}"#,
        );

        assert!(
            matches!(info, Some(FileInfo::Video { duration, .. }) if (duration - 2.0).abs() < 1e-6),
            "{info:?}"
        );
    }

    /// A .ico holds every icon size as its own video stream. Largest wins, whatever the order.
    #[test]
    fn multi_resolution_icon_takes_largest_stream() {
        let stream = |size: u64| {
            format!(
                r#"{{"codec_type":"video","width":{size},"height":{size},"r_frame_rate":"90000/1",
                    "nb_read_frames":"1","disposition":{{"attached_pic":0}}}}"#
            )
        };
        let ascending = [16, 32, 48, 64, 128, 256].map(stream).join(",");
        let descending = [256, 128, 64, 48, 32, 16].map(stream).join(",");

        for streams in [ascending, descending] {
            let info = parse(&format!(r#"{{"streams":[{streams}],"format":{{}}}}"#));
            assert!(
                matches!(
                    info,
                    Some(FileInfo::Image {
                        width: 256,
                        height: 256,
                        ..
                    })
                ),
                "{info:?}"
            );
        }
    }

    /// h264 codes a 600x800 frame at 608x800. The display size is the one that matters, so
    /// coded_width must stay strictly a fallback.
    #[test]
    fn coded_dimensions_do_not_override_display_dimensions() {
        let info = parse(
            r#"{"streams":[
                {"codec_type":"video","width":600,"height":800,"coded_width":608,
                 "coded_height":800,"r_frame_rate":"25/1","duration":"0.040000",
                 "nb_frames":"1","nb_read_frames":"1","disposition":{"attached_pic":0}}],
              "format":{"duration":"0.040000"}}"#,
        );

        assert!(
            matches!(
                info,
                Some(FileInfo::Image {
                    width: 600,
                    height: 800,
                    ..
                })
            ),
            "{info:?}"
        );
    }

    /// The encode has to be aimed at the same streams the classification came from, so the
    /// reported indices must survive a container that orders its streams audio-first and pads
    /// them with streams we ignore.
    #[test]
    fn stream_indices_point_at_the_classified_streams() {
        let probed = probe(
            r#"{"streams":[
                {"index":0,"codec_type":"subtitle","disposition":{"attached_pic":0}},
                {"index":1,"codec_type":"audio","r_frame_rate":"0/0","duration":"2.0",
                 "nb_read_frames":"87","disposition":{"attached_pic":0}},
                {"index":2,"codec_type":"video","width":64,"height":64,"r_frame_rate":"25/1",
                 "nb_read_frames":"1","disposition":{"attached_pic":1}},
                {"index":3,"codec_type":"video","width":64,"height":64,"r_frame_rate":"25/1",
                 "nb_read_frames":"50","disposition":{"attached_pic":0}}],
              "format":{"duration":"2.0"}}"#,
        )
        .expect("classifies");

        assert!(matches!(probed.info, FileInfo::Video { audio: true, .. }));
        // Not the cover art at index 2, and not the first audio-ish stream at index 0.
        assert_eq!(probed.video, Some(3));
        assert_eq!(probed.audio, Some(1));
    }

    /// The largest stream wins, and its index travels with it -- picking size from one stream
    /// and the index from another would encode the wrong icon.
    #[test]
    fn largest_stream_index_is_reported() {
        let probed = probe(
            r#"{"streams":[
                {"index":0,"codec_type":"video","width":16,"height":16,"r_frame_rate":"90000/1",
                 "nb_read_frames":"1","disposition":{"attached_pic":0}},
                {"index":1,"codec_type":"video","width":256,"height":256,"r_frame_rate":"90000/1",
                 "nb_read_frames":"1","disposition":{"attached_pic":0}},
                {"index":2,"codec_type":"video","width":32,"height":32,"r_frame_rate":"90000/1",
                 "nb_read_frames":"1","disposition":{"attached_pic":0}}],
              "format":{}}"#,
        )
        .expect("classifies");

        assert!(matches!(
            probed.info,
            FileInfo::Image {
                width: 256,
                height: 256,
                ..
            }
        ));
        assert_eq!(probed.video, Some(1));
        assert_eq!(probed.audio, None);
    }

    /// Cover art is routinely larger than the video it decorates, so picking purely on size
    /// hands a 2000x2000 album cover the win over the 320x240 video that is the actual content.
    /// `attached_pic` catches this whenever it's set, but it's only a hint -- the single frame
    /// is what makes the still lose either way.
    #[test]
    fn oversized_cover_art_never_beats_the_real_video() {
        let video = r#"{"index":0,"codec_type":"video","width":320,"height":240,
            "r_frame_rate":"25/1","nb_read_frames":"50","disposition":{"attached_pic":0}}"#;
        let audio = r#"{"index":1,"codec_type":"audio","r_frame_rate":"0/0","duration":"2.0",
            "nb_read_frames":"87","disposition":{"attached_pic":0}}"#;
        let flagged = r#"{"index":2,"codec_type":"video","width":2000,"height":2000,
            "r_frame_rate":"90000/1","nb_read_frames":"1","disposition":{"attached_pic":1}}"#;
        // Same giant still, muxed as an ordinary second video stream with no disposition set.
        let unflagged = flagged.replace(r#""attached_pic":1"#, r#""attached_pic":0"#);

        for cover in [flagged, &unflagged] {
            let probed = probe(&format!(
                r#"{{"streams":[{video},{audio},{cover}],"format":{{"duration":"2.0"}}}}"#
            ))
            .expect("classifies");

            assert!(
                matches!(
                    probed.info,
                    FileInfo::Video {
                        width: 320,
                        height: 240,
                        ..
                    }
                ),
                "{:?}",
                probed.info
            );
            assert_eq!(probed.video, Some(0));
        }
    }

    /// The still-beats-nothing half of the rule above: when every candidate is a single frame,
    /// as in a multi-size icon, size decides after all.
    #[test]
    fn size_still_decides_between_single_frame_streams() {
        let probed = probe(
            r#"{"streams":[
                {"index":0,"codec_type":"video","width":16,"height":16,"r_frame_rate":"90000/1",
                 "nb_read_frames":"1","disposition":{"attached_pic":0}},
                {"index":1,"codec_type":"video","width":256,"height":256,"r_frame_rate":"90000/1",
                 "nb_read_frames":"1","disposition":{"attached_pic":0}}],
              "format":{}}"#,
        )
        .expect("classifies");

        assert_eq!(probed.video, Some(1));
    }

    /// Cover art is excluded from the video slot, so an album-art MP3 reports only its audio.
    #[test]
    fn cover_art_audio_reports_no_video_stream() {
        let probed = probe(
            r#"{"streams":[
                {"index":0,"codec_type":"audio","r_frame_rate":"0/0","duration":"7.0",
                 "nb_read_frames":"271","disposition":{"attached_pic":0}},
                {"index":1,"codec_type":"video","width":600,"height":800,
                 "r_frame_rate":"90000/1","nb_read_frames":"1","disposition":{"attached_pic":1}}],
              "format":{"duration":"7.0"}}"#,
        )
        .expect("classifies");

        assert_eq!(probed.video, None);
        assert_eq!(probed.audio, Some(0));
    }

    /// When ffprobe can't decode a stream it still exits 0 and still reports the stream, just
    /// with every dimension zeroed. Nothing downstream can encode that, so it has to be rejected
    /// here rather than admitted as 0x0 media. Which formats decode is a property of the sidecar
    /// build, so this stays keyed on the dimensions; see `dimension`.
    #[test]
    fn undecodable_zero_dimension_stream_is_rejected() {
        let info = parse(
            r#"{"streams":[
                {"codec_type":"video","width":0,"height":0,"coded_width":0,"coded_height":0,
                 "disposition":{"attached_pic":0}}],
              "format":{}}"#,
        );

        assert!(info.is_none(), "{info:?}");
    }

    /// Animated WebP, captured from the bundled sidecar's `webp_anim` decoder. Two of the
    /// fallbacks earn their keep at once here: the format carries no duration, so it comes from
    /// 5 frames at 20fps, and `coded_*` is *smaller* than the display size (the first frame's
    /// sub-rectangle, not the canvas) -- preferring it would encode the animation at 401x78.
    #[test]
    fn animated_webp_is_video_at_canvas_size() {
        let info = parse(
            r#"{"streams":[
                {"index":0,"codec_name":"webp_anim","codec_type":"video","width":425,
                 "height":106,"coded_width":401,"coded_height":78,"r_frame_rate":"20/1",
                 "nb_read_frames":"5","disposition":{"attached_pic":0}}],
              "format":{"format_name":"webp_anim"}}"#,
        );

        assert!(
            matches!(
                info,
                Some(FileInfo::Video { width: 425, height: 106, duration, .. })
                    if (duration - 0.25).abs() < 1e-6
            ),
            "{info:?}"
        );
    }

    /// A frame count ffprobe couldn't establish is not evidence of a still image.
    #[test]
    fn unknown_frame_count_is_video() {
        let info = parse(
            r#"{"streams":[
                {"codec_type":"video","width":64,"height":64,"r_frame_rate":"25/1",
                 "nb_read_frames":"N/A","disposition":{"attached_pic":0}}],
              "format":{"duration":"2.0"}}"#,
        );

        assert!(matches!(info, Some(FileInfo::Video { .. })), "{info:?}");
    }

    #[test]
    fn still_image_is_image() {
        let info = parse(
            r#"{"streams":[
                {"codec_type":"video","width":600,"height":800,"coded_width":600,
                 "coded_height":800,"r_frame_rate":"25/1","nb_read_frames":"1",
                 "disposition":{"attached_pic":0}}],
              "format":{}}"#,
        );

        assert!(
            matches!(
                info,
                Some(FileInfo::Image {
                    width: 600,
                    height: 800,
                    transparent: false
                })
            ),
            "{info:?}"
        );
    }

    #[test]
    fn video_with_audio_is_video() {
        let info = parse(
            r#"{"streams":[
                {"codec_type":"video","width":224,"height":768,"r_frame_rate":"2997/100",
                 "duration":"66.966967","nb_frames":"2007","nb_read_frames":"2007",
                 "disposition":{"attached_pic":0}},
                {"codec_type":"audio","r_frame_rate":"0/0","duration":"66.954667",
                 "nb_frames":"3139","nb_read_frames":"3139","disposition":{"attached_pic":0}}],
              "format":{"duration":"66.966967"}}"#,
        );

        assert!(
            matches!(
                info,
                Some(FileInfo::Video { width: 224, height: 768, audio: true, duration, .. })
                    if (duration - 66.966967).abs() < 1e-6
            ),
            "{info:?}"
        );
    }

    #[test]
    fn plain_audio_is_audio() {
        let info = parse(
            r#"{"streams":[
                {"codec_type":"audio","r_frame_rate":"0/0","duration":"7.029841",
                 "nb_read_frames":"271","disposition":{"attached_pic":0}}],
              "format":{"duration":"7.029841"}}"#,
        );

        assert!(matches!(info, Some(FileInfo::Audio { .. })), "{info:?}");
    }

    /// A container with neither audio nor video isn't media we can do anything with. ffprobe
    /// emits this shape for e.g. a subtitle-only or data-only file.
    #[test]
    fn no_usable_stream_is_rejected() {
        let info = parse(
            r#"{"streams":[{"codec_type":"subtitle","disposition":{"attached_pic":0}}],
              "format":{"duration":"2.0"}}"#,
        );

        assert!(info.is_none(), "{info:?}");
    }

    /// Cover art is the only video stream present, so there is no video content at all.
    #[test]
    fn cover_art_without_audio_stream_is_rejected() {
        let info = parse(
            r#"{"streams":[
                {"codec_type":"video","width":600,"height":800,"r_frame_rate":"90000/1",
                 "nb_read_frames":"1","disposition":{"attached_pic":1}}],
              "format":{}}"#,
        );

        assert!(info.is_none(), "{info:?}");
    }
}

/// Integration coverage against the real ffprobe sidecar.
///
/// The fixtures in `mod tests` are frozen captures, so they go on passing even if ffprobe's
/// output changes shape underneath them -- and the deploy scripts fetch FFmpeg *master*
/// snapshots, so it does change: animated WebP became decodable mid-2026 and moved from
/// "rejected as 0x0" to "classified as video" with no commit here. This module builds a corpus
/// with the sidecar ffmpeg and runs it back through the real probe, so that kind of drift
/// surfaces in CI rather than in someone's pack.
///
/// `#[ignore]`d so the default `cargo test` stays hermetic and needs no binaries. To run it:
/// `./deploy/linux/download_ffmpeg_sidecars.sh && cargo test -p shared --lib sidecar -- --ignored`
#[cfg(test)]
mod sidecar {
    use super::*;

    /// Where the sidecars live. `download_ffmpeg_sidecars.sh` stages them under `pack-editor`
    /// relative to the repo root; `LEWDWARE_SIDECAR_DIR` overrides for other layouts.
    fn binaries_dir() -> PathBuf {
        std::env::var_os("LEWDWARE_SIDECAR_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../pack-editor/src-tauri/binaries")
            })
    }

    fn init() {
        let dir = binaries_dir();
        let ffprobe = dir.join("lewdware-ffprobe");
        assert!(
            ffprobe.is_file(),
            "no ffprobe sidecar at {} -- run deploy/linux/download_ffmpeg_sidecars.sh from the \
             repo root, or set LEWDWARE_SIDECAR_DIR",
            dir.display()
        );
        init_binary_paths(
            dir.join("lewdware-ffmpeg"),
            ffprobe,
            dir.join("lewdware-avifenc"),
        );
    }

    fn ffmpeg(dir: &Path, args: &[&str]) {
        let output = new_command(binaries_dir().join("lewdware-ffmpeg"))
            .current_dir(dir)
            .args(["-y", "-loglevel", "error"])
            .args(args)
            .output()
            .expect("run sidecar ffmpeg");

        assert!(
            output.status.success(),
            "ffmpeg {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn probe(dir: &Path, name: &str) -> Option<ProbedMedia> {
        file_info(&dir.join(name))
            .unwrap_or_else(|e| panic!("probing {name}: {e}"))
            .map(|(probed, _)| probed)
    }

    /// Builds one file per shape the classifier has to tell apart.
    fn build_corpus(dir: &Path) {
        // A still, and the same animation in three containers -- two of which (APNG, WebP)
        // declare no duration at all, so it has to come from the frame count.
        ffmpeg(
            dir,
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=600x800",
                "-frames:v",
                "1",
                "still.png",
            ],
        );
        ffmpeg(
            dir,
            &["-f", "lavfi", "-i", "testsrc=s=64x64:r=20:d=1", "anim.gif"],
        );
        ffmpeg(
            dir,
            &["-i", "anim.gif", "-f", "apng", "-plays", "0", "anim.png"],
        );
        ffmpeg(dir, &["-i", "anim.gif", "-loop", "0", "anim.webp"]);

        // Audio, bare and carrying cover art.
        ffmpeg(dir, &["-f", "lavfi", "-i", "sine=d=2", "plain.mp3"]);
        ffmpeg(
            dir,
            &[
                "-i",
                "plain.mp3",
                "-i",
                "still.png",
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:a",
                "copy",
                "-c:v",
                "mjpeg",
                "-disposition:v",
                "attached_pic",
                "cover.mp3",
            ],
        );

        // A 320x240 video carrying a 2000x2000 cover, with and without the disposition flag:
        // size alone would hand the win to the cover.
        ffmpeg(
            dir,
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=2000x2000",
                "-frames:v",
                "1",
                "big.png",
            ],
        );
        ffmpeg(
            dir,
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc=s=320x240:d=2",
                "-f",
                "lavfi",
                "-i",
                "sine=d=2",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-shortest",
                "base.mp4",
            ],
        );
        ffmpeg(
            dir,
            &[
                "-i",
                "base.mp4",
                "-i",
                "big.png",
                "-map",
                "0:v",
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:v:0",
                "copy",
                "-c:a",
                "copy",
                "-c:v:1",
                "mjpeg",
                "-disposition:v:1",
                "attached_pic",
                "flagged.mp4",
            ],
        );
        ffmpeg(
            dir,
            &[
                "-i",
                "base.mp4",
                "-i",
                "big.png",
                "-map",
                "0:v",
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:v:0",
                "copy",
                "-c:a",
                "copy",
                "-c:v:1",
                "mjpeg",
                "unflagged.mp4",
            ],
        );

        // Audio muxed ahead of video, so stream order and stream role disagree.
        ffmpeg(
            dir,
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=d=2",
                "-f",
                "lavfi",
                "-i",
                "testsrc=s=320x240:d=2",
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "reordered.mkv",
            ],
        );

        std::fs::write(dir.join("notmedia.txt"), "not media").expect("write notmedia.txt");
    }

    #[test]
    #[ignore]
    fn real_probe_output_still_classifies_as_expected() {
        init();
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        build_corpus(dir);

        let still = probe(dir, "still.png").expect("still.png classifies");
        assert!(
            matches!(
                still.info,
                FileInfo::Image {
                    width: 600,
                    height: 800,
                    ..
                }
            ),
            "still.png: {:?}",
            still.info
        );

        // All three are 20 frames at 20fps; only the GIF's container says so.
        for name in ["anim.gif", "anim.png", "anim.webp"] {
            let probed = probe(dir, name).unwrap_or_else(|| panic!("{name} classifies"));
            match probed.info {
                FileInfo::Video {
                    width,
                    height,
                    duration,
                    audio: false,
                    ..
                } => {
                    assert_eq!((width, height), (64, 64), "{name} dimensions");
                    assert!((duration - 1.0).abs() < 0.3, "{name} duration {duration}");
                }
                other => panic!("{name}: expected video, got {other:?}"),
            }
        }

        let cover = probe(dir, "cover.mp3").expect("cover.mp3 classifies");
        assert!(
            matches!(cover.info, FileInfo::Audio { .. }),
            "cover.mp3: {:?}",
            cover.info
        );
        assert_eq!(cover.video, None, "cover art must not fill the video slot");

        let plain = probe(dir, "plain.mp3").expect("plain.mp3 classifies");
        assert!(
            matches!(plain.info, FileInfo::Audio { .. }),
            "{:?}",
            plain.info
        );
        assert_eq!(plain.audio, Some(0));

        for name in ["flagged.mp4", "unflagged.mp4"] {
            let probed = probe(dir, name).unwrap_or_else(|| panic!("{name} classifies"));
            assert!(
                matches!(
                    probed.info,
                    FileInfo::Video {
                        width: 320,
                        height: 240,
                        audio: true,
                        ..
                    }
                ),
                "{name}: oversized cover won, got {:?}",
                probed.info
            );
            assert_eq!(probed.video, Some(0), "{name} video stream");
        }

        let reordered = probe(dir, "reordered.mkv").expect("reordered.mkv classifies");
        assert_eq!(
            (reordered.video, reordered.audio),
            (Some(1), Some(0)),
            "indices must follow roles, not order"
        );

        assert!(probe(dir, "notmedia.txt").is_none(), "non-media accepted");
    }
}
