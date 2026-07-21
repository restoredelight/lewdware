use std::time::{Duration, Instant};

use anyhow::Result;

use crate::video::{NextFrame, VideoDecoder, VideoFrame, VideoPixelFormat};
use crate::window::gpu_renderer::{GpuRenderer, GpuRendererType};
use crate::window::layer::{BackendInit, CpuFrame, GpuFrame, LayerInit, LayerStatus};
use crate::window::surface::Buffer;

/// A playing video.
pub struct VideoLayer {
    decoder: VideoDecoder,
    backend: VideoBackend,
    last_frame_time: Instant,
    duration: Option<Duration>,
    paused: bool,
}

/// Exactly one of these exists per layer, chosen from the render target it was built for — there
/// is no "GPU renderer that might be absent" to unwrap.
enum VideoBackend {
    Gpu(Box<GpuRenderer>),
    /// ARGB pixel buffer sized to the content area.
    Cpu(Vec<u32>),
}

impl VideoLayer {
    pub fn new(init: &LayerInit<'_>, mut decoder: VideoDecoder) -> Result<Self> {
        let backend = Self::build_backend(init, &decoder);

        decoder.play();

        Ok(Self {
            decoder,
            backend,
            last_frame_time: Instant::now(),
            duration: None,
            paused: false,
        })
    }

    fn build_backend(init: &LayerInit<'_>, decoder: &VideoDecoder) -> VideoBackend {
        let BackendInit::Gpu {
            wgpu,
            format,
            premultiplied_alpha,
            force_opaque,
        } = init.backend
        else {
            let content = init.content;
            return VideoBackend::Cpu(vec![0u32; (content.width * content.height) as usize]);
        };

        VideoBackend::Gpu(Box::new(GpuRenderer::new_video(
            wgpu,
            format,
            decoder.native_width(),
            decoder.native_height(),
            decoder.full_range(),
            decoder.pixel_format(),
            decoder.packed_alpha(),
            init.opacity,
            premultiplied_alpha,
            force_opaque,
        )))
    }

    /// Rebuild GPU/CPU resources after the render target changed backend. The decoder is
    /// backend-independent, so playback continues uninterrupted.
    pub fn rebuild(&mut self, init: &LayerInit<'_>) -> Result<()> {
        self.backend = Self::build_backend(init, &self.decoder);
        Ok(())
    }

    pub fn prepare_gpu(&mut self, frame: &GpuFrame<'_>) -> Result<LayerStatus> {
        let VideoBackend::Gpu(renderer) = &mut self.backend else {
            return Ok(LayerStatus::Idle);
        };

        renderer.set_opacity(frame.wgpu, frame.opacity);

        match self.decoder.next_frame() {
            NextFrame::Ready(video_frame) => {
                if let GpuRendererType::Video(video) = &mut renderer.renderer_type {
                    video.update_video(frame.wgpu, &video_frame);
                }
                Ok(LayerStatus::Draw)
            }
            NextFrame::Finish => Ok(LayerStatus::Finished),
            NextFrame::None => Ok(LayerStatus::Idle),
        }
    }

    pub fn draw_gpu(&self, rpass: &mut wgpu::RenderPass<'static>, frame: &GpuFrame<'_>) {
        let VideoBackend::Gpu(renderer) = &self.backend else {
            return;
        };
        let GpuRendererType::Video(video) = &renderer.renderer_type else {
            return;
        };

        let (pipeline, bind_group) = video.video_pipeline_and_bind_group();
        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, bind_group, &[]);
        rpass.set_bind_group(1, &renderer.window_bind_group, &[]);
        frame.content.set_viewport(rpass);
        rpass.draw(0..4, 0..1);
    }

    pub fn prepare_cpu(&mut self, frame: &CpuFrame) -> Result<LayerStatus> {
        let VideoBackend::Cpu(buffer) = &mut self.backend else {
            return Ok(LayerStatus::Idle);
        };

        match self.decoder.next_frame() {
            NextFrame::Ready(video_frame) => {
                if video_frame.frame.width() > 0 {
                    let convert = match self.decoder.pixel_format() {
                        VideoPixelFormat::Yuv420p => render_yuv420p_to_argb,
                        VideoPixelFormat::Nv12 => render_nv12_to_argb,
                    };
                    convert(
                        &video_frame,
                        buffer,
                        frame.content.width,
                        frame.content.height,
                        self.decoder.native_width(),
                        self.decoder.native_height(),
                        self.decoder.full_range(),
                        self.decoder.packed_alpha(),
                    );
                }
                Ok(LayerStatus::Draw)
            }
            NextFrame::Finish => Ok(LayerStatus::Finished),
            NextFrame::None => Ok(LayerStatus::Idle),
        }
    }

    pub fn draw_cpu(&mut self, buffer: &mut Buffer, frame: &CpuFrame) {
        let VideoBackend::Cpu(pixels) = &self.backend else {
            return;
        };

        buffer.copy_from_u32_buf(
            pixels,
            frame.content.width,
            frame.content.x,
            frame.content.y,
        );
    }

    pub fn pause(&mut self) {
        self.decoder.pause();
        self.paused = true;

        if let Some(duration) = self.duration.take() {
            self.duration = Some(duration - self.last_frame_time.elapsed());
        }
    }

    pub fn play(&mut self) {
        self.paused = false;
        self.last_frame_time = Instant::now();

        self.decoder.play();
    }

    pub fn set_volume(&self, volume: f32) {
        self.decoder.set_volume(volume);
    }

    pub fn set_loop(&self, loop_video: bool) {
        self.decoder.set_loop(loop_video);
    }
}

