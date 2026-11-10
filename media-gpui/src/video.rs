//! Video decoding and pacing, adapted from `lewdware/src/video.rs`.
//!
//! The pacing half (`VideoDecoder::next_frame` and everything it calls: audio-master with frame
//! dropping and audio pausing, the wall-clock fallback when the audio clock stalls, pause/resume
//! re-anchoring, holding the last frame for its duration) is the engine's logic, copied as is.
//!
//! What changed (see README.md, "Reuse"):
//!
//! * **No hardware decode / zero-copy.** `try_hw_setup`, `HardwareFrame` and the wgpu device are
//!   gone: gpui owns its renderer and takes CPU images only.
//! * **Frames are converted on the decode thread** to BGRA `gpui::RenderImage`s (`convert.rs`),
//!   optionally downscaled to the size they are displayed at (`target_size`). The engine hands
//!   YUV planes to a shader on the render thread instead.
//! * **No frame recycling.** The engine swaps decoded `Video` buffers back to the decoder. A
//!   `RenderImage` owns its pixels and can't give them back, so each frame is a fresh allocation.
//! * **Start offset (seeking)**: seek the input, then discard (unconverted) frames before it.
//! * **Decoder threads are configurable** (engine: always 1; it may run many videos at once).
//! * Cues dropped; a few counters added for the spike's measurements.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use ffmpeg::codec;
use ffmpeg_next::{self as ffmpeg, frame::Video};
use gpui_kit::RenderImage;
use smallvec::smallvec;

use crate::{
    audio::{AudioOut, AudioPlayer},
    convert::Converter,
    source::MediaSource,
};

pub struct VideoFrame {
    /// Monotonic within one decoder, including loop seams; independent of PTS spacing.
    pub sequence: u64,
    pub image: Arc<RenderImage>,
    pub pts: Duration,
    pub duration: Duration,
    #[allow(dead_code)]
    pub pass: u64,
    pub pass_pts: Duration,
}

/// What the decode thread measured, shared with the UI for the stats line.
#[derive(Default)]
pub struct DecodeStats {
    pub error: std::sync::Mutex<Option<String>>,
    pub finished: AtomicBool,
    /// Sum of microseconds spent in swscale + building the RenderImage.
    pub convert_us: AtomicU64,
    pub converted: AtomicU64,
    /// Sum of microseconds spent in libavcodec (send_packet + receive_frame).
    pub decode_us: AtomicU64,
}

pub struct VideoDecoder {
    rx: Receiver<VideoFrame>,
    audio_player: Option<AudioPlayer>,
    tolerance: Duration,
    clock_origin: Option<Instant>,
    paused_at: Option<Instant>,
    pending: Option<VideoFrame>,
    /// ADDED: the decoder pass playing. The decoder always runs on into the next pass; whether
    /// playback follows it is decided at the seam (`at_seam`), from the loop flag as it is then.
    pass: u64,
    current_frame_expiry: Option<Instant>,
    audio_clock: Option<(Duration, Instant)>,
    audio_stalled: bool,
    video_clock: Duration,
    pub native_width: u32,
    pub native_height: u32,
    pub duration: Option<Duration>,
    pub nominal_frame: Duration,
    paused: bool,
    pub lag_count: u32,
    /// Frames thrown away by the audio-master path for being too far behind the audio.
    pub skipped_late: u32,
    loop_video: Arc<AtomicBool>,
    pub stats: Arc<DecodeStats>,
    stop: Arc<AtomicBool>,
}

pub struct Options {
    pub source: MediaSource,
    pub packed_alpha: bool,
    pub volume: f32,
    pub start: Duration,
    pub threads: usize,
    /// Display size in device pixels, `w << 32 | h`; 0 means "decode at native size".
    pub target_size: Arc<AtomicU64>,
}

