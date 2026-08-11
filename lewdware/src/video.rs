use std::{
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use ffmpeg::codec;
use ffmpeg_next::{self as ffmpeg, ffi, frame::Video};
use winit::event_loop::EventLoopProxy;

use crate::{
    app::UserEvent,
    audio::AudioPlayer,
    media::MediaSource,
    zero_copy::{HardwareFrame, initialize_hardware_device, preferred_hw_type},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoPixelFormat {
    Yuv420p,
    Nv12,
}

thread_local! {
    static HW_PIX_FMT: Cell<i32> = const { Cell::new(ffi::AVPixelFormat::AV_PIX_FMT_NONE as i32) };
}

// fmts is a list terminated by AV_PIX_FMT_NONE. Loop through and try to find our desired format
// (HW_PIX_FMT), otherwise return AV_PIX_FMT_NONE.
unsafe extern "C" fn get_hw_format(
    _ctx: *mut ffi::AVCodecContext,
    fmts: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    let hw_fmt = HW_PIX_FMT.with(|c| c.get());
    let mut p = fmts;
    loop {
        let fmt = unsafe { *p };
        if fmt == ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            break;
        }
        if fmt as i32 == hw_fmt {
            return fmt;
        }
        p = unsafe { p.add(1) };
    }
    ffi::AVPixelFormat::AV_PIX_FMT_NONE
}

// Returns the hw pixel format (e.g. AV_PIX_FMT_VAAPI) on success.
unsafe fn try_hw_setup(
    ctx: *mut ffi::AVCodecContext,
    hw_type: ffi::AVHWDeviceType,
    wgpu_device: &std::sync::Arc<wgpu::Device>,
) -> Option<ffi::AVPixelFormat> {
    // ctx->codec is NULL before avcodec_open2; find the decoder via codec_id instead.
    let codec_id = unsafe { (*ctx).codec_id };
    let codec = unsafe { ffi::avcodec_find_decoder(codec_id) };
    if codec.is_null() {
        return None;
    }

    let mut hw_pix_fmt = ffi::AVPixelFormat::AV_PIX_FMT_NONE;
    let mut i = 0;
    // Loop through hardware configurations
    loop {
        let hw_config = unsafe { ffi::avcodec_get_hw_config(codec, i) };
        if hw_config.is_null() {
            break;
        }

        unsafe {
            if ((*hw_config).methods & ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32) != 0
                && (*hw_config).device_type == hw_type
            {
                hw_pix_fmt = (*hw_config).pix_fmt;
                break;
            }
        }
        i += 1;
    }
    if hw_pix_fmt == ffi::AVPixelFormat::AV_PIX_FMT_NONE {
        return None;
    }

    let mut hw_device_ctx = initialize_hardware_device(wgpu_device, hw_type)?;

    unsafe {
        (*ctx).hw_device_ctx = ffi::av_buffer_ref(hw_device_ctx);
        ffi::av_buffer_unref(&mut hw_device_ctx);
    }

    Some(hw_pix_fmt)
}

/// A video decoder using ffmpeg.
///
/// Which clock drives playback depends on whether the video has a soundtrack.
///
/// With audio, the audio is the master clock:
///
/// * If the video is ahead of the audio, playback will pause on the current frame until the audio
///   catches up.
/// * If the video is behind the audio, frames will be skipped until the video is back in sync.
///
/// Without audio, frames are scheduled against a wall clock anchored on the first frame.
pub struct VideoDecoder {
    rx: Receiver<VideoFrame>,
    audio_player: Option<AudioPlayer>,
    tolerance: Duration,
    clock_origin: Option<Instant>,
    paused_at: Option<Instant>,
    pending: Option<VideoFrame>,
    /// When the frame currently on screen stops being current. Only consulted once the decoder
    /// has finished, to hold the final frame for its own duration rather than tearing the window
    /// down the moment it appears.
    current_frame_expiry: Option<Instant>,
    /// The audio position last seen while waiting on it, and when it was first seen at that value.
    /// Feeds `audio_clock_has_stalled`; `None` whenever the clock is known to be moving.
    audio_clock: Option<(Duration, Instant)>,
    /// Set once the audio clock has been given up on, after which playback is paced on the wall
    /// clock like a silent video.
    audio_stalled: bool,
    video_clock: Duration,
    native_width: u32,
    native_height: u32,
    full_range: bool,
    pixel_format: VideoPixelFormat,
    packed_alpha: bool,
    paused: bool,
    lag_count: u32,
    loop_video: Arc<AtomicBool>,
}

pub struct VideoFrame {
    /// NV12 / YUV420P data in system memory, or empty when using zero-copy.
    pub frame: Video,
    pub hardware_frame: Option<HardwareFrame>,
    pub pts: Duration,
    /// How long this frame stays on screen. Only the last frame of a video that is not looping
    /// actually needs it — every other frame is displaced by its successor's `pts` — but it comes
    /// from the frame itself, so it is right even where the frame rate varies.
    pub duration: Duration,
    pub recycle_tx: SyncSender<Video>,
}

impl Drop for VideoFrame {
    fn drop(&mut self) {
        if unsafe { !self.frame.is_empty() } {
            let dummy = Video::empty();
            let frame = std::mem::replace(&mut self.frame, dummy);
            let _ = self.recycle_tx.try_send(frame);
        }
    }
}

impl VideoDecoder {
    pub fn new(
        source: MediaSource,
        play_audio: bool,
        loop_video: Arc<AtomicBool>,
        volume: f32,
        packed_alpha: bool,
        wgpu_device: Option<Arc<wgpu::Device>>,
    ) -> Result<Self> {
        let VideoStream {
            rx,
            native_width,
            native_height,
            full_range,
            pixel_format,
        } = spawn_video_stream(
            source.clone(),
            loop_video.clone(),
            packed_alpha,
            wgpu_device,
        )?;

        let audio_player = if play_audio {
            match AudioPlayer::new::<EventLoopProxy<UserEvent>>(
                source,
                loop_video.clone(),
                volume,
                None,
            ) {
                Ok(Some(audio_player)) => Some(audio_player),
                Ok(None) => None,
                Err(err) => {
                    tracing::error!("{err}");
                    None
                }
            }
        } else {
            None
        };

        Ok(Self {
            rx,
            native_width,
            native_height,
            full_range,
            pixel_format,
            packed_alpha,
            audio_player,
            loop_video,
            clock_origin: None,
            paused_at: None,
            pending: None,
            current_frame_expiry: None,
            audio_clock: None,
            audio_stalled: false,
            video_clock: Duration::ZERO,
            tolerance: Duration::from_millis(200),
            paused: true,
            lag_count: 0,
        })
    }

    pub fn native_width(&self) -> u32 {
        self.native_width
    }

    pub fn native_height(&self) -> u32 {
        self.native_height
    }

    pub fn full_range(&self) -> bool {
        self.full_range
    }

    pub fn pixel_format(&self) -> VideoPixelFormat {
        self.pixel_format
    }

    pub fn packed_alpha(&self) -> bool {
        self.packed_alpha
    }

    /// Get the next frame, if it's ready.
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

    fn next_frame_clock_master(&mut self) -> NextFrame {
        let frame = match self.pending.take() {
            Some(frame) => frame,
            None => match self.recv_frame() {
                RecvFrame::Frame(frame) => frame,
                RecvFrame::Waiting => return NextFrame::None,
                RecvFrame::Finished => return self.finish_once_the_last_frame_is_done(),
            },
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
        // `deadline.max(now)` rather than either alone: normally the frame is up from when it was
        // due, but a re-anchored or overdue frame goes up now.
        self.current_frame_expiry = Some(deadline.max(now) + frame.duration);

        NextFrame::Ready(frame)
    }

    /// Report the video over — but not before the frame on screen has had its time.
    ///
    /// Finishing the moment the decoder runs dry tears the window down a poll after the closing
    /// frame appears, so it flashes past however long it was meant to hold for. The wait is
    /// against the wall clock rather than either master clock on purpose: it has to end. The
    /// audio path's clock is `AudioPlayer::position`, which stops advancing once the sink drains,
    /// and waiting on a clock that has stopped would leave the window up for good. A wall-clock
    /// deadline is bounded by the frame duration, itself capped at `MAX_FRAME_DURATION`.
    fn finish_once_the_last_frame_is_done(&self) -> NextFrame {
        match self.current_frame_expiry {
            Some(expiry) if Instant::now() < expiry => NextFrame::None,
            _ => NextFrame::Finish,
        }
    }

    /// Whether the audio clock has stopped advancing, judged only from moments when we were
    /// waiting on it with the sink told to play.
    ///
    /// rodio refreshes the position every 5ms for as long as the source is yielding samples, and
    /// stops entirely once it runs dry — the position then sits at its final value for good. So a
    /// position that has not moved in [`AUDIO_STALL_TIMEOUT`] is not a slow clock, it is a stopped
    /// one, and no amount of further waiting will move it.
    ///
    /// The threshold is generous because being wrong in one direction costs far more than the
    /// other: falling back early only means the tail of a clip plays unsynchronised, while falling
    /// back too late — or never — leaves a window on screen that can never close itself.
    fn audio_clock_has_stalled(&mut self, position: Duration, now: Instant) -> bool {
        match self.audio_clock {
            // Sitting at the same position it was last time we looked.
            Some((seen, since)) if seen == position => {
                now.duration_since(since) > AUDIO_STALL_TIMEOUT
            }
            // Either the first look, or it has moved since the last one; start the clock again.
            _ => {
                self.audio_clock = Some((position, now));
                false
            }
        }
    }

    /// Give up on the audio clock and play out the rest of the video on the wall clock.
    fn fall_back_to_wall_clock(&mut self) {
        tracing::warn!(
            "audio clock stopped at {:?}; pacing the rest of the video on the wall clock",
            self.video_clock
        );

        self.audio_stalled = true;

        // The wall-clock path anchors itself on the first frame it is asked for, and the video is
        // already `video_clock` in. Without seeding the origin to match, every frame still to come
        // would be overdue the moment it arrived and the remainder would play out at once.
        let now = Instant::now();
        self.clock_origin = Some(now.checked_sub(self.video_clock).unwrap_or(now));
    }

    fn next_frame_audio_master(&mut self) -> NextFrame {
        let Some(position) = self.audio_player.as_ref().map(AudioPlayer::position) else {
            return self.next_frame_clock_master();
        };

        if position <= self.video_clock {
            // Video is ahead of or in sync with audio. Ensure audio is playing.
            if let Some(audio_player) = &self.audio_player {
                audio_player.play();
            }

            // We have just told the sink to play and are waiting on it to catch up. If it is not
            // moving, it is never going to, and waiting on it forever leaves a window that cannot
            // close -- see `audio_clock_has_stalled`.
            if self.audio_clock_has_stalled(position, Instant::now()) {
                self.fall_back_to_wall_clock();
                return self.next_frame_clock_master();
            }

            return NextFrame::None;
        }

        // The audio is moving, so anything the stall watch had recorded is stale.
        self.audio_clock = None;

        let frame = loop {
            match self.rx.try_recv() {
                Ok(frame) => {
                    if let Some(audio_player) = &self.audio_player {
                        audio_player.play();

                        // Drop frames if we are too far behind
                        if frame.pts < audio_player.position().saturating_sub(self.tolerance) {
                            continue;
                        }
                    }
                    break frame;
                }
                Err(TryRecvError::Empty) => {
                    self.lag_count += 1;
                    // The decoder is lagging behind the audio, so we pause the audio to wait for it.
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
        // Measured from now rather than from a scheduled deadline: on this path the audio decides
        // when a frame goes up, so now *is* when it went up.
        self.current_frame_expiry = Some(Instant::now() + frame.duration);

        NextFrame::Ready(frame)
    }

    /// Take the next decoded frame without blocking.
    fn recv_frame(&mut self) -> RecvFrame {
        match self.rx.try_recv() {
            Ok(frame) => RecvFrame::Frame(frame),
            Err(TryRecvError::Empty) => {
                self.lag_count += 1;
                RecvFrame::Waiting
            }
            // The decoder drops the sender once it has decoded everything it is going to. With
            // looping on it never does, so this is reached only for a video playing through once.
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

        // A pause stops the audio clock too, so anything the stall watch recorded before it would
        // read as a stall the moment we resume.
        self.audio_clock = None;

        // Time spent paused is not part of the video's timeline. Push the origin past it, or
        // every frame buffered behind the pause would come due at once on resume.
        if let Some(paused_at) = self.paused_at.take()
            && let Some(origin) = self.clock_origin
        {
            self.clock_origin = Some(origin + paused_at.elapsed());
        }
        self.paused = false;
    }

    pub fn set_volume(&self, volume: f32) {
        if let Some(audio_player) = &self.audio_player {
            audio_player.set_volume(volume);
        }
    }

    pub fn set_loop(&self, loop_video: bool) {
        self.loop_video.store(loop_video, Ordering::Relaxed);
    }
}

pub enum NextFrame {
    Ready(VideoFrame),
    Finish,
    None,
}

/// The outcome of a non-blocking pull from the decoder thread.
enum RecvFrame {
    Frame(VideoFrame),
    /// Nothing decoded yet.
    Waiting,
    /// The decoder is done and will send nothing more.
    Finished,
}

struct VideoMetadata {
    native_width: u32,
    native_height: u32,
    full_range: bool,
    pixel_format: VideoPixelFormat,
}

struct VideoStream {
    rx: Receiver<VideoFrame>,
    native_width: u32,
    native_height: u32,
    full_range: bool,
    pixel_format: VideoPixelFormat,
}

/// Spawn a thread to decode frames from a video.
fn spawn_video_stream(
    source: MediaSource,
    loop_video: Arc<AtomicBool>,
    packed_alpha: bool,
    wgpu_device: Option<Arc<wgpu::Device>>,
) -> Result<VideoStream> {
    let (tx, rx) = sync_channel(2);
    let (meta_tx, meta_rx) = sync_channel(1);
    let (recycle_tx, recycle_rx) = sync_channel::<Video>(5);

    thread::spawn(move || {
        if let Err(err) = decode_video(
            source,
            tx,
            loop_video,
            packed_alpha,
            meta_tx,
            recycle_rx,
            recycle_tx.clone(),
            wgpu_device,
        ) {
            tracing::error!("Error decoding video: {}", err);
        }
    });

    let meta = meta_rx
        .recv()
        .context("Failed to receive video metadata from spawn thread")?;

    Ok(VideoStream {
        rx,
        native_width: meta.native_width,
        native_height: meta.native_height,
        full_range: meta.full_range,
        pixel_format: meta.pixel_format,
    })
}

fn hw_frame_to_video_frame(
    decoded: &mut Video,
    recycle_tx: &SyncSender<Video>,
    pts: Duration,
    duration: Duration,
) -> Result<VideoFrame, ()> {
    if let Some(frame) = HardwareFrame::from_decoder_frame(decoded) {
        return Ok(VideoFrame {
            frame: Video::empty(),
            hardware_frame: Some(frame),
            pts,
            duration,
            recycle_tx: recycle_tx.clone(),
        });
    }

    // CPU fallback: transfer NV12 data to system memory.
    let mut sw = Video::empty();
    let ret = unsafe { ffi::av_hwframe_transfer_data(sw.as_mut_ptr(), decoded.as_ptr(), 0) };
    if ret < 0 {
        tracing::error!("av_hwframe_transfer_data failed: {ret}");
        return Err(());
    }

    Ok(VideoFrame {
        frame: sw,
        hardware_frame: None,
        pts,
        duration,
        recycle_tx: recycle_tx.clone(),
    })
}

/// How long the audio position may sit unchanged, while we are waiting on it with the sink told to
/// play, before it is treated as stopped rather than slow. rodio refreshes it every 5ms while the
/// source yields samples, so this is roughly two hundred missed updates.
const AUDIO_STALL_TIMEOUT: Duration = Duration::from_secs(1);

/// No real frame stays on screen for a second. A container that claims otherwise is not to be
/// believed, because the claim decides how long the last frame of a non-looping video holds the
/// window open.
const MAX_FRAME_DURATION: Duration = Duration::from_secs(1);

/// How long the video runs for, from the stream's own header, or `None` when it declares nothing
/// usable. Plenty of real files are muxed without one.
fn stream_duration(stream: &ffmpeg::Stream<'_>, time_base: ffmpeg::Rational) -> Option<Duration> {
    let units = stream.duration();
    (units > 0)
        .then(|| units as f64 * (time_base.0 as f64 / time_base.1 as f64))
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map(Duration::from_secs_f64)
}

/// A decoded frame's presentation timestamp and how long it is meant to stay up, converted from
/// `time_base` units into wall-clock terms.
///
/// Note the duration is the container's per-frame claim, taken in *decode* order — for a stream
/// with B-frames it need not equal the gap to the frame shown next. It is only ever used to decide
/// how long the closing frame of a video holds, where being approximately right beats the nothing
/// that came before it; the loop seam uses `stream_duration` instead, which does not have this
/// problem.
///
/// The duration is the frame's own, which is what makes it right for variable-frame-rate sources
/// — several of the GIFs this was built against hold one frame noticeably longer than the rest.
/// Containers that carry no per-frame duration report zero, and then `nominal` (the stream's
/// average frame rate) stands in.
fn frame_timing(
    frame: &Video,
    time_base: ffmpeg::Rational,
    nominal: Duration,
) -> (Duration, Duration) {
    timing_from_units(
        frame.pts().unwrap_or(0),
        frame.packet().duration,
        time_base,
        nominal,
    )
}

/// The arithmetic behind [`frame_timing`], over the raw `time_base` units rather than a frame.
///
/// Split out so the rules above can be exercised directly: which `AVFrame` field a duration is
/// read from moves between FFmpeg versions (`pkt_duration` before 7.0, `duration` after), so a
/// test that reached into a frame to plant one would be testing the ffmpeg build it happened to
/// link against as much as the rules here.
fn timing_from_units(
    pts: i64,
    duration: i64,
    time_base: ffmpeg::Rational,
    nominal: Duration,
) -> (Duration, Duration) {
    let seconds = |units: i64| units as f64 * (time_base.0 as f64 / time_base.1 as f64);

    // A negative timestamp is not a time we can represent, and `from_secs_f64` panics on one.
    let pts = Duration::from_secs_f64(seconds(pts).max(0.0));

    let duration = Some(duration)
        .filter(|raw| *raw > 0)
        .map(|raw| Duration::from_secs_f64(seconds(raw)))
        .filter(|duration| *duration <= MAX_FRAME_DURATION)
        .unwrap_or(nominal);

    (pts, duration)
}

/// Move the just-decoded frame out of `decoded` and into a `VideoFrame` to hand to the player,
/// leaving a recycled buffer behind for ffmpeg to fill next. `None` for a hardware frame that
/// could not be transferred, which is skipped rather than fatal.
fn take_decoded_frame(
    decoded: &mut Video,
    hw_pix_fmt: Option<ffi::AVPixelFormat>,
    recycle_rx: &Receiver<Video>,
    recycle_tx: &SyncSender<Video>,
    pts: Duration,
    duration: Duration,
) -> Option<VideoFrame> {
    if let Some(hw_fmt) = hw_pix_fmt
        && unsafe { (*decoded.as_ptr()).format } == hw_fmt as i32
    {
        return hw_frame_to_video_frame(decoded, recycle_tx, pts, duration).ok();
    }

    // Software decode, or a hardware decoder that handed back a system-memory frame anyway: swap
    // `decoded` out so ffmpeg can carry on filling a recycled buffer rather than allocating.
    let next = recycle_rx.try_recv().unwrap_or_else(|_| Video::empty());
    let frame = std::mem::replace(decoded, next);

    Some(VideoFrame {
        frame,
        hardware_frame: None,
        pts,
        duration,
        recycle_tx: recycle_tx.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn decode_video(
    source: MediaSource,
    tx: SyncSender<VideoFrame>,
    loop_video: Arc<AtomicBool>,
    packed_alpha: bool,
    meta_tx: SyncSender<VideoMetadata>,
    recycle_rx: Receiver<Video>,
    recycle_tx: SyncSender<Video>,
    wgpu_device: Option<Arc<wgpu::Device>>,
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

    // Where one pass through the video ends and the next begins. The muxer derives this by summing
    // every frame's duration, so it already accounts for however long the closing frame is meant
    // to hold — and unlike a per-frame duration it is stated in presentation terms, so B-frame
    // reordering cannot skew it (see `frame_timing`).
    let total_duration = stream_duration(&video_stream, time_base);

    let mut context_decoder = codec::Context::from_parameters(video_stream.parameters())?;

    // Attempt hardware decoding setup before avcodec_open2 (which happens inside .video()).
    let hw_pix_fmt: Option<ffi::AVPixelFormat> = wgpu_device.as_ref().and_then(|device| unsafe {
        let ctx_ptr = context_decoder.as_mut_ptr();
        let hw_type = preferred_hw_type();
        if let Some(fmt) = try_hw_setup(ctx_ptr, hw_type, device) {
            HW_PIX_FMT.with(|c| c.set(fmt as i32));
            (*ctx_ptr).get_format = Some(get_hw_format);
            Some(fmt)
        } else {
            None
        }
    });

    let mut decoder = context_decoder.decoder().video()?;

    // Limit (software-decoding) thread count to 1
    decoder.set_threading(codec::threading::Config {
        kind: codec::threading::Type::Frame,
        count: 1,
    });

    let native_width = decoder.width();

    // For packed-alpha videos the decoded frame is twice the display height.
    let native_height = if packed_alpha {
        decoder.height() / 2
    } else {
        decoder.height()
    };

    let full_range = decoder.color_range() == ffmpeg::color::Range::JPEG;
    let pixel_format = if hw_pix_fmt.is_some() {
        VideoPixelFormat::Nv12
    } else {
        VideoPixelFormat::Yuv420p
    };

    if meta_tx
        .send(VideoMetadata {
            native_width,
            native_height,
            full_range,
            pixel_format,
        })
        .is_err()
    {
        tracing::error!("Failed to send video metadata");
        return Ok(());
    }

    // `decoded` is a reusable receive buffer for the software-decode path.
    let mut decoded = Video::empty();

    let mut current_loop_offset = Duration::ZERO;
    let mut last_pts_duration = Duration::ZERO;
    // How long the final frame of the pass stays up, which is what the next pass has to start
    // after. Seeded with the stream's nominal duration in case a pass yields no frames at all.
    let mut last_frame_duration = frame_duration;

    'main: loop {
        for (stream, packet) in ictx.packets() {
            if stream.index() == stream_index {
                decoder.send_packet(&packet)?;
                while decoder.receive_frame(&mut decoded).is_ok() {
                    let (pts, duration) = frame_timing(&decoded, time_base, frame_duration);
                    last_pts_duration = pts;
                    last_frame_duration = duration;

                    let Some(video_frame) = take_decoded_frame(
                        &mut decoded,
                        hw_pix_fmt,
                        &recycle_rx,
                        &recycle_tx,
                        pts + current_loop_offset,
                        duration,
                    ) else {
                        continue;
                    };

                    if tx.send(video_frame).is_err() {
                        break 'main;
                    }
                }
            }
        }

        // Ask the decoder for the frames it is still holding back, rather than `flush`ing them
        // away — `flush` is `avcodec_flush_buffers`, which discards them, so the drain below used
        // to find nothing and the tail of every pass was silently dropped. A stream with B-frames
        // always has some in hand: `has_b_frames` of them, which for these encodes is two, and
        // those two frames then took their time out of the closing frame's spell on screen.
        decoder.send_eof()?;

        while decoder.receive_frame(&mut decoded).is_ok() {
            let (pts, duration) = frame_timing(&decoded, time_base, frame_duration);
            last_pts_duration = pts;
            last_frame_duration = duration;

            let Some(video_frame) = take_decoded_frame(
                &mut decoded,
                hw_pix_fmt,
                &recycle_rx,
                &recycle_tx,
                pts + current_loop_offset,
                duration,
            ) else {
                continue;
            };

            if tx.send(video_frame).is_err() {
                break 'main;
            }
        }

        if !loop_video.load(Ordering::Relaxed) {
            return Ok(());
        }

        ictx.seek(0, ..0)?;
        decoder.flush();
        // The next pass starts one whole video-length on, so the closing frame gets exactly its
        // own time on screen and the seam is invisible. Falling back through the last frame's
        // declared duration to the stream average only matters for a container that states no
        // length of its own; the average was what this always used, and it cut a long closing
        // frame short, and dragged out a short one, once per loop.
        current_loop_offset += total_duration
            .filter(|total| *total > last_pts_duration)
            .unwrap_or(last_pts_duration + last_frame_duration);
    }

    Ok(())
}

/// Pacing tests for the no-audio path, driving `next_frame` against a hand-fed frame channel so
/// no ffmpeg decode, file or GPU is involved — only the scheduling in
/// [`VideoDecoder::next_frame_clock_master`].
///
/// These assert against real elapsed time, so the bounds are deliberately lopsided: the lower
/// bound (a frame must not appear before it is due) is exact and is the property under test, while
/// the upper bound carries enough slack to survive a loaded machine.
#[cfg(test)]
mod pacing {
    use super::*;

    const PERIOD: Duration = Duration::from_millis(30);

    /// A decoder wired to a channel already holding frames at `pts`, playing, with no audio.
    /// The sender is dropped, so running off the end of the list reports `Finish`.
    fn decoder(pts: &[Duration]) -> (VideoDecoder, Receiver<Video>) {
        let (tx, rx) = sync_channel(pts.len());
        let (recycle_tx, recycle_rx) = sync_channel(pts.len().max(1));

        for &pts in pts {
            tx.send(VideoFrame {
                frame: Video::empty(),
                hardware_frame: None,
                pts,
                duration: PERIOD,
                recycle_tx: recycle_tx.clone(),
            })
            .expect("channel sized for every frame");
        }

        let decoder = VideoDecoder {
            rx,
            audio_player: None,
            tolerance: Duration::from_millis(200),
            clock_origin: None,
            paused_at: None,
            pending: None,
            current_frame_expiry: None,
            audio_clock: None,
            audio_stalled: false,
            video_clock: Duration::ZERO,
            native_width: 64,
            native_height: 64,
            full_range: false,
            pixel_format: VideoPixelFormat::Yuv420p,
            packed_alpha: false,
            paused: false,
            lag_count: 0,
            loop_video: Arc::new(AtomicBool::new(false)),
        };

        // The recycle receiver is returned rather than dropped so `VideoFrame::drop` has somewhere
        // to put buffers back, exactly as in the real pipeline.
        (decoder, recycle_rx)
    }

    /// Poll like the render loop does, every `interval`, until a frame comes out. Returns how
    /// long that took, or `None` if the decoder finished first.
    fn poll_for_frame(decoder: &mut VideoDecoder, interval: Duration) -> Option<Duration> {
        let start = Instant::now();
        loop {
            match decoder.next_frame() {
                NextFrame::Ready(_) => return Some(start.elapsed()),
                NextFrame::Finish => return None,
                NextFrame::None => {}
            }

            assert!(
                start.elapsed() < Duration::from_secs(5),
                "no frame after 5s; the scheduler is stuck"
            );
            thread::sleep(interval);
        }
    }

    fn millis(elapsed: Duration) -> f64 {
        elapsed.as_secs_f64() * 1000.0
    }

    /// The regression that started this: with the interval measured from the *preceding pair* of
    /// frames there was no interval to measure yet at the start, so the first two frames of every
    /// clip were handed over back to back and flashed past.
    #[test]
    fn the_first_frames_are_paced_like_any_other() {
        let (mut decoder, _recycle) = decoder(&[Duration::ZERO, PERIOD, PERIOD * 2]);

        // Frame one anchors the clock and shows at once.
        assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));

        // Frame two is not due yet, however eagerly it is asked for.
        for _ in 0..10 {
            assert!(
                matches!(decoder.next_frame(), NextFrame::None),
                "a frame was handed over before its timestamp was due"
            );
        }

        let waited = poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("frame two");
        assert!(
            waited >= PERIOD.mul_f64(0.9),
            "frame two came {:.1}ms after frame one, expected ~{:.1}ms",
            millis(waited),
            millis(PERIOD),
        );
    }

    /// Frames are scheduled against a fixed origin, so a poll cadence coarser than the frame
    /// period cannot inflate the period. Polling every 16ms for 30ms frames used to round every
    /// single frame up to 32ms, running the clip ~7% slow for as long as it played.
    #[test]
    fn a_coarse_poll_cadence_does_not_stretch_playback() {
        let count = 10;
        let pts: Vec<Duration> = (0..count).map(|i| PERIOD * i).collect();
        let (mut decoder, _recycle) = decoder(&pts);

        let start = Instant::now();
        for i in 0..count {
            poll_for_frame(&mut decoder, Duration::from_millis(16))
                .unwrap_or_else(|| panic!("frame {i} never arrived"));
        }
        let elapsed = start.elapsed();

        let expected = PERIOD * (count - 1);
        assert!(
            elapsed < expected + Duration::from_millis(40),
            "{count} frames took {:.1}ms, expected ~{:.1}ms — the schedule is drifting",
            millis(elapsed),
            millis(expected),
        );
    }

    /// Variable-frame-rate clips (several of the GIFs this was found with) carry uneven gaps.
    /// Each frame has to wait out *its own* gap, not the one before it.
    #[test]
    fn an_uneven_gap_applies_to_the_frame_that_carries_it() {
        let long = Duration::from_millis(90);
        let (mut decoder, _recycle) = decoder(&[Duration::ZERO, PERIOD, PERIOD + long]);

        assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));
        poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("frame two");

        let waited = poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("frame three");
        assert!(
            waited >= long.mul_f64(0.9),
            "the {:.0}ms gap was applied a frame late: waited only {:.1}ms",
            millis(long),
            millis(waited),
        );
    }

    /// Time spent paused is not time on the video's timeline, so resuming must not make every
    /// frame behind the pause immediately due.
    #[test]
    fn pausing_does_not_spend_the_schedule() {
        let (mut decoder, _recycle) = decoder(&[Duration::ZERO, PERIOD * 4]);

        assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));

        decoder.pause();
        thread::sleep(PERIOD * 4);
        assert!(
            matches!(decoder.next_frame(), NextFrame::None),
            "a paused decoder handed over a frame"
        );
        decoder.play();

        assert!(
            matches!(decoder.next_frame(), NextFrame::None),
            "the pause was counted against the next frame's schedule"
        );

        let waited = poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("frame two");
        assert!(
            waited >= (PERIOD * 4).mul_f64(0.8),
            "frame two came {:.1}ms after resuming, expected ~{:.1}ms",
            millis(waited),
            millis(PERIOD * 4),
        );
    }

    /// A stall past the sync tolerance abandons the old timeline instead of dumping every overdue
    /// frame at once.
    #[test]
    fn a_long_stall_re_anchors_rather_than_racing() {
        let pts: Vec<Duration> = (0..4).map(|i| PERIOD * i).collect();
        let (mut decoder, _recycle) = decoder(&pts);

        assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));

        // Nothing polls for far longer than the tolerance: every remaining frame is now overdue.
        thread::sleep(decoder.tolerance + PERIOD);

        assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));
        assert!(
            matches!(decoder.next_frame(), NextFrame::None),
            "the backlog was raced through instead of re-anchoring"
        );
    }

    /// A loop seam is not a discontinuity: the decoder offsets each pass's timestamps past the
    /// one before, so the first frame of a new pass is due exactly one period after the last
    /// frame of the old one and the schedule carries straight through it.
    #[test]
    fn playback_carries_through_a_loop_seam() {
        // A two-frame clip played twice: the second pass carries on from where the first left
        // off, which is exactly what `current_loop_offset` does in the decode thread.
        let pts: Vec<Duration> = (0..4).map(|i| PERIOD * i).collect();
        let (mut decoder, _recycle) = decoder(&pts);

        assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));
        poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("frame two");

        // The seam itself: the first frame of the second pass waits its own period, no more.
        let waited = poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("seam frame");
        assert!(
            waited >= PERIOD.mul_f64(0.9),
            "the frame after the seam came early, at {:.1}ms",
            millis(waited),
        );
        assert!(
            waited < PERIOD * 2,
            "the seam cost an extra delay: {:.1}ms for a {:.1}ms period",
            millis(waited),
            millis(PERIOD),
        );
    }

    /// A video playing through once finishes when the decoder drops the sender — there is no
    /// in-band marker for it — but not until its last frame has had its time on screen. Finishing
    /// the instant that frame is handed over tears the window down a poll later, so the last
    /// frame of every non-looping video flashed past.
    #[test]
    fn the_last_frame_is_held_for_its_own_duration() {
        let (mut decoder, _recycle) = decoder(&[Duration::ZERO]);

        assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));

        let start = Instant::now();
        for _ in 0..5 {
            assert!(
                matches!(decoder.next_frame(), NextFrame::None),
                "finished while the last frame was still due to be on screen"
            );
        }

        loop {
            match decoder.next_frame() {
                NextFrame::Finish => break,
                NextFrame::None => thread::sleep(Duration::from_millis(1)),
                NextFrame::Ready(_) => panic!("no frames left to hand over"),
            }
            assert!(start.elapsed() < Duration::from_secs(1), "never finished");
        }

        assert!(
            start.elapsed() >= PERIOD.mul_f64(0.9),
            "the last frame was held {:.1}ms, expected its full {:.1}ms",
            millis(start.elapsed()),
            millis(PERIOD),
        );
    }

    /// A video that never produced a frame has nothing to hold, and must finish rather than wait
    /// for an expiry that will never be set. This is the case that would hang the window open.
    #[test]
    fn a_video_with_no_frames_finishes_rather_than_waiting() {
        let (mut decoder, _recycle) = decoder(&[]);

        assert!(
            matches!(decoder.next_frame(), NextFrame::Finish),
            "a video with no frames at all failed to finish"
        );
    }

    /// The hold is shared by both master clocks — `next_frame_audio_master` reaches it on the same
    /// path — so its contract is pinned here directly. The audio path cannot be driven from a test
    /// (an `AudioPlayer` owns a real output device), which is exactly why the decision lives in one
    /// `&self` helper rather than being written out twice.
    #[test]
    fn the_hold_releases_once_the_frame_has_had_its_time() {
        let (mut decoder, _recycle) = decoder(&[]);

        decoder.current_frame_expiry = Some(Instant::now() + PERIOD);
        let held = decoder.finish_once_the_last_frame_is_done();
        assert!(
            matches!(held, NextFrame::None),
            "finished while the last frame was still due to be up"
        );

        decoder.current_frame_expiry = Some(Instant::now() - PERIOD);
        let released = decoder.finish_once_the_last_frame_is_done();
        assert!(
            matches!(released, NextFrame::Finish),
            "kept holding a frame whose time was already up"
        );
    }

    /// An audio clock that is still moving is not stalled, however slowly it moves.
    #[test]
    fn an_advancing_audio_clock_is_never_treated_as_stalled() {
        let (mut decoder, _recycle) = decoder(&[]);
        let start = Instant::now();

        // Well past the timeout, but the position moves every time we look.
        for step in 0..10 {
            let position = PERIOD * step;
            let now = start + AUDIO_STALL_TIMEOUT * 2 * step;
            assert!(
                !decoder.audio_clock_has_stalled(position, now),
                "a moving audio clock was declared stalled at step {step}"
            );
        }
    }

    /// A clock sitting at one position for longer than the timeout has stopped, not slowed. This
    /// is what rescues a video whose audio track ends before its last frame: without it the wait
    /// never ends, and the window can never close itself.
    #[test]
    fn an_audio_clock_that_stops_is_given_up_on() {
        let (mut decoder, _recycle) = decoder(&[]);
        let start = Instant::now();
        let stuck = Duration::from_secs(4);

        assert!(
            !decoder.audio_clock_has_stalled(stuck, start),
            "declared stalled on the very first look, with nothing to compare against"
        );
        assert!(
            !decoder.audio_clock_has_stalled(stuck, start + AUDIO_STALL_TIMEOUT),
            "declared stalled before the timeout had elapsed"
        );
        assert!(
            decoder.audio_clock_has_stalled(stuck, start + AUDIO_STALL_TIMEOUT * 2),
            "an audio clock stopped for twice the timeout was still being waited on"
        );
    }

    /// Giving up has to hand the wall-clock path a timeline that continues from where the audio
    /// got to. Anchoring it from scratch would leave every remaining frame instantly overdue, and
    /// the rest of the video would play out in one burst.
    #[test]
    fn falling_back_resumes_from_where_the_audio_stopped() {
        let played = Duration::from_secs(4);
        let (mut decoder, _recycle) = decoder(&[played, played + PERIOD]);
        decoder.video_clock = played;

        decoder.fall_back_to_wall_clock();

        assert!(decoder.audio_stalled, "did not switch off the audio clock");

        // The frame at the point the audio stopped is due immediately; the one after it waits.
        assert!(matches!(decoder.next_frame(), NextFrame::Ready(_)));
        assert!(
            matches!(decoder.next_frame(), NextFrame::None),
            "the rest of the video was dumped at once instead of being paced"
        );

        let waited = poll_for_frame(&mut decoder, Duration::from_millis(1)).expect("next frame");
        assert!(
            waited >= PERIOD.mul_f64(0.9),
            "frame after the fallback came {:.1}ms on, expected ~{:.1}ms",
            millis(waited),
            millis(PERIOD),
        );
    }

    /// A duration the container could not supply, or one too large to be real, must not strand
    /// the window on screen waiting for a frame that is never displaced.
    #[test]
    fn an_implausible_frame_duration_is_not_trusted() {
        let nominal = Duration::from_millis(33);
        let time_base = ffmpeg::Rational::new(1, 1000);
        let duration = |raw| timing_from_units(0, raw, time_base, nominal).1;

        // No duration at all: the stream's nominal frame duration stands in.
        assert_eq!(duration(0), nominal);

        // A duration past anything a real frame holds for is refused the same way.
        assert_eq!(duration(60_000), nominal);

        // A plausible one is taken at face value.
        assert_eq!(duration(40), Duration::from_millis(40));
    }
}

