use std::{
    path::{Path, PathBuf},
    thread::available_parallelism,
};

use anyhow::{Context, Result, bail};
use ffmpeg_next::{
    ChannelLayout, Dictionary, Packet, Rational, Rescale, codec, encoder, ffi, format, frame,
    media, picture, util::color,
};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use super::av::{self, Graph, HwDevice, Pull, Rotation, VideoCodec};
use super::hardware::HardwareEncoder;
use super::info::FileInfo;
use super::paths::{get_avifenc_path, new_command};
use super::probe::{ProbedMedia, Streams, extract_container_attribution, probe_file};
use super::worker;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct EncodedFile {
    pub path: PathBuf,
    pub info: FileInfo,
    #[serde(skip)]
    pub thumbnail: Option<Vec<u8>>,
    pub artists: Vec<String>,
    pub source_url: Option<String>,
}

/// Encodes `input` for a pack: AVIF for images, AV1 + Opus in MP4 for video, Opus for audio.
/// `output` is the path without its extension, which follows from what the file turns out to be.
/// `Ok(None)` means `input` isn't media this build can read.
///
/// Runs in the encode worker process when one is set up (`use_encode_worker`), so a decoder that
/// crashes on a hostile file takes down the worker, not the app, and one that hangs is killed
/// once it stops making progress.
pub fn encode_file(
    input: &Path,
    output: &Path,
    encoder: HardwareEncoder,
) -> Result<Option<EncodedFile>> {
    encode_file_cancellable(input, output, encoder, &|| false)
}

