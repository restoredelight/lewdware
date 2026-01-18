use std::{
    path::{Path, PathBuf},
    process::{self, Command},
};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

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
            } => FileInfoParts {
                file_type: FileType::Video,
                width: Some(*width),
                height: Some(*height),
                duration: Some(*duration),
                audio: Some(*audio),
                transparent: None,
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
            },
            FileType::Audio => FileInfo::Audio {
                duration: value.duration?,
            },
        })
    }
}

pub fn is_media(ffprobe_command: impl Fn() -> Command, path: &Path) -> Result<bool> {
    #[rustfmt::skip]
    let args = [
        "-v", "error",
        "-show_entries", "format=nb_streams",
        "-output_format", "default=noprint_wrappers=1:nokey=1",
    ];

    let output = ffprobe_command().args(args).arg(path).output()?;

    if !output.status.success() {
        println!("{}", String::from_utf8_lossy(&output.stderr));
        return Ok(false);
    }

    let streams: u32 = String::from_utf8_lossy(&output.stdout).trim().parse()?;

    Ok(streams > 0)
}

fn file_info(ffprobe_command: impl Fn() -> Command, path: &Path) -> Result<Option<FileInfo>> {
    #[rustfmt::skip]
    let args = [
        "-v", "error",
        "-count_packets",
        "-show_entries",
        "stream=codec_type,nb_read_packets,width,height,pix_fmt:format=duration",
        "-output_format", "json",
    ];

    let output = ffprobe_command().args(args).arg(path).output()?;

    if !output.status.success() {
        println!("{}", String::from_utf8_lossy(&output.stderr));
        return Ok(None);
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    Ok(parse_media_info(json))
}

pub fn encode_file(
    ffmpeg_command: impl Fn() -> Command,
    ffprobe_command: impl Fn() -> Command,
    input: &Path,
    output: &Path,
) -> Result<Option<(FileInfo, PathBuf)>> {
    let file_info = match file_info(ffprobe_command, input)? {
        Some(x) => x,
        None => return Ok(None),
    };

    let output = match file_info {
        FileInfo::Image { .. } => output.with_extension("avif"),
        FileInfo::Video { .. } => output.with_extension("webm"),
        FileInfo::Audio { .. } => output.with_extension("opus"),
    };

    let file_info = match file_info {
        FileInfo::Image { width, height, transparent: false } => {
            let (width, height) = encode_image(ffmpeg_command, input, &output, width, height)?;

            FileInfo::Image { width, height, transparent: false }
        }
        FileInfo::Image { width, height, transparent: true } => {
            let (width, height) = encode_image_transparent(ffmpeg_command, input, &output, width, height)?;

            FileInfo::Image { width, height, transparent: true }
        }
        FileInfo::Video {
            width,
            height,
            duration,
            audio,
        } => {
            let (width, height) =
                encode_video(ffmpeg_command, input, &output, width, height, audio, false)?;

            FileInfo::Video {
                width,
                height,
                duration,
                audio,
            }
        }
        FileInfo::Audio { .. } => {
            encode_audio(ffmpeg_command, input, &output)?;
            file_info
        }
    };

    Ok(Some((file_info, output)))
}

fn parse_media_info(json: serde_json::Value) -> Option<FileInfo> {
    let streams = json.get("streams")?.as_array()?;

    let video_stream = streams
        .iter()
        .find(|stream| stream.get("codec_type").and_then(|x| x.as_str()) == Some("video"));

    let has_audio = streams
        .iter()
        .any(|stream| stream.get("codec_type").and_then(|x| x.as_str()) == Some("audio"));

    let width = video_stream
        .and_then(|stream| stream.get("width"))
        .and_then(|x| x.as_number())
        .and_then(|x| x.as_u64());

    let height = video_stream
        .and_then(|stream| stream.get("height"))
        .and_then(|x| x.as_number())
        .and_then(|x| x.as_u64());

    let duration = json
        .get("format")?
        .get("duration")
        .and_then(|x| x.as_str())
        .and_then(|x| x.parse().ok());

    Some(match video_stream {
        Some(video_stream) => {
            if has_audio
                || video_stream
                    .get("nb_read_packets")?
                    .as_str()
                    .and_then(|x| x.parse().ok())
                    != Some(1)
            {
                FileInfo::Video {
                    width: width?,
                    height: height?,
                    duration: duration?,
                    audio: has_audio,
                }
            } else {
                let transparent = video_stream.get("pix_fmt")?.as_str()?.contains("a");

                FileInfo::Image {
                    width: width?,
                    height: height?,
                    transparent,
                }
            }
        }
        None if has_audio => FileInfo::Audio {
            duration: duration?,
        },
        None => return None,
    })
}

fn encode_image(
    ffmpeg_command: impl Fn() -> Command,
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
) -> anyhow::Result<(u64, u64)> {
    println!("{}", input.display());
    println!("{}", output.display());

    let (width, height) = resize_dimensions(width, height, MAX_IMAGE_SIZE, false);

    #[rustfmt::skip]
    let args = [
        // -i input
        "-y",
        "-c:v", "libaom-av1",
        "-cpu-used", "6",
        "-crf", "32",
        "-b:v", "0",
        "-still-picture", "1",
        "-f", "avif",
        // output
    ];

    let result = ffmpeg_command()
        .arg("-i")
        .arg(input)
        .arg("-vf")
        .arg(format!("scale=w='{width}':h='{height}',format=yuv420p"))
        .args(args)
        .arg(output)
        // .stderr(process::Stdo::null())
        .output()?;

    if !result.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&result.stderr));

        eprintln!("Encoding image failed");

        eprintln!("{}", std::fs::read_to_string(input)?);

        bail!("ffmpeg failed for {}", input.display());
    }

    Ok((width, height))
}

