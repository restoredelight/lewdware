use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::Stdio,
    thread::available_parallelism,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use super::hardware::HardwareEncoder;
use super::info::FileInfo;
use super::paths::{get_avifenc_path, get_ffmpeg_path, new_command};
use super::probe::{ProbedMedia, Streams, extract_container_attribution, probe_file};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct EncodedFile {
    pub path: PathBuf,
    pub info: FileInfo,
    #[serde(skip)]
    pub thumbnail: Option<Vec<u8>>,
    pub artists: Vec<String>,
    pub source_url: Option<String>,
}

pub fn encode_file(
    input: &Path,
    output: &Path,
    encoder: HardwareEncoder,
) -> Result<Option<EncodedFile>> {
    let (probed, probe_json) = match probe_file(input)? {
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

/// Moves the MP4 `moov` atom in front of the media data, at the cost of a rewrite pass once the
/// encode finishes.
///
/// Without it ffmpeg leaves `moov` at the end of the file, which makes the first bytes of a video
/// useless on their own: nothing can be decoded until the tail arrives. The pack editor previews
/// media over HTTP, where a player reads from the front and starts as soon as it can -- with
/// `moov` last, WebKitGTK's decoder reported "no playable streams" rather than going back for the
/// end of the file. With `moov` first, the opening bytes always carry the header and playback
/// starts immediately, whatever the file's size.
const FASTSTART: [&str; 2] = ["-movflags", "+faststart"];

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
    //
    // Only `[main]` is handed to the encoder, so only it takes the encoder's input filter; the
    // thumbnail and the alpha probe stay in software, where the rest of this function reads them.
    let main_input = encoder.input_filter();
    let filter = format!(
        "[0:{video}]scale=w='{width}':h='{height}',{main_input}[main]; \
         [0:{video}]scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease[thumb]; \
         [0:{video}]format=rgba,alphaextract,format=gray,signalstats,metadata=print:key=lavfi.signalstats.YMIN[alpha]"
    );

    let mut cmd = new_command(get_ffmpeg_path());
    cmd.arg("-y")
        .args(encoder.init_args())
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

    cmd.args(encoder.ffmpeg_args())
        .args(["-f", "mp4"])
        .args(FASTSTART);

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

    command
        .args([
            "-c:v",
            "libx264",
            "-crf",
            "23",
            "-color_range",
            "pc",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(FASTSTART);

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
