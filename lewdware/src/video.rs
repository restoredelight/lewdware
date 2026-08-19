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

    pub fn set_volume(&mut self, volume: f32) {
        if let Some(audio_player) = &mut self.audio_player {
            audio_player.set_volume(volume);
        }
    }

    pub fn start_volume_fade(&mut self, id: u64, opts: Option<crate::lua::VolumeFadeOpts>) {
        if let Some(audio_player) = &mut self.audio_player {
            audio_player.start_volume_fade(id, opts);
        }
    }

    pub fn update_volume_fade(&mut self) -> Option<u64> {
        self.audio_player.as_mut()?.update_volume_fade()
    }

    pub fn is_fading_volume(&self) -> bool {
        self.audio_player
            .as_ref()
            .is_some_and(|player| player.is_fading_volume())
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

#[cfg(test)]
mod pacing;

#[cfg(test)]
mod sidecar;