fn encode_image_transparent(
    ffmpeg_command: impl Fn() -> Command,
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
) -> anyhow::Result<(u64, u64)> {
    println!("{}", input.display());
    println!("{}", output.display());

    let (width, height) = resize_dimensions(width, height, MAX_IMAGE_SIZE, false);

    #[rustfmt::skip]
    let args = [
        // -i input
        "-map", "[v1]",
        "-map", "[a]",
        "-frames:v", "1",
        "-y",
        "-c:v", "libaom-av1",
        "-cpu-used", "6",
        "-crf", "32",
        "-b:v", "0",
        "-still-picture", "1",
        "-f", "avif",
        // output
    ];

    let result = ffmpeg_command()
        .arg("-i")
        .arg(input)
        .arg("-filter_complex")
        .arg(format!("[0:v]scale=w='{width}':h='{height}', format=yuva420p, split[v1][v2]; [v2]alphaextract[a]"))
        .args(args)
        .arg(output)
        // .stderr(process::Stdo::null())
        .output()?;

    if !result.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&result.stderr));

        eprintln!("Encoding image failed");

        eprintln!("{}", std::fs::read_to_string(input)?);

        bail!("ffmpeg failed for {}", input.display());
    }

    Ok((width, height))
}

fn encode_video(
    ffmpeg_command: impl Fn() -> Command,
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
    audio: bool,
    fixed_frame_rate: bool,
) -> anyhow::Result<(u64, u64)> {
    #[rustfmt::skip]
    let args = [
        "-y",
        "-crf", "30",
        "-b:v", "0",
        "-c:v", "libvpx-vp9",
        "-f", "webm",
    ];

    let (width, height) = resize_dimensions(width, height, MAX_VIDEO_SIZE, true);

    let mut command = ffmpeg_command();

    command
        .arg("-i")
        .arg(input)
        .arg("-vf")
        .arg(format!("scale=w='{width}':h='{height}'"))
        .args(args);

    if fixed_frame_rate {
        command.arg("-r").arg("30");
    }

    if audio {
        command.args(["-c:a", "libopus", "-b:a", "64k"]);
    } else {
        command.arg("-an");
    }

    let result = command.arg(output).output()?;

    if !result.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&result.stderr));

        if !fixed_frame_rate {
            if let Ok(res) = encode_video(ffmpeg_command, input, output, width, height, audio, true)
            {
                return Ok(res);
            }
        }

        eprintln!("Encoding video failed");

        bail!("ffmpeg failed for {}", input.display());
    }

    Ok((width, height))
}

fn encode_audio(
    ffmpeg_command: impl Fn() -> Command,
    input: &Path,
    output: &Path,
) -> anyhow::Result<()> {
    println!("{}", input.display());
    println!("{}", output.display());

    #[rustfmt::skip]
    let args = [
        // -i input
        "-y",
        "-c:a", "libopus",
        "-b:a", "64k",
        // tmp_path
    ];

    let status = ffmpeg_command()
        .arg("-i")
        .arg(input)
        .args(args)
        .arg(output)
        .stderr(process::Stdio::null())
        .status()?;

    if !status.success() {
        eprintln!("Audio failed");
        bail!("ffmpeg failed for {}", input.display());
    }

    Ok(())
}

const MAX_IMAGE_SIZE: u64 = 2560;
const MAX_VIDEO_SIZE: u64 = 1920;

fn resize_dimensions(
    original_width: u64,
    original_height: u64,
    max_size: u64,
    truncate: bool,
) -> (u64, u64) {
    let mut width = original_width as f64;
    let mut height = original_height as f64;

    let long_edge = width.max(height);

    if long_edge > max_size as f64 {
        let scale = max_size as f64 / long_edge;
        width *= scale;
        height *= scale;
    }

    if truncate {
        width = (width / 2.0).floor() * 2.0;
        height = (height / 2.0).floor() * 2.0;
    }

    (width.round() as u64, height.round() as u64)
}