/// `encode_file`, stopped early once `cancelled` returns true: the worker is killed and whatever
/// it had written of the output removed, and the encode fails. `cancelled` is polled from the
/// calling thread a few times a second, so it should be cheap -- a channel's `is_closed`, say.
///
/// Without a worker the encode can't be interrupted; `cancelled` is only checked before it starts.
pub fn encode_file_cancellable(
    input: &Path,
    output: &Path,
    encoder: HardwareEncoder,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<EncodedFile>> {
    match worker::configured() {
        Some(exe) => worker::encode(exe, input, output, &encoder, cancelled),
        None if cancelled() => bail!("the encode was cancelled"),
        None => encode_file_in_process(input, output, encoder),
    }
}

pub(super) fn encode_file_in_process(
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
                encode_video(input, &output, width, height, streams, encoder)?;
            thumbnail = thumb;
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

/// The thumbnail branch: the first frame, fitted inside 100x100, as BGRA -- which is what both
/// libwebp (as `RGB32`) and `lossless_webp` take.
const THUMBNAIL: &str = "trim=end_frame=1,\
    scale='min(iw,100)':'min(ih,100)':force_original_aspect_ratio=decrease,format=bgra";

/// The alpha probe: the frame's alpha plane, for `av::any_transparent` to scan. Only wired in
/// for pixel formats that have one (`av::has_alpha`).
const ALPHA: &str = "format=rgba,alphaextract";

fn encode_image(
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
    video: u64,
) -> Result<(Vec<u8>, u64, u64, bool)> {
    let video = video as usize;
    let mut source = av::open_input(input)?;
    let rotation = Rotation::of(&source.stream(video).context("no such stream")?);
    let (width, height) = if rotation.swaps {
        (height, width)
    } else {
        (width, height)
    };
    let (width, height) = resize_dimensions(width, height, 2560, true);

    let frame = av::first_frame(&mut source, video)?;
    let probe_alpha = av::has_alpha(frame.format());
    let r = rotation.filters;
    let main = format!("scale=w={width}:h={height},format=rgba");
    let (spec, sinks) = if probe_alpha {
        (
            format!(
                "[in]{r}split=3[a][b][c];[a]{main}[main];[b]{THUMBNAIL}[thumb];[c]{ALPHA}[alpha]"
            ),
            &["main", "thumb", "alpha"][..],
        )
    } else {
        (
            format!("[in]{r}split=2[a][b];[a]{main}[main];[b]{THUMBNAIL}[thumb]"),
            &["main", "thumb"][..],
        )
    };
    let mut frames = av::filter_frame(&frame, &spec, sinks)?.into_iter();
    let (main, thumb) = (frames.next().unwrap(), frames.next().unwrap());
    let transparent = frames
        .next()
        .is_some_and(|alpha| av::any_transparent(&alpha));

    // ffmpeg's libaom-av1 wrapper can't produce a working alpha auxiliary image for AVIF: it
    // silently drops a yuva420p input (falls back to yuv420p), and a manual dual-stream encode
    // hits an upstream libaom bitstream-conformance rejection (monochrome + identity-matrix
    // output requires 4:4:4, but ffmpeg's wrapper still configures 4:2:0 for it). So this only
    // resizes to a lossless intermediate PNG (keeping any real alpha) plus the thumbnail;
    // `avifenc` below -- which sets up the alpha-plane encode correctly itself, and is faster
    // than ffmpeg's wrapper besides -- does the actual AVIF encode.
    //
    // This PNG is a throwaway intermediate that avifenc immediately re-encodes, so its own size
    // doesn't matter -- only write speed does. zlib-compressing it is pure waste: on a 2560x1440
    // frame this alone was ~60% of the old ffmpeg step, for a file avifenc reads once and
    // discards.
    let png_encoder = av::still_encoder("png", &main, &[("pred", "none")], |encoder| {
        encoder.set_compression(Some(0));
    })?;
    let png = av::encode_still(png_encoder, main)?;
    let main_temp = NamedTempFile::with_suffix(".png")?;
    std::fs::write(main_temp.path(), png)?;

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
        .arg(main_temp.path())
        .arg(output)
        .output()?;

    if !avifenc_result.status.success() {
        tracing::error!("{}", String::from_utf8_lossy(&avifenc_result.stderr));
        bail!("avifenc failed for {}", input.display());
    }

    Ok((thumbnail_webp(thumb)?, width, height, transparent))
}

/// A thumbnail as WebP: lossy through libwebp when the linked ffmpeg has it, else lossless
/// through the `image` crate, which is ~3x the size but needs no native library.
fn thumbnail_webp(frame: frame::Video) -> Result<Vec<u8>> {
    let lossy = av::still_encoder("libwebp", &frame, &[], |_| {})
        .and_then(|encoder| av::encode_still(encoder, frame.clone()));
    match lossy {
        Ok(bytes) => Ok(bytes),
        Err(error) => {
            tracing::debug!("falling back to lossless WebP: {error:#}");
            lossless_webp(&frame)
        }
    }
}

fn lossless_webp(bgra: &frame::Video) -> Result<Vec<u8>> {
    let (width, height) = (bgra.width() as usize, bgra.height() as usize);
    let stride = bgra.stride(0);
    let data = bgra.data(0);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        for px in data[row * stride..row * stride + width * 4]
            .as_chunks::<4>()
            .0
        {
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out).encode(
        &rgba,
        width as u32,
        height as u32,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(out)
}

fn encode_video(
    input: &Path,
    output: &Path,
    width: u64,
    height: u64,
    streams: Streams,
    encoder: HardwareEncoder,
) -> Result<(Option<Vec<u8>>, u64, u64, bool)> {
    let opaque = |encoder: &HardwareEncoder, fixed_fps| {
        transcode_video(input, output, streams, &|facts| {
            plan_opaque(facts, width, height, encoder, fixed_fps)
        })
    };
    let transparent = |fixed_fps| {
        transcode_video(input, output, streams, &|facts| {
            plan_transparent(facts, width, height, fixed_fps)
        })
    };

    // The fixed-rate retry is for sources whose timestamps the encoder or muxer won't take, and
    // goes through software in case it was the hardware encoder that refused.
    let first = match opaque(&encoder, false) {
        Ok(result) => Ok(result),
        Err(error) => {
            tracing::error!(
                "encoding {} failed, retrying at a fixed 30 fps: {error:#}",
                input.display()
            );
            opaque(&HardwareEncoder::SoftwareFallback, true)
        }
    };
    let (thumbnail, width, height) = match first? {
        Transcoded::Done {
            thumbnail,
            width,
            height,
        } => return Ok((thumbnail, width, height, false)),
        Transcoded::Transparent => {
            let _ = std::fs::remove_file(output);
            match transparent(false) {
                Ok(result) => result,
                Err(error) => {
                    tracing::error!(
                        "encoding {} failed, retrying at a fixed 30 fps: {error:#}",
                        input.display()
                    );
                    transparent(true)?
                }
            }
            .done()?
        }
    };
    // The packed frame is colour over alpha, twice the picture's height.
    Ok((thumbnail, width, height / 2, true))
}

/// Lays out the filter graph and picks the encoder once the source's facts are known.
type Plan<'a> = &'a dyn Fn(&VideoFacts) -> Result<VideoPlan>;

/// What a video plan has to know about its source before it can lay out the filter graph.
struct VideoFacts {
    rotation: Rotation,
    /// The source's pixel format can carry transparency, so the graph needs the alpha probe.
    alpha: bool,
}

struct VideoPlan {
    /// Filters from `[in]` to `[main]` (what's encoded), `[thumb]` and, if `probe_alpha`,
    /// `[alpha]`.
    spec: String,
    probe_alpha: bool,
    codec: VideoCodec,
    device: Option<HwDevice>,
}

fn fitted(facts: &VideoFacts, width: u64, height: u64) -> (u64, u64) {
    let (width, height) = if facts.rotation.swaps {
        (height, width)
    } else {
        (width, height)
    };
    resize_dimensions(width, height, 1280, true)
}

fn fps(fixed: bool) -> &'static str {
    if fixed { ",fps=30" } else { "" }
}

/// Every video starts here, encoded as-is with the probe watching for transparency; one that
/// turns out to have any goes again through `plan_transparent`.
fn plan_opaque(
    facts: &VideoFacts,
    width: u64,
    height: u64,
    encoder: &HardwareEncoder,
    fixed_fps: bool,
) -> Result<VideoPlan> {
    let (w, h) = fitted(facts, width, height);
    let r = facts.rotation.filters;
    let main = format!(
        "scale=w={w}:h={h}{},{}",
        fps(fixed_fps),
        encoder.input_filter()
    );
    let spec = if facts.alpha {
        format!("[in]{r}split=3[a][b][c];[a]{main}[main];[b]{THUMBNAIL}[thumb];[c]{ALPHA}[alpha]")
    } else {
        format!("[in]{r}split=2[a][b];[a]{main}[main];[b]{THUMBNAIL}[thumb]")
    };
    Ok(VideoPlan {
        spec,
        probe_alpha: facts.alpha,
        codec: encoder.codec(),
        device: encoder.device()?,
    })
}

/// Packs colour (top) and alpha-as-luma (bottom) into a single frame twice the picture's height.
/// Both halves are full range, so the engine's shader can read alpha directly (0 transparent,
/// 1 opaque). Always software: the hardware encoders' handling of full-range input is
/// untested, and transparent videos are rare enough not to need the speed.
fn plan_transparent(
    facts: &VideoFacts,
    width: u64,
    height: u64,
    fixed_fps: bool,
) -> Result<VideoPlan> {
    let (w, h) = fitted(facts, width, height);
    let r = facts.rotation.filters;
    let spec = format!(
        "[in]{r}split=3[a][b][t];\
         [a]scale=w={w}:h={h}:out_range=pc,format=yuv420p[color];\
         [b]scale=w={w}:h={h},{ALPHA},scale=out_range=pc,format=yuv420p[alpha_yuv];\
         [color][alpha_yuv]vstack=inputs=2{}[main];\
         [t]{THUMBNAIL}[thumb]",
        fps(fixed_fps)
    );
    let mut codec = HardwareEncoder::SoftwareFallback.codec();
    codec.range = Some(color::Range::JPEG);
    Ok(VideoPlan {
        spec,
        probe_alpha: false,
        codec,
        device: None,
    })
}

enum Transcoded {
    Done {
        thumbnail: Option<Vec<u8>>,
        width: u64,
        height: u64,
    },
    /// The alpha probe found a transparent pixel, so the encode was abandoned part-way for the
    /// packed-alpha one.
    Transparent,
}

impl Transcoded {
    fn done(self) -> Result<(Option<Vec<u8>>, u64, u64)> {
        match self {
            Self::Done {
                thumbnail,
                width,
                height,
            } => Ok((thumbnail, width, height)),
            Self::Transparent => bail!("the transparent encode asked to be made transparent"),
        }
    }
}

/// Moves the MP4 `moov` atom in front of the media data, at the cost of a rewrite pass once the
/// encode finishes.
///
/// Without it the muxer leaves `moov` at the end of the file, which makes the first bytes of a
/// video useless on their own: nothing can be decoded until the tail arrives. A player reading
/// over HTTP reads from the front and starts as soon as it can -- with `moov` last, WebKitGTK's
/// decoder reported "no playable streams" rather than going back for the end of the file. With
/// `moov` first, the opening bytes always carry the header and playback starts immediately,
/// whatever the file's size.
const FASTSTART: (&str, &str) = ("movflags", "+faststart");

fn transcode_video(
    input: &Path,
    output: &Path,
    streams: Streams,
    plan: Plan,
) -> Result<Transcoded> {
    transcode(
        input,
        output,
        "mp4",
        Some((streams.video as usize, plan)),
        streams.audio,
    )
}

fn encode_audio(input: &Path, output: &Path, audio: u64) -> Result<()> {
    // The probed stream, not whichever the demuxer calls best: see `ProbedMedia`.
    transcode(input, output, "opus", None, Some(audio))?;
    Ok(())
}

/// Demuxes `input`, re-encodes the one video and/or one audio stream asked for, and muxes them
/// into `output` as `container`.
fn transcode(
    input: &Path,
    output: &Path,
    container: &str,
    video: Option<(usize, Plan)>,
    audio: Option<u64>,
) -> Result<Transcoded> {
    let mut source = av::open_input(input)?;
    let mut muxer = Muxer::new(output, container)?;

    // Video first, so it's the output's first stream.
    let mut video = video
        .map(|(index, plan)| VideoLane::open(&source, index, plan, &mut muxer))
        .transpose()?;
    let mut audio = audio
        .map(|index| AudioLane::open(&source, index as usize, &mut muxer))
        .transpose()?;
    if video.is_none() {
        muxer.start()?;
    }

    while let Some(packet) = av::read_packet(&mut source) {
        let index = packet.stream();
        if let Some(lane) = video.as_mut().filter(|lane| lane.index == index) {
            if let Flow::Transparent = lane.decode(Some(&packet), &mut muxer)? {
                return Ok(Transcoded::Transparent);
            }
        } else if let Some(lane) = audio.as_mut().filter(|lane| lane.index == index) {
            lane.decode(Some(&packet), &mut muxer)?;
        }
    }
    if let Some(lane) = video.as_mut()
        && let Flow::Transparent = lane.decode(None, &mut muxer)?
    {
        return Ok(Transcoded::Transparent);
    }
    if let Some(lane) = audio.as_mut() {
        lane.decode(None, &mut muxer)?;
    }

    let (thumbnail, width, height) = match video {
        Some(lane) => {
            let encoder = lane.encoder.context("no video frames could be decoded")?;
            let thumbnail = lane.thumbnail.map(thumbnail_webp).transpose()?;
            (thumbnail, encoder.width() as u64, encoder.height() as u64)
        }
        None => (None, 0, 0),
    };
    muxer.finish()?;
    Ok(Transcoded::Done {
        thumbnail,
        width,
        height,
    })
}

#[derive(PartialEq)]
enum Flow {
    Continue,
    Transparent,
}

/// The output file.
///
/// Its header can only be written once every stream's parameters are known, and the video
/// stream's aren't until its first frame is filtered -- that's when its encoder opens (see
/// `av::open_video_encoder`) -- so packets that are ready before then, the audio's, wait here.
struct Muxer {
    context: format::context::Output,
    container: String,
    path: PathBuf,
    started: bool,
    queued: Vec<(Packet, Rational)>,
}

impl Muxer {
    fn new(path: &Path, container: &str) -> Result<Self> {
        let context = av::create_output(path, container)?;
        Ok(Self {
            context,
            container: container.to_owned(),
            path: path.to_owned(),
            started: false,
            queued: Vec::new(),
        })
    }

    fn global_header(&self) -> bool {
        self.context
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER)
    }

    /// Writes the header, once every stream's parameters are in, and then anything queued.
    fn start(&mut self) -> Result<()> {
        let mut options = Dictionary::new();
        if self.container == "mp4" {
            options.set(FASTSTART.0, FASTSTART.1);
        }
        self.context
            .write_header_with(options)
            .with_context(|| format!("writing {}", self.path.display()))?;
        self.started = true;
        for (packet, time_base) in std::mem::take(&mut self.queued) {
            self.write(packet, time_base)?;
        }
        Ok(())
    }

    /// Writes a packet whose timestamps are in `time_base`, the encoder's.
    fn write(&mut self, mut packet: Packet, time_base: Rational) -> Result<()> {
        worker::tick();
        if !self.started {
            self.queued.push((packet, time_base));
            return Ok(());
        }
        // The muxer picks its streams' time bases while writing the header.
        let stream_time_base = self.context.stream(packet.stream()).unwrap().time_base();
        packet.rescale_ts(time_base, stream_time_base);
        packet.write_interleaved(&mut self.context)?;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        anyhow::ensure!(self.started, "nothing was encoded");
        self.context
            .write_trailer()
            .with_context(|| format!("finishing {}", self.path.display()))
    }
}

/// Everything ready from `encoder`, into its stream in the muxer.
fn drain(
    encoder: &mut encoder::Encoder,
    stream: usize,
    time_base: Rational,
    muxer: &mut Muxer,
) -> Result<()> {
    let mut packet = Packet::empty();
    while let Pull::Ready = av::receive_packet(encoder, &mut packet)? {
        packet.set_stream(stream);
        muxer.write(std::mem::replace(&mut packet, Packet::empty()), time_base)?;
    }
    Ok(())
}

/// Fills in an output stream from its opened encoder.
fn set_stream_parameters(
    muxer: &mut Muxer,
    stream: usize,
    encoder: &codec::context::Context,
) -> Result<Rational> {
    let time_base = unsafe { Rational::from((*encoder.as_ptr()).time_base) };
    let mut out = muxer.context.stream_mut(stream).unwrap();
    unsafe {
        let ret =
            ffi::avcodec_parameters_from_context((*out.as_mut_ptr()).codecpar, encoder.as_ptr());
        if ret < 0 {
            bail!(
                "copying encoder parameters: {}",
                ffmpeg_next::Error::from(ret)
            );
        }
    }
    out.set_time_base(time_base);
    Ok(time_base)
}

struct VideoLane {
    index: usize,
    decoder: codec::decoder::Video,
    graph: Graph,
    codec: VideoCodec,
    frame_rate: Rational,
    /// Opened on the first frame out of `[main]`.
    encoder: Option<encoder::Video>,
    stream: usize,
    time_base: Rational,
    probe_alpha: bool,
    thumbnail: Option<frame::Video>,
    last_pts: Option<i64>,
    decoded: frame::Video,
    filtered: frame::Video,
    // Held for the graph's and the encoder's lifetime, which reference it.
    _device: Option<HwDevice>,
}

impl VideoLane {
    fn open(
        source: &format::context::Input,
        index: usize,
        plan: Plan,
        muxer: &mut Muxer,
    ) -> Result<Self> {
        let stream = source
            .stream(index)
            .with_context(|| format!("no stream {index}"))?;
        let decoder = av::video_decoder(&stream)?;
        let facts = VideoFacts {
            rotation: Rotation::of(&stream),
            alpha: av::has_alpha(decoder.format()),
        };
        let VideoPlan {
            spec,
            probe_alpha,
            codec,
            device,
        } = plan(&facts)?;

        let frame_rate = av::frame_rate_hint(&stream);
        let args = av::video_source_args(&decoder, stream.time_base(), frame_rate)?;
        let sinks: &[&str] = if probe_alpha {
            &["main", "thumb", "alpha"]
        } else {
            &["main", "thumb"]
        };
        let graph = Graph::new(Some(("buffer", &args)), &spec, sinks, device.as_ref())?;
        let out = muxer
            .context
            .add_stream(encoder::find_by_name(codec.name))
            .context("adding the video stream")?
            .index();

        Ok(Self {
            index,
            decoder,
            graph,
            codec,
            frame_rate,
            encoder: None,
            stream: out,
            time_base: Rational::new(1, 1),
            probe_alpha,
            thumbnail: None,
            last_pts: None,
            decoded: frame::Video::empty(),
            filtered: frame::Video::empty(),
            _device: device,
        })
    }

    /// Decodes `packet` (or, for `None`, flushes everything through to the muxer).
    fn decode(&mut self, packet: Option<&Packet>, muxer: &mut Muxer) -> Result<Flow> {
        match packet {
            Some(packet) => av::send_packet(&mut self.decoder, packet),
            None => self.decoder.send_eof()?,
        }
        let mut skipped = 0;
        loop {
            match av::receive_frame(&mut self.decoder, &mut self.decoded)? {
                Pull::Ready => {
                    let timestamp = self.decoded.timestamp();
                    self.decoded.set_pts(timestamp);
                    self.graph.push(&self.decoded)?;
                    if let Flow::Transparent = self.filter(muxer)? {
                        return Ok(Flow::Transparent);
                    }
                }
                Pull::Skipped if skipped < av::MAX_SKIPPED => skipped += 1,
                _ => break,
            }
        }
        if packet.is_none() {
            self.graph.close()?;
            if let Flow::Transparent = self.filter(muxer)? {
                return Ok(Flow::Transparent);
            }
            if let Some(encoder) = self.encoder.as_mut() {
                encoder.send_eof()?;
                drain(encoder, self.stream, self.time_base, muxer)?;
            }
        }
        Ok(Flow::Continue)
    }

    /// Takes everything the graph has ready out of each sink. Every sink has to be drained every
    /// time, or frames pile up in the ones nobody reads.
    fn filter(&mut self, muxer: &mut Muxer) -> Result<Flow> {
        // The probe goes first, so a transparent video is caught before more is encoded.
        if self.probe_alpha {
            while let Pull::Ready = self.graph.pull("alpha", &mut self.filtered)? {
                if av::any_transparent(&self.filtered) {
                    return Ok(Flow::Transparent);
                }
            }
        }
        while let Pull::Ready = self.graph.pull("thumb", &mut self.filtered)? {
            if self.thumbnail.is_none() {
                self.thumbnail = Some(self.filtered.clone());
            }
        }
        while let Pull::Ready = self.graph.pull("main", &mut self.filtered)? {
            // An encoder needs strictly increasing timestamps. A frame that repeats or goes back
            // on the last one is a duplicate, and dropped, as the CLI's variable-rate MP4 output
            // did; one with no timestamp at all follows on from the last.
            let pts = match (self.filtered.pts(), self.last_pts) {
                (Some(pts), Some(last)) if pts <= last => continue,
                (Some(pts), _) => pts,
                (None, last) => last.map_or(0, |last| last + 1),
            };
            self.last_pts = Some(pts);
            self.filtered.set_pts(Some(pts));
            // Leave keyframe placement to the encoder: the decoder's picture types would
            // otherwise ride through and force a keyframe wherever the source had one.
            self.filtered.set_kind(picture::Type::None);

            let encoder = match &mut self.encoder {
                Some(encoder) => encoder,
                None => {
                    let encoder = av::open_video_encoder(
                        &mut self.graph,
                        "main",
                        &self.filtered,
                        &self.codec,
                        self.frame_rate,
                        muxer.global_header(),
                    )?;
                    self.time_base = set_stream_parameters(muxer, self.stream, &encoder)?;
                    muxer.start()?;
                    self.encoder.insert(encoder)
                }
            };
            encoder.send_frame(&self.filtered)?;
            drain(encoder, self.stream, self.time_base, muxer)?;
        }
        Ok(Flow::Continue)
    }
}

/// Opus has no use for more than stereo in a popup, and libopus can't take every layout a source
/// might have, so everything is mono or stereo.
const OPUS_BITRATE: usize = 64_000;
const OPUS_RATE: i32 = 48_000;

struct AudioLane {
    index: usize,
    decoder: codec::decoder::Audio,
    graph: Graph,
    encoder: encoder::Audio,
    stream: usize,
    time_base: Rational,
    graph_time_base: Rational,
    decoded: frame::Audio,
    filtered: frame::Audio,
}

impl AudioLane {
    fn open(source: &format::context::Input, index: usize, muxer: &mut Muxer) -> Result<Self> {
        let stream = source
            .stream(index)
            .with_context(|| format!("no stream {index}"))?;
        let decoder = av::audio_decoder(&stream)?;
        let (layout, channels) = av::channel_layout_name(&decoder)?;
        let (out_layout, out_name) = if channels == 1 {
            (ChannelLayout::MONO, "mono")
        } else {
            (ChannelLayout::STEREO, "stereo")
        };

        let found =
            encoder::find_by_name("libopus").context("this build has no libopus encoder")?;
        let mut encoder = codec::context::Context::new_with_codec(found)
            .encoder()
            .audio()?;
        encoder.set_rate(OPUS_RATE);
        encoder.set_channel_layout(out_layout);
        encoder.set_format(format::Sample::F32(format::sample::Type::Packed));
        encoder.set_bit_rate(OPUS_BITRATE);
        encoder.set_time_base(Rational::new(1, OPUS_RATE));
        if muxer.global_header() {
            encoder.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let encoder = encoder
            .open_with(Dictionary::new())
            .context("opening the libopus encoder")?;

        let time_base = stream.time_base();
        let args = format!(
            "time_base={}/{}:sample_rate={}:sample_fmt={}:channel_layout={layout}",
            time_base.numerator(),
            time_base.denominator(),
            decoder.rate(),
            decoder.format().name(),
        );
        let spec = format!(
            "[in]aresample={OPUS_RATE},aformat=sample_fmts=flt:channel_layouts={out_name}[out]"
        );
        let mut graph = Graph::new(Some(("abuffer", &args)), &spec, &["out"], None)?;
        if encoder.frame_size() > 0 {
            graph.set_frame_size("out", encoder.frame_size());
        }
        let graph_time_base = graph.sink_time_base("out");
        let stream = muxer
            .context
            .add_stream(found)
            .context("adding the audio stream")?
            .index();
        let time_base = set_stream_parameters(muxer, stream, &encoder)?;

        Ok(Self {
            index,
            decoder,
            graph,
            encoder,
            stream,
            time_base,
            graph_time_base,
            decoded: frame::Audio::empty(),
            filtered: frame::Audio::empty(),
        })
    }

    fn decode(&mut self, packet: Option<&Packet>, muxer: &mut Muxer) -> Result<()> {
        match packet {
            Some(packet) => av::send_packet(&mut self.decoder, packet),
            None => self.decoder.send_eof()?,
        }
        let mut skipped = 0;
        loop {
            match av::receive_frame(&mut self.decoder, &mut self.decoded)? {
                Pull::Ready => {
                    let timestamp = self.decoded.timestamp();
                    self.decoded.set_pts(timestamp);
                    self.graph.push(&self.decoded)?;
                    self.filter(muxer)?;
                }
                Pull::Skipped if skipped < av::MAX_SKIPPED => skipped += 1,
                _ => break,
            }
        }
        if packet.is_none() {
            self.graph.close()?;
            self.filter(muxer)?;
            self.encoder.send_eof()?;
            drain(&mut self.encoder, self.stream, self.time_base, muxer)?;
        }
        Ok(())
    }

    fn filter(&mut self, muxer: &mut Muxer) -> Result<()> {
        while let Pull::Ready = self.graph.pull("out", &mut self.filtered)? {
            let pts = self
                .filtered
                .pts()
                .map(|pts| pts.rescale(self.graph_time_base, self.time_base));
            self.filtered.set_pts(pts);
            self.encoder.send_frame(&self.filtered)?;
            drain(&mut self.encoder, self.stream, self.time_base, muxer)?;
        }
        Ok(())
    }
}

/// A JPEG of `path`'s first picture, fitted inside 800x600, for the pack editor's inspector.
///
/// Sized for the inspector preview, the largest consumer: it grows with the inspector, up to
/// 420x262 CSS px, which is twice that on a HiDPI display. `min()` keeps this a cap rather than
/// a target -- a small source is served at its own size instead of being upscaled here.
///
/// `packed_alpha` is for a transparent video, which is packed as colour on top and alpha below
/// (see `plan_transparent`): only the top half is the picture. Not for a transparent image,
/// which keeps a real alpha plane.
pub fn preview_jpeg(path: &Path, packed_alpha: bool) -> Result<Vec<u8>> {
    let mut input = av::open_input(path)?;
    let stream = input
        .streams()
        .best(media::Type::Video)
        .context("no picture to preview")?
        .index();
    let frame = av::first_frame(&mut input, stream)?;
    let crop = if packed_alpha {
        "crop=iw:ih/2:0:0,"
    } else {
        ""
    };
    let spec = format!(
        "[in]{crop}scale='min(iw,800)':'min(ih,600)':force_original_aspect_ratio=decrease\
         :out_range=pc,format=yuv420p[out]"
    );
    let picture = av::filter_frame(&frame, &spec, &["out"])?.remove(0);
    let encoder = av::still_encoder("mjpeg", &picture, &[], |encoder| {
        encoder.set_color_range(color::Range::JPEG);
        // `-q:v 4`: a fixed quantiser rather than a bitrate.
        encoder.set_flags(codec::Flags::QSCALE);
        encoder.set_global_quality(4 * ffi::FF_QP2LAMBDA);
    })?;
    av::encode_still(encoder, picture)
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
