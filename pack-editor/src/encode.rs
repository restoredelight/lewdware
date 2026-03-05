use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{self, Command},
    sync::OnceLock,
    thread::{self, available_parallelism},
};

use anyhow::{anyhow, bail, Result};
use dioxus::{
    signals::{ReadSignal, ReadableExt, Signal, SyncSignal, WritableExt, WritableVecExt},
    stores::Store,
};
use futures::{stream, StreamExt};
use image::{imageops::FilterType, ImageFormat, ImageReader};
use infer::MatcherType;
use shared::encode::FileInfo;
use tempfile::NamedTempFile;
use tokio::sync::{oneshot, Semaphore};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{image_list::Media, pack::MediaPack, upload_files::UploadFilesContext, utils::file_name};

pub fn explore_folder(path: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut walkdir = WalkDir::new(&path);

    if !recursive {
        walkdir = walkdir.max_depth(1);
    }

    walkdir
        .into_iter()
        .filter_map(|x| x.ok())
        .filter(|e| {
            let path = e.path();

            if !path.is_file() {
                return false;
            }

            match is_media(path) {
                Ok(x) => x,
                Err(err) => {
                    eprintln!("{err}");
                    false
                }
            }
        })
        .map(|x| x.path().to_path_buf())
        .collect()
}

pub struct ProcessFilesError {
    pub path: PathBuf,
    pub error_type: ProcessFilesErrorType,
}

pub enum ProcessFilesErrorType {
    Skipped,
    EncodeError(anyhow::Error),
    PackError(anyhow::Error),
    OpenError(io::Error),
    HashError(io::Error),
    QueryHashError(anyhow::Error),
    Other(anyhow::Error),
}

impl std::fmt::Display for ProcessFilesErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skipped => write!(f, "Skipped"),
            Self::EncodeError(err) => write!(f, "Error encoding file: {err}"),
            Self::PackError(err) => write!(f, "Error adding file to pack: {err}"),
            Self::OpenError(err) => write!(f, "Error opening file: {err}"),
            Self::HashError(err) => write!(f, "Error computing file hash: {err}"),
            Self::QueryHashError(err) => write!(f, "Error checking file hash: {err}"),
            Self::Other(err) => write!(f, "{err}"),
        }
    }
}

static ENCODE_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();

fn get_encode_semaphore() -> &'static Semaphore {
    ENCODE_SEMAPHORE.get_or_init(|| {
        let permits = available_parallelism()
            .map(|x| (x.get() / 4).max(2))
            .unwrap_or(2);
        Semaphore::new(permits)
    })
}

pub async fn process_files(
    media_pack: ReadSignal<MediaPack>,
    paths: Vec<PathBuf>,
    mut context: UploadFilesContext,
    mut files: Store<Vec<Media>>,
    skip_duplicates: bool,
) {
    let media_ref = media_pack.read();
    let dir = &media_ref.dir();

    // Bound the orchestration (hashing/DB queries) to available parallelism,
    // while the heavy encoding will be further limited by the global semaphore.
    let limit = available_parallelism().map(|x| x.get()).ok();

    stream::iter(paths)
        .for_each_concurrent(
            limit,
            |path| async move {
                context.set_currently_processing(file_name(&path));
                match process_file(
                    media_pack,
                    &path,
                    dir.to_path_buf(),
                    skip_duplicates,
                )
                .await
                {
                    Ok(Some(media)) => {
                        files.push(media);
                    }
                    Ok(None) => {}
                    Err(err) => {
                        context.handle_error(ProcessFilesError {
                            path: path.to_path_buf(),
                            error_type: err
                        });
                    }
                }

                context.increment_processed();
            },
        )
        .await;
}

async fn process_file(
    media_pack: ReadSignal<MediaPack>,
    path: &Path,
    dir: PathBuf,
    skip_duplicates: bool,
) -> Result<Option<Media>, ProcessFilesErrorType> {
    println!("Hashing file");

    let hash = {
        let path = path.to_path_buf();
        match tokio::task::spawn_blocking(move || hash_file(&path)).await {
            Ok(x) => x?,
            Err(err) => {
                if err.is_cancelled() {
                    return Ok(None);
                } else {
                    return Err(ProcessFilesErrorType::HashError(err.into()));
                }
            }
        }
    };

    println!("Hashed");

    if skip_duplicates
        && media_pack
            .read()
            .check_hash(&hash)
            .await
            .map_err(|err| ProcessFilesErrorType::QueryHashError(err))?
    {
        return Err(ProcessFilesErrorType::Skipped);
    }

    // Acquire global semaphore permit before encoding
    let _permit = get_encode_semaphore().acquire().await.map_err(|err| {
        ProcessFilesErrorType::Other(anyhow!("Failed to acquire encode semaphore: {err}"))
    })?;

    let (tx, rx) = oneshot::channel();

    let path_clone = path.to_path_buf();

    let id = Uuid::new_v4();
    let output_path = dir.join("media").join(id.to_string());

    let fun = move || -> anyhow::Result<_> {
        encode_file(&path_clone, &output_path)
    };

    rayon::spawn(move || {
        if tx.is_closed() {
            return;
        }

        let _ = tx.send(fun());
    });

    match rx
        .await
        .map_err(|err| ProcessFilesErrorType::Other(err.into()))?
        .map_err(|err| ProcessFilesErrorType::EncodeError(err))?
    {
        Some(encoded_file) => Ok(Some(
            media_pack
                .read()
                .add_file(encoded_file, path, hash)
                .await
                .map_err(|err| ProcessFilesErrorType::PackError(err))?,
        )),
        None => Ok(None),
    }
}