/// Decode-level coverage against real files, built at test time with the sidecar ffmpeg.
///
/// `mod pacing` drives `next_frame` against a hand-fed channel, which deliberately involves no
/// ffmpeg at all — so nothing there exercises `decode_video`, and the bug these cover (the tail of
/// every pass being discarded rather than drained) was invisible to it. It only shows up against
/// a stream that actually holds frames back, which means real B-frames and a real decoder.
///
/// `#[ignore]`d so the default `cargo test` needs no binaries, matching `shared::encode`'s own
/// sidecar module. To run:
/// `./deploy/linux/download_ffmpeg_sidecars.sh && cargo test -p lewdware --bin lewdware-engine sidecar -- --ignored`
#[cfg(test)]
mod sidecar {
    use super::*;
    use crate::media::MediaSource;
    use std::path::{Path, PathBuf};

    /// Where the sidecars live. `download_ffmpeg_sidecars.sh` stages them under `pack-editor`
    /// relative to the repo root; `LEWDWARE_SIDECAR_DIR` overrides for other layouts.
    fn binaries_dir() -> PathBuf {
        std::env::var_os("LEWDWARE_SIDECAR_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../pack-editor/src-tauri/binaries")
            })
    }

    fn ffmpeg(dir: &Path, args: &[&str]) {
        let ffmpeg = binaries_dir().join("lewdware-ffmpeg");
        assert!(
            ffmpeg.is_file(),
            "no ffmpeg sidecar at {} -- run deploy/linux/download_ffmpeg_sidecars.sh from the \
             repo root, or set LEWDWARE_SIDECAR_DIR",
            binaries_dir().display()
        );

        let output = std::process::Command::new(ffmpeg)
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

    /// One `-show_entries` value, as ffprobe printed it.
    fn probe(path: &Path, entries: &str) -> String {
        let output = std::process::Command::new(binaries_dir().join("lewdware-ffprobe"))
            .args(["-v", "error", "-select_streams", "v:0"])
            .args(["-show_entries", entries])
            .args(["-of", "default=nw=1:nk=1"])
            .arg(path)
            .output()
            .expect("run sidecar ffprobe");

        assert!(
            output.status.success(),
            "ffprobe {entries} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// A 2s clip at the same awkward 100/3 fps the GIF encodes land on, encoded exactly as
    /// `shared::encode`'s software path does. Returns its path, frame count and container length.
    fn fixture(dir: &Path) -> (PathBuf, usize, Duration) {
        ffmpeg(
            dir,
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc=s=64x64:r=100/3:d=2",
                "-c:v",
                "libx264",
                "-crf",
                "23",
                "-pix_fmt",
                "yuv420p",
                "clip.mp4",
            ],
        );

        let path = dir.join("clip.mp4");

        // The whole point of the fixture: a decoder with nothing in hand cannot demonstrate the
        // difference between draining and discarding.
        let b_frames = probe(&path, "stream=has_b_frames");
        let b_frames: u32 = b_frames.parse().expect("has_b_frames");
        assert!(
            b_frames > 0,
            "fixture has no B-frames, so it cannot catch a decoder that discards held-back frames"
        );

        let frames: usize = probe(&path, "stream=nb_frames").parse().expect("nb_frames");
        let duration: f64 = probe(&path, "format=duration").parse().expect("duration");

        (path, frames, Duration::from_secs_f64(duration))
    }

    fn source(path: &Path) -> MediaSource {
        MediaSource {
            path: path.to_path_buf(),
            offset: 0,
            length: std::fs::metadata(path).expect("fixture exists").len(),
        }
    }

    /// Read straight from the decode thread rather than through `VideoDecoder`, so the frames
    /// arrive as fast as they decode instead of in real time.
    fn stream(path: &Path, loop_video: bool) -> Receiver<VideoFrame> {
        spawn_video_stream(
            source(path),
            Arc::new(AtomicBool::new(loop_video)),
            false,
            None,
        )
        .expect("spawn decode thread")
        .rx
    }

    /// Every frame the container declares has to come out of the decoder. Two of them (this
    /// stream's `has_b_frames`) are held back until the decoder is told the input has ended, and
    /// `flush` — which is `avcodec_flush_buffers` — discards them instead of handing them over,
    /// so the tail of the clip silently went missing.
    #[test]
    #[ignore]
    fn a_clip_played_once_decodes_every_frame() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, expected, _) = fixture(tmp.path());

        let rx = stream(&path, false);
        let mut decoded = 0;
        while rx.recv().is_ok() {
            decoded += 1;
        }

        assert_eq!(
            decoded, expected,
            "decoded {decoded} of the container's {expected} frames"
        );
    }

    /// The same holds on every pass of a looping video, and each pass has to start exactly one
    /// container-length on from the last — that is what gives the closing frame its own time on
    /// screen rather than leaving a gap where the dropped frames used to be.
    #[test]
    #[ignore]
    fn a_looping_clip_repeats_whole_at_its_container_length() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, expected, duration) = fixture(tmp.path());

        let passes = 3;
        let rx = stream(&path, true);
        let pts: Vec<Duration> = (0..expected * passes)
            .map(|i| rx.recv().unwrap_or_else(|e| panic!("frame {i}: {e}")).pts)
            .collect();

        // Timestamps carry on across passes, so a pass boundary is not visible as a reset; what
        // must hold is that frame N of one pass is exactly one container-length before frame N of
        // the next. A dropped tail shows up here as a period short by the missing frames.
        for pass in 1..passes {
            for frame in 0..expected {
                let this = pts[pass * expected + frame];
                let previous = pts[(pass - 1) * expected + frame];
                let period = this - previous;

                let drift = period.abs_diff(duration);
                assert!(
                    drift < Duration::from_millis(2),
                    "pass {pass} frame {frame} came {:.1}ms after the same frame of the pass \
                     before, but the clip is {:.1}ms long",
                    period.as_secs_f64() * 1000.0,
                    duration.as_secs_f64() * 1000.0,
                );
            }
        }
    }
}