// BT.709 YCbCr → linear RGB (clipped). Limited range scales Y from [16,235] and Cb/Cr from
// [16,240]; full range (JPEG / yuvj420p) maps [0,255] directly.
fn yuv_to_argb(y: u8, cb: u8, cr: u8, alpha: u8, full_range: bool) -> u32 {
    let (y_f, cb_f, cr_f) = if full_range {
        (
            y as f32 / 255.0,
            cb as f32 / 255.0 - 0.5,
            cr as f32 / 255.0 - 0.5,
        )
    } else {
        (
            (y as f32 - 16.0) / 219.0,
            (cb as f32 - 128.0) / 224.0,
            (cr as f32 - 128.0) / 224.0,
        )
    };
    let r = ((y_f + 1.57480 * cr_f).clamp(0.0, 1.0) * 255.0) as u8;
    let g = ((y_f - 0.18732 * cb_f - 0.46812 * cr_f).clamp(0.0, 1.0) * 255.0) as u8;
    let b = ((y_f + 1.85560 * cb_f).clamp(0.0, 1.0) * 255.0) as u8;
    ((alpha as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Convert a YUV420P `VideoFrame` into ARGB u32 pixels scaled to `(dst_w, dst_h)`.
/// `packed_alpha`: top half = colour, bottom half = alpha-as-luma (same layout as packed MP4).
#[allow(clippy::too_many_arguments)]
fn render_yuv420p_to_argb(
    frame: &VideoFrame,
    dst: &mut [u32],
    dst_w: u32,
    dst_h: u32,
    src_display_w: u32,
    src_display_h: u32,
    full_range: bool,
    packed_alpha: bool,
) {
    let f = &frame.frame;
    let y_data = f.data(0);
    let cb_data = f.data(1);
    let cr_data = f.data(2);
    let y_stride = f.stride(0);
    let cb_stride = f.stride(1);
    let cr_stride = f.stride(2);

    let sw = src_display_w as usize;
    let sh = src_display_h as usize;
    let dw = dst_w as usize;
    let dh = dst_h as usize;

    for dy in 0..dh {
        let sy = (dy * sh) / dh;
        let cy = sy / 2;
        let ay = sy + sh; // alpha row offset (packed only)
        for dx in 0..dw {
            let sx = (dx * sw) / dw;
            let cx = sx / 2;
            let y = y_data[sy * y_stride + sx];
            let cb = cb_data[cy * cb_stride + cx];
            let cr = cr_data[cy * cr_stride + cx];
            let alpha = if packed_alpha {
                y_data[ay * y_stride + sx]
            } else {
                255
            };
            dst[dy * dw + dx] = yuv_to_argb(y, cb, cr, alpha, full_range);
        }
    }
}

/// Convert an NV12 `VideoFrame` into ARGB u32 pixels scaled to `(dst_w, dst_h)`.
/// Handles both software NV12 (from `av_hwframe_transfer_data`) and the packed-alpha layout.
#[allow(clippy::too_many_arguments)]
fn render_nv12_to_argb(
    frame: &VideoFrame,
    dst: &mut [u32],
    dst_w: u32,
    dst_h: u32,
    src_display_w: u32,
    src_display_h: u32,
    full_range: bool,
    packed_alpha: bool,
) {
    let f = &frame.frame;
    let y_data = f.data(0);
    let uv_data = f.data(1);
    let y_stride = f.stride(0);
    let uv_stride = f.stride(1);

    let sw = src_display_w as usize;
    let sh = src_display_h as usize;
    let dw = dst_w as usize;
    let dh = dst_h as usize;

    for dy in 0..dh {
        let sy = (dy * sh) / dh;
        let cy = sy / 2;
        let ay = sy + sh;
        for dx in 0..dw {
            let sx = (dx * sw) / dw;
            let cx = sx / 2;
            let y = y_data[sy * y_stride + sx];
            // UV plane: interleaved Cb Cr pairs
            let cb = uv_data[cy * uv_stride + cx * 2];
            let cr = uv_data[cy * uv_stride + cx * 2 + 1];
            let alpha = if packed_alpha {
                y_data[ay * y_stride + sx]
            } else {
                255
            };
            dst[dy * dw + dx] = yuv_to_argb(y, cb, cr, alpha, full_range);
        }
    }
}