impl VideoDecoder {
    pub fn new(
        opts: Options,
        loop_video: Arc<AtomicBool>,
        audio_out: Option<&AudioOut>,
    ) -> Result<Self> {
        let stats = Arc::new(DecodeStats::default());
        let stop = Arc::new(AtomicBool::new(false));
        let VideoStream { rx, meta } =
            spawn_video_stream(&opts, stats.clone(), stop.clone())?;

        let audio_player = match audio_out {
            Some(out) => match AudioPlayer::new(
                out,
                opts.source.clone(),
                loop_video.clone(),
                opts.volume,
                opts.start,
            ) {
                Ok(player) => player,
                Err(err) => {
                    tracing::error!("audio: {err:#}");
                    None
                }
            },
            None => None,
        };

        Ok(Self {
            rx,
            audio_player,
            tolerance: Duration::from_millis(200),
            clock_origin: None,
            paused_at: None,
            pending: None,
            pass: 0,
            current_frame_expiry: None,
            audio_clock: None,
            audio_stalled: false,
            video_clock: opts.start,
            native_width: meta.native_width,
            native_height: meta.native_height,
            duration: meta.duration,
            nominal_frame: meta.nominal_frame,
            paused: true,
            lag_count: 0,
            skipped_late: 0,
            loop_video,
            stats,
            stop,
        })
    }

    pub fn has_audio(&self) -> bool {
        self.audio_player.is_some()
    }

    /// The clock frames are being paced against right now, for measuring lateness: the audio
    /// position on the audio path, the wall clock mapped onto media time otherwise.
    pub fn reference_clock(&self) -> Option<Duration> {
        if let Some(player) = &self.audio_player
            && !self.audio_stalled
        {
            return Some(player.position());
        }
        let origin = self.clock_origin?;
        let now = self.paused_at.unwrap_or_else(Instant::now);
        Some(now.saturating_duration_since(origin))
    }

    /// Take a frame regardless of pacing or pause state: used to show the first frame after
    /// opening or seeking while paused.
    pub fn take_preview(&mut self) -> Option<VideoFrame> {
        let frame = self.pending.take().or_else(|| self.rx.try_recv().ok())?;
        self.video_clock = frame.pts;
        Some(frame)
    }

    pub fn next_frame(&mut self) -> NextFrame {
        if self.paused {
            return NextFrame::None;
        }
        if self.audio_player.is_some() && !self.audio_stalled {
            self.next_frame_audio_master()
        } else {
            self.next_frame_clock_master()
        }
    }

    // ---- From here to `set_loop`: the engine's pacing, unchanged except where marked. ----

    fn next_frame_clock_master(&mut self) -> NextFrame {
        let frame = match self.pending.take() {
            Some(frame) => frame,
            None => match self.recv_frame() {
                RecvFrame::Frame(frame) => frame,
                RecvFrame::Waiting => return NextFrame::None,
                RecvFrame::Finished => return self.finish_once_the_last_frame_is_done(),
            },
        };
        let Some(frame) = self.at_seam(frame) else {
            return self.finish_once_the_last_frame_is_done();
        };

        let now = Instant::now();
        let origin = *self
            .clock_origin
            .get_or_insert_with(|| now.checked_sub(frame.pts).unwrap_or(now));
        let deadline = origin + frame.pts;

        if now < deadline {
            self.pending = Some(frame);
            return NextFrame::None;
        }

        if now - deadline > self.tolerance {
            self.clock_origin = Some(now.checked_sub(frame.pts).unwrap_or(now));
        }

        self.video_clock = frame.pts;
        self.current_frame_expiry = Some(deadline.max(now) + frame.duration);

        NextFrame::Ready(frame)
    }

    /// ADDED: lets `frame` through unless it starts a pass that playback shouldn't follow into
    /// (loop off); then it's held back and the video finishes. Held, so turning loop on before
    /// the last frame is done still carries on.
    fn at_seam(&mut self, frame: VideoFrame) -> Option<VideoFrame> {
        if frame.pass != self.pass {
            if !self.loop_video.load(Ordering::Relaxed) {
                self.pending = Some(frame);
                return None;
            }
            self.pass = frame.pass;
        }
        Some(frame)
    }

    fn finish_once_the_last_frame_is_done(&self) -> NextFrame {
        match self.current_frame_expiry {
            Some(expiry) if Instant::now() < expiry => NextFrame::None,
            _ => NextFrame::Finish,
        }
    }

    fn audio_clock_has_stalled(&mut self, position: Duration, now: Instant) -> bool {
        match self.audio_clock {
            Some((seen, since)) if seen == position => {
                now.duration_since(since) > AUDIO_STALL_TIMEOUT
            }
            _ => {
                self.audio_clock = Some((position, now));
                false
            }
        }
    }

