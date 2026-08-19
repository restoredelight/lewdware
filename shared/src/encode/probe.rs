use std::path::Path;

use anyhow::Result;

use crate::attribution::ExtractedAttribution;

use super::info::FileInfo;
use super::paths::{get_ffprobe_path, new_command};

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
pub struct Streams {
    pub video: u64,
    pub audio: Option<u64>,
}

pub fn file_info(path: &Path) -> Result<Option<(ProbedMedia, serde_json::Value)>> {
    let args = [
        "-v",
        "error",
        "-count_frames",
        "-show_entries",
        "stream=index,codec_name,codec_type,nb_read_frames,nb_frames,width,height,coded_width,\
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
pub fn extract_container_attribution(json: &serde_json::Value) -> ExtractedAttribution {
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

pub fn probe_file(path: &Path) -> Result<Option<(ProbedMedia, serde_json::Value)>> {
    file_info(path)
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

/// The stream's own index within the container. ffprobe lists streams in index order, so the
/// array position is an accurate fallback for the rare stream that reports no index.
fn stream_index(stream: &serde_json::Value, position: usize) -> u64 {
    stream_number(stream, "index").unwrap_or(position as u64)
}

pub(crate) fn parse_media_info(json: &serde_json::Value) -> Option<ProbedMedia> {
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