fn hash_file(path: &Path) -> Result<blake3::Hash, ProcessFilesErrorType> {
    let file = File::open(path).map_err(|err| ProcessFilesErrorType::OpenError(err))?;
    let mut hasher = blake3::Hasher::new();

    hasher
        .update_reader(file)
        .map_err(|err| ProcessFilesErrorType::HashError(err))?;

    Ok(hasher.finalize())
}

fn is_media(path: &Path) -> anyhow::Result<bool> {
    let guess = mime_guess::from_path(path);

    if guess.iter().any(|mime| {
        let mime_type = mime.type_();

        mime_type == mime::IMAGE || mime_type == mime::VIDEO || mime_type == mime::AUDIO
    }) {
        return Ok(true);
    }

    if guess.first().is_some() {
        return Ok(false);
    }

    let better_guess = infer::get_from_path(path)?;
    if let Some(guess) = better_guess {
        let file_type = guess.matcher_type();

        if file_type == MatcherType::Image
            || file_type == MatcherType::Audio
            || file_type == MatcherType::Video
        {
            return Ok(true);
        }
    }

    Ok(false)
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
    pub path: PathBuf,
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
            let (thumb, width, height) = encode_image(input, &output, width, height)?;
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
        path: output,
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

    println!("Decoding file");
    let image = ImageReader::open(path)?.with_guessed_format()?.decode()?;

    println!("Making thumbnail");
    let thumbnail = image.thumbnail(100, 100);
    let webp_encoder = webp::Encoder::from_image(&thumbnail).map_err(|err| anyhow!("{err}"))?;
    let thumbnail_webp = webp_encoder.encode(75.0).to_vec();

    println!("Resizing");
    let image = if image.width() != width as u32 || image.height() != height as u32 {
        image.resize_exact(width as u32, height as u32, FilterType::Lanczos3)
    } else {
        image
    };

    println!("Saving");
    image.save_with_format(output, ImageFormat::Avif)?;

    println!("Done");
    Ok((thumbnail_webp, width, height))
}

fn encode_image(
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
) -> Result<(Vec<u8>, u64, u64)> {
    println!("{}", input.display());
    println!("{}", output.display());

    let (width, height) = resize_dimensions(width, height, MAX_IMAGE_SIZE, true);

    let thumb_temp = NamedTempFile::new()?;
    let thumb_path = thumb_temp.path();

    #[rustfmt::skip]
    let args = [
        "-y",
        "-c:v", "libaom-av1",
        "-cpu-used", "6",
        "-crf", "32",
        "-b:v", "0",
        "-still-picture", "1",
        "-f", "avif",
    ];

    let filter = format!(
        "[0:v]scale=w='{width}':h='{height}',format=yuv420p[main]; \
         [0:v]scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease[thumb]"
    );

    let result = Command::new("ffmpeg")
        .arg("-i")
        .arg(input)
        .arg("-filter_complex")
        .arg(filter)
        .arg("-map")
        .arg("[main]")
        .args(args)
        .arg(output)
        .arg("-map")
        .arg("[thumb]")
        .arg("-frames:v")
        .arg("1")
        .arg("-f")
        .arg("webp")
        .arg(thumb_path)
        .output()?;

    if !result.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&result.stderr));
        eprintln!("Encoding image failed");
        bail!("ffmpeg failed for {}", input.display());
    }

    let mut thumbnail = Vec::new();
    File::open(thumb_path)?.read_to_end(&mut thumbnail)?;

    Ok((thumbnail, width, height))
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
    println!("Audio: {audio}");

    #[rustfmt::skip]
    let args = [
        "-y",
        "-c:v", "libsvtav1",
        "-preset", "8",
        "-svtav1-params", "fast-decode=1"
    ];

    let (width, height) = resize_dimensions(width, height, MAX_VIDEO_SIZE, true);

    let thumb_temp = NamedTempFile::new()?;
    let thumb_path = thumb_temp.path();

    let mut command = Command::new("ffmpeg");

    let filter = format!(
        "[0:v]scale=w='{width}':h='{height}'[main]; \
         [0:v]scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease[thumb]"
    );

    command
        .arg("-i")
        .arg(input)
        .arg("-filter_complex")
        .arg(filter)
        .arg("-map")
        .arg("[main]");

    if audio {
        command.args(["-map", "0:a?", "-c:a", "libopus", "-b:a", "64k"]);
    } else {
        command.arg("-an");
    }

    command.args(args);

    if fixed_frame_rate {
        command.arg("-r").arg("30");
    }

    command
        .arg(output)
        .arg("-map")
        .arg("[thumb]")
        .arg("-frames:v")
        .arg("1")
        .arg("-f")
        .arg("webp")
        .arg(thumb_path);

    let result = command.output()?;

    eprintln!("{}", String::from_utf8_lossy(&result.stdout));
    eprintln!("{}", String::from_utf8_lossy(&result.stderr));

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

    let mut thumbnail = Vec::new();
    File::open(thumb_path)?.read_to_end(&mut thumbnail)?;

    Ok((thumbnail, width, height))
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
