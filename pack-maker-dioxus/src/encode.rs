use std::{
    fs::File,
    path::{Path, PathBuf},
    process::{self, Command},
    thread::available_parallelism,
};

use anyhow::{anyhow, bail, Result};
use dioxus::signals::{ReadSignal, ReadableExt, Signal, SyncSignal, WritableExt};
use dioxus_stores::Store;
use futures::{stream, StreamExt};
use image::{imageops::FilterType, ImageFormat, ImageReader};
use shared::encode::FileInfo;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::{image_list::Media, pack::MediaPack, utils::file_name};

async fn process_files(
    media_pack: ReadSignal<MediaPack>,
    paths: Vec<PathBuf>,
    processing: SyncSignal<String>,
    mut files: Store<Vec<Media>>,
    mut errors: Store<Vec<anyhow::Error>>,
    mut processed: Signal<usize>,
) -> Result<()> {
    let media_ref = media_pack.read();
    let dir = media_ref.dir();
    let id = media_ref.id();

    stream::iter(paths)
        .for_each_concurrent(Some(available_parallelism()?.get()), |path| async move {
            match process_file(media_pack, &path, dir.to_path_buf(), processing).await {
                Ok(Some((encoded_file, hash))) => {
                    match media_pack.read().add_file(encoded_file, &path, hash).await {
                        Ok(media) => {
                            files.push(media);
                        },
                        Err(err) => {
                            errors.push(err);
                        },
                    };
                }
                Ok(None) => {}
                Err(err) => {
                    errors.push(err);
                }
            }

            processed += 1;
        })
        .await;

    Ok(())
}

async fn process_file(
    media_view: ReadSignal<MediaPack>,
    path: &Path,
    dir: PathBuf,
    mut processing: SyncSignal<String>,
) -> Result<Option<(EncodedFile, blake3::Hash)>> {
    let hash = {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || hash_file(&path)).await??
    };

    if media_view.read().check_hash(&hash).await? {
        bail!("Duplicate file (skipped)");
    }
    let (tx, rx) = oneshot::channel();

    let path = path.to_path_buf();

    let mut fun = move || -> anyhow::Result<_> {
        *processing.write() = file_name(&path);
        let id = Uuid::new_v4();

        let output_path = dir.join("media").join(id.to_string());

        encode_file(&path, &output_path)
    };

    rayon::spawn(move || {
        if tx.is_closed() {
            return;
        }

        let _ = tx.send(fun());
    });

    Ok(rx.await??.map(|encoded| (encoded, hash)))
}

fn hash_file(path: &Path) -> anyhow::Result<blake3::Hash> {
    let file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();

    hasher.update_reader(file)?;

    Ok(hasher.finalize())
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

fn file_info(path: &Path) -> Result<Option<FileInfo>> {
    #[rustfmt::skip]
    let args = [
        "-v", "error",
        "-count_packets",
        "-show_entries",
        "stream=codec_type,nb_read_packets,width,height,pix_fmt:format=duration",
        "-output_format", "json",
    ];

    let output = Command::new("ffprobe").args(args).arg(path).output()?;

    if !output.status.success() {
        println!("{}", String::from_utf8_lossy(&output.stderr));
        return Ok(None);
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    Ok(parse_media_info(json))
}

pub struct EncodedFile {
    pub info: FileInfo,
    pub thumbnail: Option<Vec<u8>>,
}

pub fn encode_file(input: &Path, output: &Path) -> Result<Option<EncodedFile>> {
    let file_info = match file_info(input)? {
        Some(x) => x,
        None => return Ok(None),
    };

    let output = match file_info {
        FileInfo::Image { .. } => output.with_extension("avif"),
        FileInfo::Video { .. } => output.with_extension("webm"),
        FileInfo::Audio { .. } => output.with_extension("opus"),
    };

    let mut thumbnail = None;
    let file_info = match file_info {
        FileInfo::Image { width, height, .. } => {
            let (thumb, width, height) = encode_image_(input, &output, width, height)?;
            thumbnail = Some(thumb);

            FileInfo::Image {
                width,
                height,
                transparent: false,
            }
        }
        FileInfo::Video {
            width,
            height,
            duration,
            audio,
        } => {
            let (thumb, width, height) = encode_video(input, &output, width, height, audio, false)?;
            thumbnail = Some(thumb);

            FileInfo::Video {
                width,
                height,
                duration,
                audio,
            }
        }
        FileInfo::Audio { .. } => {
            encode_audio(input, &output)?;
            file_info
        }
    };

    Ok(Some(EncodedFile {
        info: file_info,
        thumbnail,
    }))
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

pub fn encode_image_(
    path: &Path,
    output: &Path,
    width: u64,
    height: u64,
) -> Result<(Vec<u8>, u64, u64)> {
    let (width, height) = resize_dimensions(width, height, MAX_IMAGE_SIZE, false);

    let image = ImageReader::open(path)?.with_guessed_format()?.decode()?;

    let thumbnail = image.thumbnail(100, 100);
    let webp_encoder = webp::Encoder::from_image(&thumbnail).map_err(|err| anyhow!("{err}"))?;
    let thumbnail_webp = webp_encoder.encode(75.0).to_vec();

    let image = if image.width() != width as u32 || image.height() != height as u32 {
        image.resize_exact(width as u32, height as u32, FilterType::Lanczos3)
    } else {
        image
    };

    image.save_with_format(output, ImageFormat::Avif)?;

    Ok((thumbnail_webp, width, height))
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
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
    audio: bool,
    fixed_frame_rate: bool,
) -> anyhow::Result<(Vec<u8>, u64, u64)> {
    #[rustfmt::skip]
    let args = [
        "-y",
        "-crf", "30",
        "-b:v", "0",
        "-c:v", "libvpx-vp9",
        "-f", "webm",
    ];

    let (width, height) = resize_dimensions(width, height, MAX_VIDEO_SIZE, true);

    let mut command = Command::new("ffmpeg");

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
            if let Ok(res) = encode_video(input, output, width, height, audio, true) {
                return Ok(res);
            }
        }

        eprintln!("Encoding video failed");

        bail!("ffmpeg failed for {}", input.display());
    }

    let thumbnail = encode_video_thumbnail(input)?;

    Ok((thumbnail, width, height))
}

fn encode_video_thumbnail(path: &Path) -> Result<Vec<u8>> {
    let mut command = Command::new("ffmpeg");
    command.args(["-y"]);

    command.arg("-i").arg(path);

    command.args([
        "-frames:v",
        "1",
        "-vf",
        "scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease",
        "-f",
        "webp",
        "pipe:1",
    ]);

    let output = command.output()?;

    if !output.status.success() {
        eprintln!("{:?}", String::from_utf8_lossy(&output.stderr));
        bail!("ffmpeg command failed");
    }

    Ok(output.stdout)
}

fn encode_audio(input: &Path, output: &Path) -> anyhow::Result<()> {
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

    let status = Command::new("ffmpeg")
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
