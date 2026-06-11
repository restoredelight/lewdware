use std::{
    cell::Cell,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use ffmpeg::{codec, format};
use ffmpeg_next::{self as ffmpeg, ffi, frame::Video};

use crate::{audio::AudioPlayer, media::FileOrPath};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoPixelFormat {
    Yuv420p,
    Nv12,
}

thread_local! {
    static HW_PIX_FMT: Cell<i32> = Cell::new(ffi::AVPixelFormat::AV_PIX_FMT_NONE as i32);
}

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

#[cfg(target_os = "linux")]
fn preferred_hw_type() -> ffi::AVHWDeviceType {
    ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI
}

#[cfg(target_os = "macos")]
fn preferred_hw_type() -> ffi::AVHWDeviceType {
    ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX
}

#[cfg(target_os = "windows")]
fn preferred_hw_type() -> ffi::AVHWDeviceType {
    ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D12VA
}

// Returns the hw pixel format (e.g. AV_PIX_FMT_VAAPI) on success.
unsafe fn try_hw_setup(
    ctx: *mut ffi::AVCodecContext,
    hw_type: ffi::AVHWDeviceType,
    #[cfg(target_os = "windows")]
    wgpu_device: Option<&std::sync::Arc<wgpu::Device>>,
) -> Option<ffi::AVPixelFormat> {
    // ctx->codec is NULL before avcodec_open2; find the decoder via codec_id instead.
    let codec_id = unsafe { (*ctx).codec_id };
    let codec = unsafe { ffi::avcodec_find_decoder(codec_id) };
    if codec.is_null() {
        return None;
    }

    let mut hw_pix_fmt = ffi::AVPixelFormat::AV_PIX_FMT_NONE;
    let mut i = 0;
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

    let mut hw_device_ctx: *mut ffi::AVBufferRef = std::ptr::null_mut();

    // On Windows with D3D12VA: inject wgpu's D3D12 device so textures are on the same device.
    #[cfg(target_os = "windows")]
    if let Some(device) = wgpu_device {
        if let Some(ctx_buf) = unsafe { crate::d3d12_import::alloc_d3d12va_device_ctx(device, hw_type) } {
            hw_device_ctx = ctx_buf;
        }
    }

    if hw_device_ctx.is_null() {
        let ret = unsafe {
            ffi::av_hwdevice_ctx_create(
                &mut hw_device_ctx,
                hw_type,
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            )
        };
        if ret < 0 || hw_device_ctx.is_null() {
            return None;
        }
    }

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
/// catches up.
/// * If the video is behind the audio, frames will be skipped until the video is back in sync.
pub struct VideoDecoder {
    receiver: Receiver<Option<VideoFrame>>,
    _video: FileOrPath,
    audio_player: Option<AudioPlayer>,
    tolerance: Duration,
    last_frame_time: Instant,
    frame_duration: Duration,
    video_clock: Duration,
    native_width: u32,
    native_height: u32,
    full_range: bool,
    pixel_format: VideoPixelFormat,
    paused: bool,
    video_duration: Duration,
    pub lag_count: u32,
}

/// On Linux, holds a DRM PRIME mapped frame that keeps the VAAPI surface alive.
/// `frame.data[0]` points to an `AVDRMFrameDescriptor` with DMA-BUF fd(s).
#[cfg(target_os = "linux")]
pub struct DrmPrimeFrame {
    pub frame: Video,
}

/// On macOS, retains a `CVPixelBufferRef` from a VideoToolbox-decoded frame.
/// The pixel buffer contains an IOSurface that Metal textures can be created from directly.
#[cfg(target_os = "macos")]
pub struct VtbFrame {
    pub pixel_buf: *const std::ffi::c_void,
}

#[cfg(target_os = "macos")]
impl Drop for VtbFrame {
    fn drop(&mut self) {
        unsafe { CVPixelBufferRelease(self.pixel_buf) };
    }
}

// Safety: CVPixelBufferRef is safe to move between threads; retain/release are thread-safe.
#[cfg(target_os = "macos")]
unsafe impl Send for VtbFrame {}
#[cfg(target_os = "macos")]
unsafe impl Sync for VtbFrame {}

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferRetain(buffer: *const std::ffi::c_void) -> *const std::ffi::c_void;
    fn CVPixelBufferRelease(buffer: *const std::ffi::c_void);
}

/// On Windows, holds a D3D12VA-decoded frame keeping the texture in the decode pool,
/// along with fence synchronization info for GPU ordering.
#[cfg(target_os = "windows")]
pub struct D3d12Frame {
    /// The D3D12VA hardware frame — keeps the decode surface alive in ffmpeg's pool.
    pub frame: Video,
    /// Raw `ID3D12Resource*` pointer (COM, no extra AddRef — lifetime covered by `frame`).
    pub texture_raw: *mut std::ffi::c_void,
    /// Array slice index within the texture (0 for non-array resources).
    pub index: u32,
    /// Raw `ID3D12Fence*` or null if no fence is present.
    pub fence_raw: *mut std::ffi::c_void,
    /// Fence value to wait for before sampling the texture.
    pub fence_value: u64,
}

// Safety: COM pointers are safe to move across threads; fence/wait are thread-safe.
#[cfg(target_os = "windows")]
unsafe impl Send for D3d12Frame {}
#[cfg(target_os = "windows")]
unsafe impl Sync for D3d12Frame {}

pub struct VideoFrame {
    /// NV12 / YUV420P data in system memory, or empty when using zero-copy.
    pub frame: Video,
    /// DRM PRIME mapped frame for zero-copy GPU import (Linux only).
    /// Wrapped in Arc so DrmImportedTextures can share ownership, keeping the VAAPI
    /// surface alive until the wgpu texture is dropped from drm_ring.
    #[cfg(target_os = "linux")]
    pub drm_prime: Option<Arc<DrmPrimeFrame>>,
    /// Retained CVPixelBuffer for zero-copy IOSurface import (macOS only).
    /// Wrapped in Arc so VtbImportedTextures can share ownership.
    #[cfg(target_os = "macos")]
    pub vtb_frame: Option<Arc<VtbFrame>>,
    /// D3D12VA hardware frame for zero-copy GPU import (Windows only).
    /// Wrapped in Arc so D3d12ImportedTextures can share ownership.
    #[cfg(target_os = "windows")]
    pub d3d12va_frame: Option<Arc<D3d12Frame>>,
    pub pts: Duration,
    pub recycle_tx: SyncSender<Video>,
}

impl Drop for VideoFrame {
    fn drop(&mut self) {
        let dummy = Video::empty();
        let frame = std::mem::replace(&mut self.frame, dummy);
        let _ = self.recycle_tx.try_send(frame);
    }
}

impl VideoDecoder {
    pub fn new(
        video: FileOrPath,
        play_audio: bool,
        loop_video: bool,
        #[cfg(target_os = "windows")]
        wgpu_device: Option<Arc<wgpu::Device>>,
    ) -> Result<Self> {
        let path = video.path();

        let (receiver, video_duration, native_width, native_height, full_range, pixel_format) =
            spawn_video_stream(
                path.to_path_buf(),
                loop_video,
                #[cfg(target_os = "windows")]
                wgpu_device,
            )?;

        let audio_player = if play_audio {
            match AudioPlayer::new(path.to_path_buf(), loop_video, None, None) {
                Ok(audio_player) => Some(audio_player),
                Err(err) => {
                    eprintln!("{err}");
                    None
                }
            }
        } else {
            None
        };

        Ok(Self {
            receiver,
            native_width,
            native_height,
            full_range,
            pixel_format,
            audio_player,
            last_frame_time: Instant::now(),
            frame_duration: Duration::ZERO,
            video_clock: Duration::ZERO,
            tolerance: Duration::from_millis(200),
            _video: video,
            paused: true,
            video_duration,
            lag_count: 0,
        })
    }

    pub fn width(&self) -> u32 {
        self.native_width
    }

    pub fn height(&self) -> u32 {
        self.native_height
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
            match self.receiver.try_recv() {
                Ok(Some(frame)) => {
                    // We got a frame, so if we were waiting for it, resume the audio.
                    if let Some(audio_player) = &self.audio_player {
                        audio_player.play();
                    }

                    // If there's no audio, we just display the frame since needs_next_frame said so.
                    if self.audio_player.is_none() {
                        break frame;
                    }

                    // With audio, we might need to drop frames if we are too far behind.
                    if let Some(audio_player) = &self.audio_player {
                        // Compare directly with total audio position
                        if frame.pts < audio_player.position().saturating_sub(self.tolerance) {
                            continue;
                        }
                    }
                    break frame;
                }
                Ok(None) => {
                    // End of stream from decoder. If looping, it will start again.
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

        if self.audio_player.is_none() {
            if self.video_clock > Duration::ZERO {
                self.frame_duration = next_pts.saturating_sub(self.video_clock);
            }
            // For the very first frame, frame_duration will be zero, but last_frame_time was
            // set at creation, so needs_next_frame will return true immediately.
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
}

pub enum NextFrame {
    Ready(VideoFrame),
    Finish,
    None,
}

struct VideoMetadata {
    duration: Duration,
    native_width: u32,
    native_height: u32,
    full_range: bool,
    pixel_format: VideoPixelFormat,
}

/// Spawn a thread to decode frames from a video.
fn spawn_video_stream(
    path: PathBuf,
    loop_video: bool,
    #[cfg(target_os = "windows")]
    wgpu_device: Option<Arc<wgpu::Device>>,
) -> Result<(Receiver<Option<VideoFrame>>, Duration, u32, u32, bool, VideoPixelFormat)> {
    let (tx, rx) = sync_channel(2);
    let (meta_tx, meta_rx) = sync_channel(1);
    let (recycle_tx, recycle_rx) = sync_channel::<Video>(5);

    thread::spawn(move || {
        let video_duration_inner = match get_video_duration(&path) {
            Ok(duration) => duration,
            Err(err) => {
                eprintln!("Failed to get video duration: {err}");
                return;
            }
        };

        if let Err(err) = decode_video(
            path,
            tx,
            loop_video,
            video_duration_inner,
            meta_tx,
            recycle_rx,
            recycle_tx.clone(),
            #[cfg(target_os = "windows")]
            wgpu_device,
        ) {
            eprintln!("Error decoding video: {}", err);
        }
    });

    let meta = meta_rx
        .recv()
        .context("Failed to receive video metadata from spawn thread")?;

    Ok((rx, meta.duration, meta.native_width, meta.native_height, meta.full_range, meta.pixel_format))
}

// New helper function to get video duration
fn get_video_duration(path: &PathBuf) -> Result<Duration> {
    ffmpeg::init()?;
    let ictx = format::input(path)?;
    let duration_us = ictx.duration(); // Duration in microseconds

    // ffmpeg's duration is in AV_TIME_BASE units, which is 1,000,000 (microseconds).
    let duration_seconds = duration_us as f64 / 1_000_000.0;
    Ok(Duration::from_secs_f64(duration_seconds))
}

/// Converts a hardware-decoded frame to a `VideoFrame`.
/// On Linux, tries DRM PRIME zero-copy first; falls back to `av_hwframe_transfer_data`.
/// Returns `Err(())` if the frame should be skipped (transfer error).
fn hw_frame_to_video_frame(
    decoded: &mut Video,
    recycle_rx: &Receiver<Video>,
    recycle_tx: &SyncSender<Video>,
    pts: Duration,
) -> Result<VideoFrame, ()> {
    #[cfg(target_os = "linux")]
    {
        // Try DRM PRIME zero-copy path first.
        // Set dst->format = DRM_PRIME so av_hwframe_map exports as DMA-BUF rather than NV12.
        let mut drm = Video::empty();
        unsafe { (*drm.as_mut_ptr()).format = ffi::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as i32 };
        let ret = unsafe {
            ffi::av_hwframe_map(
                drm.as_mut_ptr(),
                decoded.as_ptr(),
                ffi::AV_HWFRAME_MAP_READ as i32,
            )
        };
        if ret >= 0 && unsafe { (*drm.as_ptr()).format } == ffi::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as i32 {
            let next = recycle_rx.try_recv().unwrap_or_else(|_| Video::empty());
            *decoded = next;
            return Ok(VideoFrame {
                frame: Video::empty(),
                drm_prime: Some(Arc::new(DrmPrimeFrame { frame: drm })),
                pts,
                recycle_tx: recycle_tx.clone(),
            });
        }
        // av_hwframe_map failed — fall through to av_hwframe_transfer_data.
    }

    #[cfg(target_os = "macos")]
    {
        // VideoToolbox: data[3] is a CVPixelBufferRef wrapping an IOSurface.
        // Retain before swapping out the decoded frame so the buffer outlives it.
        let pixel_buf = unsafe { (*decoded.as_ptr()).data[3] } as *const std::ffi::c_void;
        if !pixel_buf.is_null() {
            unsafe { CVPixelBufferRetain(pixel_buf) };
            let next = recycle_rx.try_recv().unwrap_or_else(|_| Video::empty());
            *decoded = next;
            return Ok(VideoFrame {
                frame: Video::empty(),
                vtb_frame: Some(Arc::new(VtbFrame { pixel_buf })),
                pts,
                recycle_tx: recycle_tx.clone(),
            });
        }
        // pixel_buf null — fall through to av_hwframe_transfer_data.
    }

    #[cfg(target_os = "windows")]
    {
        // D3D12VA: data[0] is a pointer to AVD3D12VAFrame containing the texture + fence.
        let d3d12_frame_ptr = unsafe { (*decoded.as_ptr()).data[0] } as *const crate::d3d12_import::AvD3d12VaFrame;
        if !d3d12_frame_ptr.is_null() {
            let (texture_raw, index, fence_raw, fence_value) = unsafe {
                let f = &*d3d12_frame_ptr;
                let (fence_raw, fence_value) = if !f.sync_ctx.is_null() {
                    ((*f.sync_ctx).fence, (*f.sync_ctx).fence_value)
                } else {
                    (std::ptr::null_mut(), 0u64)
                };
                (f.texture, f.index, fence_raw, fence_value)
            };
            if !texture_raw.is_null() {
                // Swap decoded frame out so ffmpeg can reuse the buffer slot.
                let next = recycle_rx.try_recv().unwrap_or_else(|_| Video::empty());
                let d3d12_holding_frame = std::mem::replace(decoded, next);
                return Ok(VideoFrame {
                    frame: Video::empty(),
                    d3d12va_frame: Some(Arc::new(D3d12Frame {
                        frame: d3d12_holding_frame,
                        texture_raw,
                        index,
                        fence_raw,
                        fence_value,
                    })),
                    pts,
                    recycle_tx: recycle_tx.clone(),
                });
            }
        }
        // Null data — fall through to av_hwframe_transfer_data.
    }

    // CPU fallback: transfer NV12 data to system memory.
    let mut sw = Video::empty();
    let ret = unsafe { ffi::av_hwframe_transfer_data(sw.as_mut_ptr(), decoded.as_ptr(), 0) };
    if ret < 0 {
        eprintln!("av_hwframe_transfer_data failed: {ret}");
        return Err(());
    }
    Ok(VideoFrame {
        frame: sw,
        #[cfg(target_os = "linux")]
        drm_prime: None,
        #[cfg(target_os = "macos")]
        vtb_frame: None,
        #[cfg(target_os = "windows")]
        d3d12va_frame: None,
        pts,
        recycle_tx: recycle_tx.clone(),
    })
}

fn decode_video(
    path: PathBuf,
    tx: SyncSender<Option<VideoFrame>>,
    loop_video: bool,
    video_duration: Duration,
    meta_tx: SyncSender<VideoMetadata>,
    recycle_rx: Receiver<Video>,
    recycle_tx: SyncSender<Video>,
    #[cfg(target_os = "windows")]
    wgpu_device: Option<Arc<wgpu::Device>>,
) -> Result<()> {
    ffmpeg::init()?;
    let mut ictx = format::input(&path)?;
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
    let hw_pix_fmt: Option<ffi::AVPixelFormat> = unsafe {
        let ctx_ptr = context_decoder.as_mut_ptr();
        let hw_type = preferred_hw_type();
        if let Some(fmt) = try_hw_setup(
            ctx_ptr,
            hw_type,
            #[cfg(target_os = "windows")]
            wgpu_device.as_ref(),
        ) {
            HW_PIX_FMT.with(|c| c.set(fmt as i32));
            (*ctx_ptr).get_format = Some(get_hw_format);
            Some(fmt)
        } else {
            None
        }
    };

    let mut decoder = context_decoder.decoder().video()?;

    // Limit thread count to 1 to prevent resource contention when running many video popups.
    // (HW decoders ignore this, but it's a no-op for them.)
    decoder.set_threading(codec::threading::Config {
        kind: codec::threading::Type::Frame,
        count: 1,
    });

    let native_width = decoder.width();
    let native_height = decoder.height();
    let full_range = decoder.color_range() == ffmpeg::color::Range::JPEG;
    let pixel_format = if hw_pix_fmt.is_some() {
        VideoPixelFormat::Nv12
    } else {
        VideoPixelFormat::Yuv420p
    };

    if meta_tx
        .send(VideoMetadata {
            duration: video_duration,
            native_width,
            native_height,
            full_range,
            pixel_format,
        })
        .is_err()
    {
        eprintln!("Failed to send video metadata");
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
                                &recycle_rx,
                                &recycle_tx,
                                pts_duration + current_loop_offset,
                            )
                        } else {
                            let next = recycle_rx.try_recv().unwrap_or_else(|_| Video::empty());
                            let frame = std::mem::replace(&mut decoded, next);
                            Ok(VideoFrame {
                                frame,
                                #[cfg(target_os = "linux")]
                                drm_prime: None,
                                #[cfg(target_os = "macos")]
                                vtb_frame: None,
                                #[cfg(target_os = "windows")]
                                d3d12va_frame: None,
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
                            #[cfg(target_os = "linux")]
                            drm_prime: None,
                            #[cfg(target_os = "macos")]
                            vtb_frame: None,
                            #[cfg(target_os = "windows")]
                            d3d12va_frame: None,
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
                        &recycle_rx,
                        &recycle_tx,
                        pts_duration + current_loop_offset,
                    )
                } else {
                    let next = recycle_rx.try_recv().unwrap_or_else(|_| Video::empty());
                    let frame = std::mem::replace(&mut decoded, next);
                    Ok(VideoFrame {
                        frame,
                        #[cfg(target_os = "linux")]
                        drm_prime: None,
                        #[cfg(target_os = "macos")]
                        vtb_frame: None,
                        #[cfg(target_os = "windows")]
                        d3d12va_frame: None,
                        pts: pts_duration + current_loop_offset,
                        recycle_tx: recycle_tx.clone(),
                    })
                }
            } else {
                let next = recycle_rx.try_recv().unwrap_or_else(|_| Video::empty());
                let frame = std::mem::replace(&mut decoded, next);
                Ok(VideoFrame {
                    frame,
                    #[cfg(target_os = "linux")]
                    drm_prime: None,
                    #[cfg(target_os = "macos")]
                    vtb_frame: None,
                    #[cfg(target_os = "windows")]
                    d3d12va_frame: None,
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

        if !loop_video {
            return Ok(());
        }

        ictx.seek(0, ..0)?;
        decoder.flush();
        current_loop_offset += last_pts_duration + frame_duration;
    }

    Ok(())
}