    fn fall_back_to_wall_clock(&mut self) {
        tracing::warn!(
            "audio clock stopped at {:?}; pacing the rest of the video on the wall clock",
            self.video_clock
        );
        self.audio_stalled = true;
        let now = Instant::now();
        self.clock_origin = Some(now.checked_sub(self.video_clock).unwrap_or(now));
    }

    fn next_frame_audio_master(&mut self) -> NextFrame {
        let Some((position, output_position)) = self
            .audio_player
            .as_ref()
            .map(|player| (player.position(), player.output_position()))
        else {
            return self.next_frame_clock_master();
        };

        if position <= self.video_clock {
            if let Some(audio_player) = &self.audio_player {
                audio_player.play();
            }
            if self.audio_clock_has_stalled(output_position, Instant::now()) {
                self.fall_back_to_wall_clock();
                return self.next_frame_clock_master();
            }
            return NextFrame::None;
        }

        self.audio_clock = None;

        // ADDED: a frame held back at the seam (see `at_seam`).
        if let Some(frame) = self.pending.take() {
            return match self.at_seam(frame) {
                Some(frame) => {
                    self.pending = Some(frame);
                    self.next_frame_clock_master()
                }
                None => self.finish_once_the_last_frame_is_done(),
            };
        }

        let frame = loop {
            match self.rx.try_recv() {
                Ok(frame) => {
                    if let Some(audio_player) = &self.audio_player {
                        audio_player.play();
                        if frame.pts < audio_player.position().saturating_sub(self.tolerance) {
                            self.skipped_late += 1; // ADDED: counter
                            continue;
                        }
                    }
                    match self.at_seam(frame) {
                        Some(frame) => break frame,
                        None => return self.finish_once_the_last_frame_is_done(),
                    }
                }
                Err(TryRecvError::Empty) => {
                    self.lag_count += 1;
                    if let Some(audio_player) = &self.audio_player {
                        audio_player.pause();
                    }
                    return NextFrame::None;
                }
                Err(TryRecvError::Disconnected) => {
                    return self.finish_once_the_last_frame_is_done();
                }
            }
        };

        self.video_clock = frame.pts;
        self.current_frame_expiry = Some(Instant::now() + frame.duration);

        NextFrame::Ready(frame)
    }

    fn recv_frame(&mut self) -> RecvFrame {
        match self.rx.try_recv() {
            Ok(frame) => RecvFrame::Frame(frame),
            Err(TryRecvError::Empty) => {
                self.lag_count += 1;
                RecvFrame::Waiting
            }
            Err(TryRecvError::Disconnected) => RecvFrame::Finished,
        }
    }

    pub fn pause(&mut self) {
        if let Some(audio_player) = &self.audio_player {
            audio_player.pause();
        }
        if !self.paused {
            self.paused_at = Some(Instant::now());
        }
        self.paused = true;
    }

    pub fn play(&mut self) {
        if let Some(audio_player) = &self.audio_player {
            audio_player.play();
        }
        self.audio_clock = None;
        if let Some(paused_at) = self.paused_at.take()
            && let Some(origin) = self.clock_origin
        {
            self.clock_origin = Some(origin + paused_at.elapsed());
        }
        self.paused = false;
    }

    pub fn set_volume(&mut self, volume: f32) {
        if let Some(audio_player) = &self.audio_player {
            audio_player.set_volume(volume);
        }
    }

