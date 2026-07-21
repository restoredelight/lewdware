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
/// Audio is used as the master clock for synchronization.
///
/// * If the video is ahead of the audio, playback will pause on the current frame until the audio
///   catches up.
/// * If the video is behind the audio, frames will be skipped until the video is back in sync.
pub struct VideoDecoder {
    rx: Receiver<Option<VideoFrame>>,
    audio_player: Option<AudioPlayer>,
    tolerance: Duration,
    last_frame_time: Instant,
    frame_duration: Duration,
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
                Ok(audio_player) => Some(audio_player),
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
            last_frame_time: Instant::now(),
            frame_duration: Duration::ZERO,
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

        if !self.needs_next_frame() {
            // Video is ahead of or in sync with audio. Ensure audio is playing.
            if let Some(audio_player) = &self.audio_player {
                audio_player.play();
            }
            return NextFrame::None;
        }

        let frame = loop {
            match self.rx.try_recv() {
                Ok(Some(frame)) => {
                    if let Some(audio_player) = &self.audio_player {
                        audio_player.play();
                    }

                    if self.audio_player.is_none() {
                        break frame;
                    }

                    // Drop frames if we are too far behind
                    if let Some(audio_player) = &self.audio_player {
                        if frame.pts < audio_player.position().saturating_sub(self.tolerance) {
                            continue;
                        }
                    }
                    break frame;
                }
                Ok(None) => {
                    return NextFrame::None;
                }
                Err(TryRecvError::Empty) => {
                    self.lag_count += 1;
                    // The decoder is lagging behind the audio, so we pause the audio to wait for it.
                    if let Some(audio_player) = &self.audio_player {
                        audio_player.pause();
                    }
                    return NextFrame::None;
                }
                Err(TryRecvError::Disconnected) => return NextFrame::Finish,
            }
        };

        let next_pts = frame.pts;

        if self.audio_player.is_none() && self.video_clock > Duration::ZERO {
            self.frame_duration = next_pts.saturating_sub(self.video_clock);
        }

        self.video_clock = next_pts;
        self.last_frame_time = Instant::now();

        NextFrame::Ready(frame)
    }

    fn needs_next_frame(&self) -> bool {
        match &self.audio_player {
            Some(audio_player) => {
                // Compare directly with total audio position
                audio_player.position() > self.video_clock
            }
            None => self.last_frame_time.elapsed() >= self.frame_duration,
        }
    }

    pub fn pause(&mut self) {
        if let Some(audio_player) = &self.audio_player {
            audio_player.pause();
        }
        self.paused = true;
    }

    pub fn play(&mut self) {
        if let Some(audio_player) = &self.audio_player {
            audio_player.play();
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

struct VideoMetadata {
    native_width: u32,
    native_height: u32,
    full_range: bool,
    pixel_format: VideoPixelFormat,
}

struct VideoStream {
    rx: Receiver<Option<VideoFrame>>,
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
) -> Result<VideoFrame, ()> {
    if let Some(frame) = HardwareFrame::from_decoder_frame(decoded) {
        return Ok(VideoFrame {
            frame: Video::empty(),
            hardware_frame: Some(frame),
            pts,
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
        recycle_tx: recycle_tx.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn decode_video(
    source: MediaSource,
    tx: SyncSender<Option<VideoFrame>>,
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

    'main: loop {
        for (stream, packet) in ictx.packets() {
            if stream.index() == stream_index {
                decoder.send_packet(&packet)?;
                while decoder.receive_frame(&mut decoded).is_ok() {
                    let pts_raw = decoded.pts().unwrap_or(0);
                    let pts_seconds = pts_raw as f64 * (time_base.0 as f64 / time_base.1 as f64);
                    let pts_duration = Duration::from_secs_f64(pts_seconds);
                    last_pts_duration = pts_duration;

                    let video_frame = if let Some(hw_fmt) = hw_pix_fmt {
                        if unsafe { (*decoded.as_ptr()).format } == hw_fmt as i32 {
                            hw_frame_to_video_frame(
                                &mut decoded,
                                &recycle_tx,
                                pts_duration + current_loop_offset,
                            )
                        } else {
                            let next = recycle_rx.try_recv().unwrap_or_else(|_| Video::empty());
                            let frame = std::mem::replace(&mut decoded, next);
                            Ok(VideoFrame {
                                frame,
                                hardware_frame: None,
                                pts: pts_duration + current_loop_offset,
                                recycle_tx: recycle_tx.clone(),
                            })
                        }
                    } else {
                        // Software decode: swap decoded out so ffmpeg can reuse the buffer.
                        let next = recycle_rx.try_recv().unwrap_or_else(|_| Video::empty());
                        let frame = std::mem::replace(&mut decoded, next);
                        Ok(VideoFrame {
                            frame,
                            hardware_frame: None,
                            pts: pts_duration + current_loop_offset,
                            recycle_tx: recycle_tx.clone(),
                        })
                    };

                    let video_frame = match video_frame {
                        Ok(f) => f,
                        Err(_) => continue,
                    };

                    if tx.send(Some(video_frame)).is_err() {
                        break 'main;
                    }
                }
            }
        }

        decoder.flush();

        while decoder.receive_frame(&mut decoded).is_ok() {
            let pts_raw = decoded.pts().unwrap_or(0);
            let pts_seconds = pts_raw as f64 * (time_base.0 as f64 / time_base.1 as f64);
            let pts_duration = Duration::from_secs_f64(pts_seconds);
            last_pts_duration = pts_duration;

            let video_frame = if let Some(hw_fmt) = hw_pix_fmt {
                if unsafe { (*decoded.as_ptr()).format } == hw_fmt as i32 {
                    hw_frame_to_video_frame(
                        &mut decoded,
                        &recycle_tx,
                        pts_duration + current_loop_offset,
                    )
                } else {
                    let next = recycle_rx.try_recv().unwrap_or_else(|_| Video::empty());
                    let frame = std::mem::replace(&mut decoded, next);
                    Ok(VideoFrame {
                        frame,
                        hardware_frame: None,
                        pts: pts_duration + current_loop_offset,
                        recycle_tx: recycle_tx.clone(),
                    })
                }
            } else {
                let next = recycle_rx.try_recv().unwrap_or_else(|_| Video::empty());
                let frame = std::mem::replace(&mut decoded, next);
                Ok(VideoFrame {
                    frame,
                    hardware_frame: None,
                    pts: pts_duration + current_loop_offset,
                    recycle_tx: recycle_tx.clone(),
                })
            };

            let video_frame = match video_frame {
                Ok(f) => f,
                Err(_) => continue,
            };

            if tx.send(Some(video_frame)).is_err() {
                break 'main;
            }
        }

        if tx.send(None).is_err() {
            break 'main;
        }

        if !loop_video.load(Ordering::Relaxed) {
            return Ok(());
        }

        ictx.seek(0, ..0)?;
        decoder.flush();
        current_loop_offset += last_pts_duration + frame_duration;
    }

    Ok(())
}