    pub fn set_loop(&self, loop_video: bool) {
        self.loop_video.store(loop_video, Ordering::Relaxed);
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub enum NextFrame {
    Ready(VideoFrame),
    Finish,
    None,
}

enum RecvFrame {
    Frame(VideoFrame),
    Waiting,
    Finished,
}

struct VideoMetadata {
    native_width: u32,
    native_height: u32,
    duration: Option<Duration>,
    nominal_frame: Duration,
}

struct VideoStream {
    rx: Receiver<VideoFrame>,
    meta: VideoMetadata,
}

fn spawn_video_stream(
    opts: &Options,
    stats: Arc<DecodeStats>,
    stop: Arc<AtomicBool>,
) -> Result<VideoStream> {
    // CHANGED: the engine buffers 2 decoded frames. Converted BGRA frames are bigger (8MB at
    // 1080p) but one more gives the UI thread a frame of slack on a hitch.
    let (tx, rx) = sync_channel(3);
    let (meta_tx, meta_rx) = sync_channel(1);

    let source = opts.source.clone();
    let packed_alpha = opts.packed_alpha;
    let start = opts.start;
    let threads = opts.threads;
    let target = opts.target_size.clone();
    let status = stats.clone();

    thread::Builder::new()
        .name("video-decode".into())
        .spawn(move || {
            if let Err(err) = decode_video(
                source,
                tx,
                packed_alpha,
                meta_tx,
                start,
                threads,
                target,
                stats,
                stop,
            ) {
                tracing::error!("Error decoding video: {err:#}");
                *status.error.lock().unwrap() = Some(format!("{err:#}"));
            }
            status.finished.store(true, Ordering::Release);
        })?;

    let meta = meta_rx
        .recv()
        .map_err(|_| anyhow::anyhow!("This file could not be decoded as video"))?;
    Ok(VideoStream { rx, meta })
}

const AUDIO_STALL_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_FRAME_DURATION: Duration = Duration::from_secs(1);

fn stream_duration(stream: &ffmpeg::Stream<'_>, time_base: ffmpeg::Rational) -> Option<Duration> {
    let units = stream.duration();
    (units > 0)
        .then(|| units as f64 * (time_base.0 as f64 / time_base.1 as f64))
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map(Duration::from_secs_f64)
}

fn timing_from_units(
    pts: i64,
    duration: i64,
    time_base: ffmpeg::Rational,
    nominal: Duration,
) -> (Duration, Duration) {
    let seconds = |units: i64| units as f64 * (time_base.0 as f64 / time_base.1 as f64);
    let pts = Duration::from_secs_f64(seconds(pts).max(0.0));
    let duration = Some(duration)
        .filter(|raw| *raw > 0)
        .map(|raw| Duration::from_secs_f64(seconds(raw)))
        .filter(|duration| *duration <= MAX_FRAME_DURATION)
        .unwrap_or(nominal);
    (pts, duration)
}

/// The size to convert to: the displayed size if smaller than native, keeping aspect.
fn dst_size(native_w: u32, native_h: u32, target: u64) -> (u32, u32) {
    let (tw, th) = ((target >> 32) as u32, target as u32);
    if tw == 0 || th == 0 || (tw >= native_w && th >= native_h) {
        return (native_w, native_h);
    }
    let scale = (tw as f64 / native_w as f64)
        .min(th as f64 / native_h as f64)
        .min(1.0);
    let w = ((native_w as f64 * scale).round() as u32).max(2) & !1;
    let h = ((native_h as f64 * scale).round() as u32).max(2) & !1;
    (w, h)
}

#[allow(clippy::too_many_arguments)]
fn decode_video(
    source: MediaSource,
    tx: SyncSender<VideoFrame>,
    packed_alpha: bool,
    meta_tx: SyncSender<VideoMetadata>,
    start: Duration,
    threads: usize,
    target: Arc<AtomicU64>,
    stats: Arc<DecodeStats>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    ffmpeg::init()?;
    let mut ictx = source.open()?;

    let stream_index = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .context("Couldn't find video stream")?
        .index();

    let video_stream = ictx.stream(stream_index).context("Invalid stream index")?;
    let time_base = video_stream.time_base();
    let avg_frame_rate = video_stream.avg_frame_rate();
    let frame_duration = if avg_frame_rate.numerator() > 0 {
        Duration::from_secs_f64(
            avg_frame_rate.denominator() as f64 / avg_frame_rate.numerator() as f64,
        )
    } else {
        Duration::from_millis(33)
    };
    let total_duration = stream_duration(&video_stream, time_base);

    let context_decoder = codec::Context::from_parameters(video_stream.parameters())?;
    let mut decoder = context_decoder.decoder().video()?;
    decoder.set_threading(codec::threading::Config {
        kind: codec::threading::Type::Frame,
        count: threads,
    });

    let native_width = decoder.width();
    let native_height = if packed_alpha {
        decoder.height() / 2
    } else {
        decoder.height()
    };

    let container_duration =
        (ictx.duration() > 0).then(|| Duration::from_micros(ictx.duration() as u64));

    if meta_tx
        .send(VideoMetadata {
            native_width,
            native_height,
            duration: total_duration.or(container_duration),
            nominal_frame: frame_duration,
        })
        .is_err()
    {
        return Ok(());
    }

    if !start.is_zero() {
        let ts = start.as_micros() as i64;
        ictx.seek(ts, ..ts).context("seeking the video")?;
    }

    let mut converter = Converter::new();
    let mut decoded = Video::empty();
    let mut current_loop_offset = Duration::ZERO;
    let mut pass = 0;
    let mut sequence = 0;
    // ADDED: frames this pass decoded, so a pass that decodes none ends the stream.
    let mut frames_in_pass = 0u64;
    let mut last_pts_duration = Duration::ZERO;
    let mut last_frame_duration = frame_duration;
    // Frames before the seek target are decoded but not converted or sent. Half a frame of slack
    // so the frame *at* the target isn't lost to rounding.
    let mut skip_before = (!start.is_zero()).then(|| start.saturating_sub(frame_duration / 2));

    // Returns false when the receiver is gone.
    let mut emit = |decoded: &mut Video,
                    loop_offset: Duration,
                    pass: u64,
                    skip_before: &mut Option<Duration>|
     -> (bool, Duration, Duration) {
        let (pts, duration) = timing_from_units(
            decoded.pts().unwrap_or(0),
            decoded.packet().duration,
            time_base,
            frame_duration,
        );
        if let Some(skip) = *skip_before {
            if pts < skip {
                return (true, pts, duration);
            }
            *skip_before = None;
        }

        let t = Instant::now();
        let (w, h) = dst_size(native_width, native_height, target.load(Ordering::Relaxed));
        let image = match converter.convert(decoded, packed_alpha, w, h) {
            Ok(bgra) => {
                let buffer =
                    image::RgbaImage::from_raw(w, h, bgra).expect("buffer sized for the image");
                Arc::new(RenderImage::new(smallvec![image::Frame::new(buffer)]))
            }
            Err(err) => {
                tracing::error!("convert: {err:#}");
                return (true, pts, duration);
            }
        };
        stats
            .convert_us
            .fetch_add(t.elapsed().as_micros() as u64, Ordering::Relaxed);
        stats.converted.fetch_add(1, Ordering::Relaxed);

        let frame = VideoFrame {
            sequence,
            image,
            pts: pts + loop_offset,
            duration,
            pass,
            pass_pts: pts,
        };
        sequence += 1;
        (tx.send(frame).is_ok(), pts, duration)
    };

    'main: loop {
        for (stream, packet) in ictx.packets() {
            if stop.load(Ordering::Relaxed) {
                break 'main;
            }
            if stream.index() == stream_index {
                let t = Instant::now();
                decoder.send_packet(&packet)?;
                loop {
                    let t2 = Instant::now();
                    let got = decoder.receive_frame(&mut decoded).is_ok();
                    stats
                        .decode_us
                        .fetch_add(t2.elapsed().as_micros() as u64, Ordering::Relaxed);
                    if !got {
                        break;
                    }
                    frames_in_pass += 1;
                    let (ok, pts, duration) =
                        emit(&mut decoded, current_loop_offset, pass, &mut skip_before);
                    last_pts_duration = pts;
                    last_frame_duration = duration;
                    if !ok {
                        break 'main;
                    }
                }
                let _ = t;
            }
        }

        decoder.send_eof()?;
        while decoder.receive_frame(&mut decoded).is_ok() {
            frames_in_pass += 1;
            let (ok, pts, duration) =
                emit(&mut decoded, current_loop_offset, pass, &mut skip_before);
            last_pts_duration = pts;
            last_frame_duration = duration;
            if !ok {
                break 'main;
            }
        }

        // CHANGED: always on to the next pass; `VideoDecoder::at_seam` decides as the seam plays
        // whether it's shown. A pass that produced nothing would only spin.
        if frames_in_pass == 0 {
            return Ok(());
        }
        frames_in_pass = 0;

        ictx.seek(0, ..0)?;
        decoder.flush();
        pass += 1;
        skip_before = None;
        current_loop_offset += total_duration
            .filter(|total| *total > last_pts_duration)
            .unwrap_or(last_pts_duration + last_frame_duration);
    }

    Ok(())
}
