//! Handles the different popup windows. We draw to image windows using `softbuffer` (which works
//! on the CPU), and render videos using `pixels` (which works on the GPU, using `wgpu`). Prompt
//! windows are also drawn using `wgpu`. We do this because having too many GPU rendered windows
//! can exhaust the device's VRAM, causing a crash. However, we still want to use the GPU to render
//! videos for smooth playback.

use std::{
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use egui::{RichText, TextEdit};
use egui_software_backend::BufferMutRef;
use tiny_skia::{Color, IntSize, Paint, PathBuilder, Pixmap, PixmapMut, Rect, Stroke, Transform};
use tokio::sync::mpsc;
use winit::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, PhysicalUnit},
    event::{Touch, WindowEvent},
    window::Window as WinitWindow,
};

use crate::{
    egui::{EguiCPUWindow, WgpuState},
    error::{LewdwareError, MonitorError},
    header::{HEADER_HEIGHT, Header},
    lua::{self, ChoiceWindowOption, Coord, Easing, MoveOpts},
    media::{ImageData, VideoData},
    video::{NextFrame, VideoDecoder, VideoFrame, VideoPixelFormat},
};

#[cfg(target_os = "linux")]
use crate::drm_import::DrmImportedTextures;
#[cfg(target_os = "macos")]
use crate::vtb_import::VtbImportedTextures;
#[cfg(target_os = "windows")]
use crate::d3d12_import::D3d12ImportedTextures;

pub enum WindowType<'a> {
    Image(ImageWindow<'a>),
    Video(VideoWindow<'a>),
    Prompt(PromptWindow<'a>),
    Choice(ChoiceWindow<'a>),
}

impl<'a> WindowType<'a> {
    pub fn inner_window(&self) -> &InnerWindow<'_> {
        match self {
            Self::Image(image_window) => &image_window.inner_window,
            Self::Video(video_window) => &video_window.inner_window,
            Self::Prompt(prompt_window) => &prompt_window.inner_window,
            Self::Choice(choice_window) => &choice_window.inner_window,
        }
    }

    pub fn inner_window_mut(&mut self) -> &mut InnerWindow<'a> {
        match self {
            Self::Image(image_window) => &mut image_window.inner_window,
            Self::Video(video_window) => &mut video_window.inner_window,
            Self::Prompt(prompt_window) => &mut prompt_window.inner_window,
            Self::Choice(choice_window) => &mut choice_window.inner_window,
        }
    }
}

enum VideoFrameTextures {
    Yuv420p {
        y_texture: wgpu::Texture,
        cb_texture: wgpu::Texture,
        cr_texture: wgpu::Texture,
        bind_group: wgpu::BindGroup,
        pipeline: Arc<wgpu::RenderPipeline>,
    },
    Nv12 {
        y_texture: wgpu::Texture,
        uv_texture: wgpu::Texture,
        bind_group: wgpu::BindGroup,
        pipeline: Arc<wgpu::RenderPipeline>,
    },
}

struct VideoRenderer {
    frame_textures: VideoFrameTextures,
    video_width: u32,
    video_height: u32,
    // CPU-rendered UI / decorations overlay
    ui_texture: wgpu::Texture,
    ui_bind_group: wgpu::BindGroup,
    ui_pipeline: Arc<wgpu::RenderPipeline>,
    // DMA-BUF imported textures ring (Linux only): keeps last N frames alive for GPU safety.
    // The most recent entry (back) is the active frame for rendering.
    #[cfg(target_os = "linux")]
    drm_ring: std::collections::VecDeque<DrmImportedTextures>,
    // IOSurface-backed Metal textures ring (macOS only): same purpose as drm_ring.
    #[cfg(target_os = "macos")]
    vtb_ring: std::collections::VecDeque<VtbImportedTextures>,
    // D3D12VA-decoded NV12 textures ring (Windows only): same purpose as drm_ring.
    #[cfg(target_os = "windows")]
    d3d12_ring: std::collections::VecDeque<D3d12ImportedTextures>,
}

fn make_r8_texture(device: &wgpu::Device, label: &str, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn make_rg8_texture(device: &wgpu::Device, label: &str, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

impl VideoRenderer {
    fn new(
        wgpu_state: &WgpuState,
        format: wgpu::TextureFormat,
        video_width: u32,
        video_height: u32,
        full_range: bool,
        pixel_format: VideoPixelFormat,
        ui_width: u32,
        ui_height: u32,
    ) -> Self {
        let device = &wgpu_state.device;
        let chroma_w = (video_width + 1) / 2;
        let chroma_h = (video_height + 1) / 2;

        let frame_textures = match pixel_format {
            VideoPixelFormat::Yuv420p => {
                let y_texture  = make_r8_texture(device, "Y Plane",  video_width, video_height);
                let cb_texture = make_r8_texture(device, "Cb Plane", chroma_w,    chroma_h);
                let cr_texture = make_r8_texture(device, "Cr Plane", chroma_w,    chroma_h);

                let y_view  = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
                let cb_view = cb_texture.create_view(&wgpu::TextureViewDescriptor::default());
                let cr_view = cr_texture.create_view(&wgpu::TextureViewDescriptor::default());

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("YUV Bind Group"),
                    layout: &wgpu_state.yuv_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&y_view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&cb_view) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&cr_view) },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&wgpu_state.sampler) },
                    ],
                });

                let pipeline = wgpu_state.get_yuv_pipeline(format, full_range);
                VideoFrameTextures::Yuv420p { y_texture, cb_texture, cr_texture, bind_group, pipeline }
            }
            VideoPixelFormat::Nv12 => {
                let y_texture  = make_r8_texture(device,  "Y Plane",  video_width, video_height);
                let uv_texture = make_rg8_texture(device, "UV Plane", chroma_w,    chroma_h);

                let y_view  = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
                let uv_view = uv_texture.create_view(&wgpu::TextureViewDescriptor::default());

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("NV12 Bind Group"),
                    layout: &wgpu_state.nv12_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&y_view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&uv_view) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&wgpu_state.sampler) },
                    ],
                });

                let pipeline = wgpu_state.get_nv12_pipeline(format, full_range);
                VideoFrameTextures::Nv12 { y_texture, uv_texture, bind_group, pipeline }
            }
        };

        let ui_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("UI Texture"),
            size: wgpu::Extent3d { width: ui_width, height: ui_height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let ui_view = ui_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let ui_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("UI Bind Group"),
            layout: &wgpu_state.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&ui_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&wgpu_state.sampler) },
            ],
        });

        let ui_pipeline = wgpu_state.get_pipeline(format);

        Self {
            frame_textures,
            video_width,
            video_height,
            ui_texture,
            ui_bind_group,
            ui_pipeline,
            #[cfg(target_os = "linux")]
            drm_ring: std::collections::VecDeque::new(),
            #[cfg(target_os = "macos")]
            vtb_ring: std::collections::VecDeque::new(),
            #[cfg(target_os = "windows")]
            d3d12_ring: std::collections::VecDeque::new(),
        }
    }

    fn update_ui(&self, queue: &wgpu::Queue, data: &[u8], width: u32, height: u32) {
        upload_texture_data(queue, &self.ui_texture, data, width, height, width * 4, 4);
    }

    fn update_video(
        &mut self,
        wgpu_state: &WgpuState,
        frame: &VideoFrame,
    ) {
        // On Linux, try zero-copy DRM PRIME import first.
        #[cfg(target_os = "linux")]
        if let Some(drm_prime) = &frame.drm_prime {
            if let VideoFrameTextures::Nv12 { pipeline, .. } = &self.frame_textures {
                let pipeline = pipeline.clone();
                if let Some(imported) = crate::drm_import::try_import_drm_prime(
                    &wgpu_state.device,
                    drm_prime,
                    self.video_width,
                    self.video_height,
                    &wgpu_state.nv12_bind_group_layout,
                    &wgpu_state.sampler,
                ) {
                    self.drm_ring.push_back(imported);
                    // Keep at most 3 frames in the ring so GPU can finish with older ones.
                    while self.drm_ring.len() > 3 {
                        self.drm_ring.pop_front();
                    }
                    return;
                }
            }
        }

        // CPU path: clear the DRM ring (DRM PRIME not in use for this frame).
        #[cfg(target_os = "linux")]
        self.drm_ring.clear();

        // On macOS, try VideoToolbox IOSurface zero-copy import.
        #[cfg(target_os = "macos")]
        if let Some(vtb_frame) = &frame.vtb_frame {
            if let VideoFrameTextures::Nv12 { .. } = &self.frame_textures {
                if let Some(imported) = crate::vtb_import::try_import_vtb_frame(
                    &wgpu_state.device,
                    vtb_frame,
                    self.video_width,
                    self.video_height,
                    &wgpu_state.nv12_bind_group_layout,
                    &wgpu_state.sampler,
                ) {
                    self.vtb_ring.push_back(imported);
                    while self.vtb_ring.len() > 3 {
                        self.vtb_ring.pop_front();
                    }
                    return;
                }
            }
        }
        // CPU path: clear the VTB ring (IOSurface not in use for this frame).
        #[cfg(target_os = "macos")]
        self.vtb_ring.clear();

        // On Windows, try D3D12VA zero-copy NV12 import.
        #[cfg(target_os = "windows")]
        if let Some(d3d12_frame) = &frame.d3d12va_frame {
            if let VideoFrameTextures::Nv12 { .. } = &self.frame_textures {
                if let Some(imported) = crate::d3d12_import::try_import_d3d12va_frame(
                    &wgpu_state.device,
                    d3d12_frame.clone(),
                    self.video_width,
                    self.video_height,
                    &wgpu_state.nv12_bind_group_layout,
                    &wgpu_state.sampler,
                ) {
                    self.d3d12_ring.push_back(imported);
                    while self.d3d12_ring.len() > 3 {
                        self.d3d12_ring.pop_front();
                    }
                    return;
                }
            }
        }
        // CPU path: clear the D3D12 ring (zero-copy not in use for this frame).
        #[cfg(target_os = "windows")]
        self.d3d12_ring.clear();

        if frame.frame.width() == 0 {
            return;
        }

        let queue = &wgpu_state.queue;
        let w = frame.frame.width();
        let h = frame.frame.height();
        let chroma_w = (w + 1) / 2;
        let chroma_h = (h + 1) / 2;

        match &self.frame_textures {
            VideoFrameTextures::Yuv420p { y_texture, cb_texture, cr_texture, .. } => {
                upload_texture_data(queue, y_texture,  frame.frame.data(0), w,        h,        frame.frame.stride(0) as u32, 1);
                upload_texture_data(queue, cb_texture, frame.frame.data(1), chroma_w, chroma_h, frame.frame.stride(1) as u32, 1);
                upload_texture_data(queue, cr_texture, frame.frame.data(2), chroma_w, chroma_h, frame.frame.stride(2) as u32, 1);
            }
            VideoFrameTextures::Nv12 { y_texture, uv_texture, .. } => {
                // NV12: plane 0 = Y (full size, 1 byte/texel), plane 1 = UV (chroma size, 2 bytes/texel)
                upload_texture_data(queue, y_texture,  frame.frame.data(0), w,        h,        frame.frame.stride(0) as u32, 1);
                upload_texture_data(queue, uv_texture, frame.frame.data(1), chroma_w, chroma_h, frame.frame.stride(1) as u32, 2);
            }
        }
    }

    fn video_pipeline_and_bind_group(&self) -> (&wgpu::RenderPipeline, &wgpu::BindGroup) {
        // On Linux, prefer the DRM-imported bind group from the most recent frame.
        #[cfg(target_os = "linux")]
        if let Some(latest) = self.drm_ring.back() {
            let pipeline = match &self.frame_textures {
                VideoFrameTextures::Nv12 { pipeline, .. } => pipeline.as_ref(),
                VideoFrameTextures::Yuv420p { pipeline, .. } => pipeline.as_ref(),
            };
            return (pipeline, &latest.bind_group);
        }
        // On macOS, prefer the IOSurface-backed bind group from the most recent frame.
        #[cfg(target_os = "macos")]
        if let Some(latest) = self.vtb_ring.back() {
            let pipeline = match &self.frame_textures {
                VideoFrameTextures::Nv12 { pipeline, .. } => pipeline.as_ref(),
                VideoFrameTextures::Yuv420p { pipeline, .. } => pipeline.as_ref(),
            };
            return (pipeline, &latest.bind_group);
        }
        // On Windows, prefer the D3D12VA-imported bind group from the most recent frame.
        #[cfg(target_os = "windows")]
        if let Some(latest) = self.d3d12_ring.back() {
            let pipeline = match &self.frame_textures {
                VideoFrameTextures::Nv12 { pipeline, .. } => pipeline.as_ref(),
                VideoFrameTextures::Yuv420p { pipeline, .. } => pipeline.as_ref(),
            };
            return (pipeline, &latest.bind_group);
        }
        match &self.frame_textures {
            VideoFrameTextures::Yuv420p { pipeline, bind_group, .. } => (pipeline, bind_group),
            VideoFrameTextures::Nv12    { pipeline, bind_group, .. } => (pipeline, bind_group),
        }
    }
}

enum Surface<'a> {
    Wgpu {
        surface: wgpu::Surface<'a>,
        surface_config: wgpu::SurfaceConfiguration,
        frame_buffer: Vec<u8>,
        texture: wgpu::Texture,
        bind_group: wgpu::BindGroup,
        video_renderer: Option<VideoRenderer>,
    },
    Softbuffer {
        _context: softbuffer::Context<Arc<WinitWindow>>,
        surface: softbuffer::Surface<Arc<WinitWindow>, Arc<WinitWindow>>,
    },
}

impl<'a> Surface<'a> {
    pub fn is_gpu(&self) -> bool {
        matches!(self, Self::Wgpu { .. })
    }

    fn buffer(&mut self) -> Result<Buffer<'_>> {
        match self {
            Surface::Wgpu { frame_buffer, surface_config, .. } => {
                let dest = PixmapMut::from_bytes(frame_buffer, surface_config.width, surface_config.height)
                    .context("Invalid pixmap size")?;

                Ok(Buffer::Pixmap(dest))
            }
            Surface::Softbuffer { _context, surface } => {
                let buffer = surface.buffer_mut().map_err(|err| anyhow!("{err}"))?;

                Ok(Buffer::Softbuffer(buffer))
            }
        }
    }
}

enum Buffer<'a> {
    Pixmap(PixmapMut<'a>),
    Softbuffer(softbuffer::Buffer<'a, Arc<WinitWindow>, Arc<WinitWindow>>),
}

impl<'a> Buffer<'a> {
    fn copy_from_slice(&mut self, start: usize, src: &[u8]) {
        match self {
            Buffer::Pixmap(pixmap) => {
                let start = start * 4;
                pixmap.data_mut()[start..(start + src.len())].copy_from_slice(src);
            }
            Buffer::Softbuffer(buffer) => {
                for (index, pixel) in src.chunks_exact(4).enumerate() {
                    let r = pixel[0] as u32;
                    let g = pixel[1] as u32;
                    let b = pixel[2] as u32;
                    let a = pixel[3] as u32;

                    buffer[start + index] = (a << 24) | (r << 16) | (g << 8) | b;
                }
            }
        }
    }

    fn copy_from_pixmap(&mut self, source: &Pixmap, x: u32, y: u32) {
        let dst_width = self.width();
        let offset = (y * dst_width) as usize;
        let src_data = source.data();

        if x == 0 && dst_width == source.width() {
            self.copy_from_slice(offset, src_data);
        } else {
            for (i, row) in src_data
                .chunks_exact(source.width() as usize * 4)
                .enumerate()
            {
                let index = offset + (dst_width * i as u32 + x) as usize;

                self.copy_from_slice(index, row);
            }
        }
    }

    fn copy_from_frame(&mut self, frame: &VideoFrame, x: u32, y: u32) {
        let frame_width = frame.frame.width() as usize;
        let frame_height = frame.frame.height() as usize;
        let line_size = frame.frame.stride(0); // Bytes per row
        let data = frame.frame.data(0);

        let copy_width = frame_width.min(self.width().saturating_sub(x) as usize);
        let copy_height = frame_height.min(self.height().saturating_sub(y) as usize);

        let dst_width = self.width();
        let offset = (y * dst_width) as usize;

        for row_index in 0..copy_height {
            let src_start = row_index * line_size;
            let src_end = src_start + copy_width * 4;

            let index = offset + (dst_width * row_index as u32 + x) as usize;

            self.copy_from_slice(index, &data[src_start..src_end]);
        }
    }

    fn copy_from_u32_buf(&mut self, src: &[u32], width: u32, x: u32, y: u32) {
        let offset = (y * self.width()) as usize;
        let dst_width = self.width();

        match self {
            Buffer::Pixmap(pixmap) => {
                let data = pixmap.data_mut();
                let src_bytes = bytemuck::cast_slice::<u32, u8>(src);
                let row_bytes = width as usize * 4;
                for (i, row) in src_bytes.chunks_exact(row_bytes).enumerate() {
                    let index = offset + (dst_width * i as u32 + x) as usize;
                    let byte_index = index * 4;
                    data[byte_index..(byte_index + row.len())].copy_from_slice(row);
                }
            }
            Buffer::Softbuffer(buffer) => {
                for (i, row) in src.chunks_exact(width as usize).enumerate() {
                    let index = offset + (dst_width * i as u32 + x) as usize;

                    buffer[index..(index + row.len())].copy_from_slice(row);
                }
            }
        }
    }

    fn draw_border(&mut self) {
        match self {
            Buffer::Pixmap(pixmap) => {
                let border = PathBuilder::from_rect(
                    Rect::from_xywh(0.0, 0.0, pixmap.width() as f32, pixmap.height() as f32)
                        .unwrap(),
                );
                let mut paint = Paint::default();
                paint.set_color(Color::BLACK);
                pixmap.stroke_path(
                    &border,
                    &paint,
                    &Stroke::default(),
                    Transform::default(),
                    None,
                );
            }
            Buffer::Softbuffer(buffer) => {
                let black = Color::BLACK.to_color_u8();
                let color = ((black.alpha() as u32) << 24)
                    | ((black.red() as u32) << 16)
                    | ((black.green() as u32) << 8)
                    | (black.blue() as u32);
                let width = buffer.width().get() as usize;
                let height = buffer.height().get() as usize;

                for i in 0..width {
                    buffer[i] = color;
                    buffer[width * (height - 1) + i] = color;
                }

                for i in 0..height {
                    buffer[i * width] = color;
                    buffer[i * width + (width - 1)] = color;
                }
            }
        }
    }

    fn width(&self) -> u32 {
        match self {
            Buffer::Pixmap(pixmap) => pixmap.width(),
            Buffer::Softbuffer(buffer) => buffer.width().get(),
        }
    }

    fn height(&self) -> u32 {
        match self {
            Buffer::Pixmap(pixmap) => pixmap.height(),
            Buffer::Softbuffer(buffer) => buffer.height().get(),
        }
    }
}

/// A window displaying an image. Image windows are rendered using softbuffer.
pub struct ImageWindow<'a> {
    inner_window: InnerWindow<'a>,
    image: Option<ImageData>,
}

impl<'a> ImageWindow<'a> {
    /// Create a new image window.
    ///
    /// * `close_button`: Whether to display a close button on the window.
    /// * `moving`: Whether to move the window around the screen.
    pub fn new(inner_window: InnerWindow<'a>, image: ImageData) -> Result<Self> {
        Ok(Self {
            inner_window,
            image: Some(image),
        })
    }

    pub fn draw(&mut self) -> Result<()> {
        let mut render = false;

        self.inner_window.start_render()?;
        render = render || self.inner_window.render_decorations()?;

        if let Some(image) = self.image.take() {
            let width = image.width();
            let height = image.height();

            let image_pixmap =
                Pixmap::from_vec(image.into_vec(), IntSize::from_wh(width, height).unwrap())
                    .unwrap();

            self.inner_window.render_pixmap(&image_pixmap)?;

            render = true;
        }

        if render {
            self.inner_window.present()?;
        }

        Ok(())
    }
}

fn calculate_size(
    window: &Arc<WinitWindow>,
    decorations: bool,
) -> (PhysicalSize<u32>, PhysicalSize<u32>) {
    let outer_size = window.inner_size();

    let inner_size = if decorations {
        let logical_size = outer_size.to_logical::<u32>(window.scale_factor());
        LogicalSize::new(
            logical_size.width - 2,
            logical_size.height - 2 - HEADER_HEIGHT,
        )
        .to_physical(window.scale_factor())
    } else {
        outer_size.clone()
    };

    (inner_size, outer_size)
}

/// A video popup, rendered using pixels.
pub struct VideoWindow<'a> {
    inner_window: InnerWindow<'a>,
    video_player: VideoDecoder,
    last_frame_time: Instant,
    duration: Option<Duration>,
    loop_video: bool,
    paused: bool,
    window_id: u32,
    frames_rendered: u32,
    total_upload_time: Duration,
}

fn init_softbuffer(
    window: Arc<WinitWindow>,
) -> Result<(
    softbuffer::Context<Arc<WinitWindow>>,
    softbuffer::Surface<Arc<WinitWindow>, Arc<WinitWindow>>,
)> {
    let context = softbuffer::Context::new(window.clone()).map_err(|err| anyhow!("{}", err))?;
    let surface =
        softbuffer::Surface::new(&context, window.clone()).map_err(|err| anyhow!("{}", err))?;

    Ok((context, surface))
}

impl<'a> VideoWindow<'a> {
    /// Create a new video popup.
    ///
    /// * `close_button`: Whether to display a close button on the window.
    /// * `play_audio`: Whether to play the video's audio.
    pub fn new(
        mut inner_window: InnerWindow<'a>,
        mut video_player: VideoDecoder,
        loop_video: bool,
    ) -> anyhow::Result<Self> {
        inner_window.init_video_texture(
            video_player.native_width(),
            video_player.native_height(),
            video_player.full_range(),
            video_player.pixel_format(),
        )?;

        video_player.play();

        inner_window.window.request_redraw();

        static NEXT_WINDOW_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        let window_id = NEXT_WINDOW_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(Self {
            inner_window,
            video_player,
            last_frame_time: Instant::now(),
            duration: None,
            loop_video,
            paused: false,
            window_id,
            frames_rendered: 0,
            total_upload_time: Duration::ZERO,
        })
    }

    pub fn update(&mut self) -> Result<bool> {
        let mut render = false;

        self.inner_window.start_render()?;

        render = render || self.inner_window.render_decorations()?;

        match self.video_player.next_frame() {
            NextFrame::Ready(frame) => {
                let start_upload = Instant::now();
                self.inner_window.render_frame(&frame)?;
                let upload_duration = start_upload.elapsed();

                self.frames_rendered += 1;
                self.total_upload_time += upload_duration;

                if self.frames_rendered >= 100 {
                    let avg_upload = self.total_upload_time.as_secs_f64() * 1000.0 / self.frames_rendered as f64;
                    println!(
                        "[Video Window {}] Avg GPU Upload: {:.2}ms | Lags: {}",
                        self.window_id,
                        avg_upload,
                        self.video_player.lag_count
                    );
                    self.frames_rendered = 0;
                    self.total_upload_time = Duration::ZERO;
                    self.video_player.lag_count = 0;
                }

                render = true;
            }
            NextFrame::Finish => {
                return Ok(true);
            }
            NextFrame::None => {
                // println!("No frame received");
            }
        }

        if render {
            self.inner_window.present()?;
        }

        Ok(false)
    }

    pub fn pause(&mut self) {
        self.video_player.pause();
        self.paused = true;

        if let Some(duration) = self.duration.take() {
            self.duration = Some(duration - self.last_frame_time.elapsed());
        }
    }

    pub fn play(&mut self) {
        self.paused = false;
        self.last_frame_time = Instant::now();

        self.video_player.play();
    }
}

/// A prompt window, rendered using `egui`.
pub struct PromptWindow<'a> {
    inner_window: InnerWindow<'a>,
    egui_window: EguiCPUWindow,
    text: Option<String>,
    placeholder: Option<String>,
    value: String,
}

impl<'a> PromptWindow<'a> {
    pub fn new(
        inner_window: InnerWindow<'a>,
        text: Option<String>,
        placeholder: Option<String>,
        initial_value: Option<String>,
    ) -> Result<Self> {
        let egui_window = EguiCPUWindow::new(
            inner_window.window.clone(),
            inner_window.is_gpu(),
            inner_window.transparent,
        )?;

        Ok(Self {
            inner_window,
            egui_window,
            text,
            placeholder,
            value: initial_value.unwrap_or_default(),
        })
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        let event = if self.inner_window.decorations {
            &translate_event_position(event.clone(), self.inner_window.window.scale_factor())
        } else {
            event
        };

        self.egui_window.handle_event(event);
    }

    pub fn render(&mut self) -> Result<()> {
        self.inner_window.start_render()?;

        let id = self.inner_window.window.id();
        let lua_event_tx = self.inner_window.lua_event_tx.clone();

        self.inner_window.render_with_softbuffer_buffer(|buffer| {
            self.egui_window.redraw(buffer, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        ui.heading("Repeat after me");
                        ui.add_space(20.0);

                        if let Some(text) = &self.text {
                            ui.label(RichText::new(text).heading());
                        }

                        let mut prompt = TextEdit::singleline(&mut self.value);

                        if let Some(placeholder) = &self.placeholder {
                            prompt = prompt.hint_text(placeholder);
                        };

                        let response = ui.add(prompt);
                        response.request_focus();

                        ui.add_space(ui.available_height() - 50.0);

                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("Submit")).clicked() {
                                if let Err(err) = lua_event_tx.send(lua::Event::PromptSubmit {
                                    id,
                                    text: self.value.clone(),
                                }) {
                                    eprintln!("{err}");
                                }
                            }
                        })
                    })
                });
            })
        })?;

        self.inner_window.present()?;

        Ok(())
    }

    pub fn set_text(&mut self, text: Option<String>) {
        self.text = text;
        self.inner_window.window.request_redraw();
    }

    pub fn set_value(&mut self, value: Option<String>) {
        self.value = value.unwrap_or_default();
        self.inner_window.window.request_redraw();
    }
}

fn translate_event_position(event: WindowEvent, scale_factor: f64) -> WindowEvent {
    match event {
        WindowEvent::CursorMoved {
            device_id,
            position,
        } => WindowEvent::CursorMoved {
            device_id,
            position: translate_position(position, scale_factor),
        },
        WindowEvent::Touch(Touch {
            device_id,
            phase,
            location,
            force,
            id,
        }) => WindowEvent::Touch(Touch {
            device_id,
            phase,
            location: translate_position(location, scale_factor),
            force,
            id,
        }),
        event => event,
    }
}

fn translate_position(position: PhysicalPosition<f64>, scale_factor: f64) -> PhysicalPosition<f64> {
    let mut logical_position: LogicalPosition<f64> = position.to_logical(scale_factor);
    logical_position.x -= 1.0;
    logical_position.y -= 1.0 + HEADER_HEIGHT as f64;

    return logical_position.to_physical(scale_factor);
}

pub struct ChoiceWindow<'a> {
    inner_window: InnerWindow<'a>,
    egui_window: EguiCPUWindow,
    text: Option<String>,
    options: Vec<ChoiceWindowOption>,
}

impl<'a> ChoiceWindow<'a> {
    pub fn new(
        inner_window: InnerWindow<'a>,
        text: Option<String>,
        options: Vec<ChoiceWindowOption>,
    ) -> Result<Self> {
        let egui_window = EguiCPUWindow::new(
            inner_window.window.clone(),
            inner_window.is_gpu(),
            inner_window.transparent,
        )?;

        Ok(Self {
            inner_window,
            egui_window,
            text,
            options,
        })
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        let event = if self.inner_window.decorations {
            &translate_event_position(event.clone(), self.inner_window.window.scale_factor())
        } else {
            event
        };

        self.egui_window.handle_event(event);
    }

    pub fn render(&mut self) -> Result<()> {
        self.inner_window.start_render()?;
        self.inner_window.render_decorations()?;

        let id = self.inner_window.window.id();
        let lua_event_tx = self.inner_window.lua_event_tx.clone();

        self.inner_window.render_with_softbuffer_buffer(|buffer| {
            self.egui_window.redraw(buffer, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        // ui.heading("Repeat after me");
                        ui.add_space(20.0);

                        if let Some(text) = &self.text {
                            ui.label(RichText::new(text).heading());
                        }

                        ui.add_space(ui.available_height() - 100.0);

                        ui.with_layout(
                            egui::Layout::left_to_right(egui::Align::Center)
                                .with_main_wrap(true)
                                .with_main_align(egui::Align::Center)
                                .with_main_justify(true),
                            |ui| {
                                for option in &self.options {
                                    if ui.button(&option.label).clicked() {
                                        let _ = lua_event_tx.send(lua::Event::ChoiceSelect {
                                            id,
                                            option_id: option.id.clone(),
                                        });
                                    }
                                    ui.add_space(5.0);
                                }
                            },
                        )
                    })
                });
            })
        })?;

        self.inner_window.present()?;

        Ok(())
    }

    pub fn set_text(&mut self, text: Option<String>) {
        self.text = text;
        self.inner_window.window.request_redraw();
    }

    pub fn set_options(&mut self, options: Vec<ChoiceWindowOption>) {
        self.options = options;
        self.inner_window.window.request_redraw();
    }
}

pub struct InnerWindow<'a> {
    window: Arc<WinitWindow>,
    surface: Surface<'a>,
    decorations: bool,
    border_rendered: bool,
    header: Option<Header>,
    inner_size: PhysicalSize<u32>,
    outer_size: PhysicalSize<u32>,
    position: LogicalPosition<u32>,
    lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    current_move: Option<Move>,
    wgpu_state: Arc<WgpuState>,
    transparent: bool,
}

struct Move {
    id: u64,
    from: LogicalPosition<u32>,
    to: LogicalPosition<u32>,
    duration: Duration,
    start: Instant,
    easing: Easing,
}

impl<'a> InnerWindow<'a> {
    pub fn new(
        window: WinitWindow,
        wgpu_state: Arc<WgpuState>,
        decorations: bool,
        title: Option<String>,
        closeable: bool,
        gpu: bool,
        transparent: bool,
        position: LogicalPosition<u32>,
        lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    ) -> Result<Self> {
        let window = Arc::new(window);
        let (inner_size, outer_size) = calculate_size(&window, decorations);

        let surface = if gpu && !wgpu_state.error.load(Ordering::Acquire) {
            let surface = wgpu_state.instance.create_surface(window.clone())?;
            let surface_caps = surface.get_capabilities(&wgpu_state.adapter);
            let surface_format = surface_caps
                .formats
                .iter()
                .find(|f| f.is_srgb())
                .unwrap_or(&surface_caps.formats[0]);

            let alpha_mode = if transparent {
                if surface_caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
                    wgpu::CompositeAlphaMode::PreMultiplied
                } else if surface_caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
                    wgpu::CompositeAlphaMode::PostMultiplied
                } else {
                    wgpu::CompositeAlphaMode::Auto
                }
            } else {
                wgpu::CompositeAlphaMode::Opaque
            };

            let surface_config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: *surface_format,
                width: outer_size.width,
                height: outer_size.height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&wgpu_state.device, &surface_config);

            let frame_buffer = vec![0; (outer_size.width * outer_size.height * 4) as usize];

            let texture = wgpu_state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Wgpu Surface Frame Texture"),
                size: wgpu::Extent3d {
                    width: outer_size.width,
                    height: outer_size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let bind_group = wgpu_state.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Wgpu Surface Frame Bind Group"),
                layout: &wgpu_state.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&wgpu_state.sampler),
                    },
                ],
            });

            Surface::Wgpu {
                surface,
                surface_config,
                frame_buffer,
                texture,
                bind_group,
                video_renderer: None,
            }
        } else {
            let (context, surface) = init_softbuffer(window.clone())?;

            Surface::Softbuffer {
                _context: context,
                surface,
            }
        };

        let scale_factor = window.scale_factor();
        let header = decorations.then(|| {
            Header::new(
                window.clone(),
                inner_size.clone(),
                scale_factor,
                title,
                closeable,
            )
        });

        Ok(Self {
            window,
            surface,
            decorations,
            border_rendered: false,
            header,
            inner_size,
            outer_size,
            position,
            lua_event_tx,
            current_move: None,
            wgpu_state,
            transparent,
        })
    }

    pub fn init_video_texture(
        &mut self,
        width: u32,
        height: u32,
        full_range: bool,
        pixel_format: VideoPixelFormat,
    ) -> Result<()> {
        if let Surface::Wgpu {
            video_renderer,
            surface_config,
            ..
        } = &mut self.surface
        {
            *video_renderer = Some(VideoRenderer::new(
                &self.wgpu_state,
                surface_config.format,
                width,
                height,
                full_range,
                pixel_format,
                surface_config.width,
                surface_config.height,
            ));
        }
        Ok(())
    }

    fn start_render(&mut self) -> Result<()> {
        match &mut self.surface {
            Surface::Wgpu { .. } => {
                if self.wgpu_state.error.load(Ordering::Acquire) {
                    println!("wgpu error; switching to softbuffer");
                    let (context, surface) = init_softbuffer(self.window.clone())?;

                    self.surface = Surface::Softbuffer {
                        _context: context,
                        surface,
                    };

                    return self.start_render();
                }
            }
            Surface::Softbuffer { _context, surface } => {
                surface
                    .resize(
                        NonZeroU32::new(self.outer_size.width).context("Window has 0 width")?,
                        NonZeroU32::new(self.outer_size.height).context("Window has 0 height")?,
                    )
                    .map_err(|err| anyhow!("{}", err))?;
            }
        }

        Ok(())
    }

    fn present(&mut self) -> Result<()> {
        let (x, y) = self.inner_offset();

        match &mut self.surface {
            Surface::Wgpu {
                surface,
                surface_config,
                frame_buffer,
                texture,
                bind_group,
                video_renderer,
            } => {
                if self.wgpu_state.error.load(Ordering::Acquire) {
                    bail!("wgpu error; stopping rendering");
                }

                let width = self.inner_size.width;
                let height = self.inner_size.height;

                if let Some(video) = video_renderer.as_ref() {
                    // Upload frame_buffer only to the UI overlay texture; skip the redundant
                    // upload to `texture` which is never used in the video render path.
                    video.update_ui(
                        &self.wgpu_state.queue,
                        frame_buffer,
                        surface_config.width,
                        surface_config.height,
                    );
                } else {
                    upload_texture_data(
                        &self.wgpu_state.queue,
                        texture,
                        frame_buffer,
                        surface_config.width,
                        surface_config.height,
                        surface_config.width * 4,
                        4,
                    );
                }

                let output = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(surface_texture) => surface_texture,
                    wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => surface_texture,
                    wgpu::CurrentSurfaceTexture::Timeout => return Ok(()),
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        surface.configure(&self.wgpu_state.device, surface_config);
                        return Ok(());
                    }
                    wgpu::CurrentSurfaceTexture::Lost => {
                        surface.configure(&self.wgpu_state.device, surface_config);
                        return Ok(());
                    }
                    _ => return Ok(()),
                };

                let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

                let mut encoder = self.wgpu_state.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Wgpu Surface Render Encoder"),
                });

                {
                    if let Some(video) = video_renderer {
                        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Video Render Pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(if self.transparent {
                                        wgpu::Color::TRANSPARENT
                                    } else {
                                        wgpu::Color::BLACK
                                    }),
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });

                        // Video: YUV/NV12→RGB via dedicated pipeline, scaled to inner viewport
                        let (vid_pipeline, vid_bind_group) = video.video_pipeline_and_bind_group();
                        rpass.set_pipeline(vid_pipeline);
                        rpass.set_bind_group(0, vid_bind_group, &[]);
                        rpass.set_viewport(
                            x as f32, y as f32,
                            width as f32, height as f32,
                            0.0, 1.0,
                        );
                        rpass.draw(0..4, 0..1);

                        // UI overlay: RGBA pipeline, full surface
                        rpass.set_pipeline(&video.ui_pipeline);
                        rpass.set_bind_group(0, &video.ui_bind_group, &[]);
                        rpass.set_viewport(
                            0.0, 0.0,
                            surface_config.width as f32, surface_config.height as f32,
                            0.0, 1.0,
                        );
                        rpass.draw(0..4, 0..1);
                    } else {
                        // Render the CPU frame buffer texture
                        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Frame Render Pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(if self.transparent {
                                        wgpu::Color::TRANSPARENT
                                    } else {
                                        wgpu::Color::BLACK
                                    }),
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });

                        let pipeline = self.wgpu_state.get_pipeline(surface_config.format);
                        rpass.set_pipeline(&pipeline);
                        rpass.set_bind_group(0, &*bind_group, &[]);
                        rpass.set_viewport(
                            0.0,
                            0.0,
                            surface_config.width as f32,
                            surface_config.height as f32,
                            0.0,
                            1.0,
                        );
                        rpass.draw(0..4, 0..1);
                    }
                }

                self.wgpu_state.queue.submit(Some(encoder.finish()));
                output.present();
            }
            Surface::Softbuffer { _context, surface } => {
                surface
                    .buffer_mut()
                    .map_err(|err| anyhow!("{err}"))?
                    .present()
                    .map_err(|err| anyhow!("{err}"))?;
            }
        }

        Ok(())
    }

    fn render_border(&mut self) -> Result<bool> {
        if !self.border_rendered {
            self.surface.buffer()?.draw_border();

            self.border_rendered = true;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn render_header(&mut self) -> Result<bool> {
        if let Some(header) = &mut self.header {
            let scale_factor = self.window.scale_factor();
            let border_offset = PhysicalUnit::from_logical::<_, u32>(1, scale_factor).0;

            if let Some(pixmap) = header.draw() {
                self.surface
                    .buffer()?
                    .copy_from_pixmap(pixmap, border_offset, border_offset);

                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }

    fn inner_offset(&self) -> (u32, u32) {
        if self.decorations {
            let scale_factor = self.window.scale_factor();
            let border_offset = PhysicalUnit::from_logical::<_, u32>(1, scale_factor).0;
            let header_height: u32 =
                PhysicalUnit::from_logical::<_, u32>(HEADER_HEIGHT, scale_factor).0;

            (border_offset, border_offset + header_height)
        } else {
            (0, 0)
        }
    }

    fn render_pixmap(&mut self, pixmap: &Pixmap) -> Result<()> {
        let (x, y) = self.inner_offset();

        self.surface.buffer()?.copy_from_pixmap(pixmap, x, y);

        Ok(())
    }

    fn render_frame(&mut self, frame: &VideoFrame) -> Result<()> {
        if let Surface::Wgpu {
            video_renderer: Some(video),
            ..
        } = &mut self.surface
        {
            let wgpu_state = self.wgpu_state.clone();
            video.update_video(&wgpu_state, frame);
            return Ok(());
        }

        let (x, y) = self.inner_offset();
        self.surface.buffer()?.copy_from_frame(frame, x, y);

        Ok(())
    }

    fn render_with_softbuffer_buffer(
        &mut self,
        f: impl FnOnce(&mut BufferMutRef) -> Result<()>,
    ) -> Result<()> {
        if self.decorations {
            let mut buffer = vec![0; (self.inner_size.width * self.inner_size.height) as usize];

            let buffer_ref = &mut BufferMutRef::new(
                bytemuck::cast_slice_mut(&mut buffer),
                self.inner_size.width as usize,
                self.inner_size.height as usize,
            );

            f(buffer_ref)?;

            let (x, y) = self.inner_offset();
            self.surface
                .buffer()?
                .copy_from_u32_buf(&mut buffer, self.inner_size.width, x, y);
        } else {
            match self.surface.buffer()? {
                Buffer::Pixmap(mut pixmap) => {
                    pixmap.data_mut().fill(0);

                    let buffer_ref = &mut BufferMutRef::new(
                        bytemuck::cast_slice_mut(pixmap.data_mut()),
                        self.inner_size.width as usize,
                        self.inner_size.height as usize,
                    );

                    f(buffer_ref)?;
                }
                Buffer::Softbuffer(mut buffer) => {
                    buffer.fill(0);

                    let buffer_ref = &mut BufferMutRef::new(
                        bytemuck::cast_slice_mut(&mut buffer),
                        self.inner_size.width as usize,
                        self.inner_size.height as usize,
                    );

                    f(buffer_ref)?;
                }
            }
        }

        Ok(())
    }

    fn render_decorations(&mut self) -> Result<bool> {
        if self.decorations {
            let border = self.render_border()?;
            let header = self.render_header()?;
            Ok(border || header)
        } else {
            Ok(false)
        }
    }

    pub fn is_gpu(&self) -> bool {
        self.surface.is_gpu()
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn start_move(&mut self, id: u64, opts: MoveOpts) -> Result<(), LewdwareError> {
        let scale_factor = self.window.scale_factor();

        let size = self.window.inner_size().to_logical(scale_factor);

        let monitor_size = self
            .window
            .current_monitor()
            .ok_or(LewdwareError::MonitorError(
                MonitorError::WindowMonitorNotFound,
            ))?
            .size()
            .to_logical::<u32>(scale_factor);

        let x = match opts.x {
            Some(Coord::Pixel(x)) => Some(opts.anchor.resolve(x, size.width)),
            Some(Coord::Percent { percent }) => Some(opts.anchor.resolve(
                ((percent * monitor_size.width as f64) / 100.0).round() as u32,
                size.width,
            )),
            None => None,
        };

        let y = match opts.y {
            Some(Coord::Pixel(y)) => Some(opts.anchor.resolve(y, size.height)),
            Some(Coord::Percent { percent }) => Some(opts.anchor.resolve(
                ((percent * monitor_size.height as f64) / 100.0).round() as u32,
                size.height,
            )),
            None => None,
        };

        let new_position = if opts.relative {
            LogicalPosition::new(
                self.position.x + x.unwrap_or(0),
                self.position.y + y.unwrap_or(0),
            )
        } else {
            LogicalPosition::new(x.unwrap_or(self.position.x), y.unwrap_or(self.position.y))
        };

        println!("{:?}", self.position);

        let move_obj = Move {
            id: id,
            from: self.position.clone(),
            to: new_position,
            duration: Duration::from_millis(opts.duration),
            start: Instant::now(),
            easing: opts.easing,
        };

        self.current_move = Some(move_obj);

        Ok(())
    }

    pub fn is_moving(&self) -> bool {
        self.current_move.is_some()
    }

    pub fn update_position(&mut self) {
        if let Some(current_move) = &self.current_move {
            let percent = current_move
                .start
                .elapsed()
                .div_duration_f64(current_move.duration)
                .min(1.0);

            let new_position = LogicalPosition::new(
                current_move.from.x + ((current_move.to.x - current_move.from.x) as f64 * percent).round() as u32,
                current_move.from.y + ((current_move.to.y - current_move.from.y) as f64 * percent).round() as u32,
            );

            if new_position != self.position {
                let monitor_position = self
                    .window
                    .current_monitor()
                    .map(|monitor| monitor.position().to_logical(self.window.scale_factor()))
                    .unwrap_or(LogicalPosition::new(0, 0));

                self.window.set_outer_position(LogicalPosition::new(
                    monitor_position.x + new_position.x,
                    monitor_position.y + new_position.y,
                ));

                self.position = new_position;
            }

            if percent >= 1.0 {
                if let Err(err) = self.lua_event_tx.send(lua::Event::MoveFinish {
                    id: self.window.id(),
                    move_id: current_move.id,
                }) {
                    eprintln!("{err}");
                }

                self.current_move = None;
            }
        }
    }

    pub fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if let Some(header) = &mut self.header {
            header.handle_cursor_moved(position);
        }
    }

    pub fn handle_cursor_left(&mut self) {
        if let Some(header) = &mut self.header {
            header.handle_cursor_left();
        }
    }

    pub fn handle_mouse_down(&mut self) {
        if let Some(header) = &mut self.header {
            header.handle_mouse_down();
        }
    }

    pub fn handle_mouse_up(&mut self) -> bool {
        if let Some(header) = &mut self.header {
            header.handle_mouse_up()
        } else {
            false
        }
    }

    pub fn set_visible(&self, visible: bool) {
        self.window.set_visible(visible);
    }

    pub fn set_title(&mut self, text: Option<String>) {
        if let Some(header) = &mut self.header {
            header.set_title(text);
        }
    }
}

impl Drop for InnerWindow<'_> {
    fn drop(&mut self) {
        if let Err(_) = self.lua_event_tx.send(lua::Event::WindowClosed {
            id: self.window.id(),
        }) {
            eprintln!("Event receiver closed");
        }
    }
}

fn upload_texture_data(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    data: &[u8],
    width: u32,
    height: u32,
    source_stride: u32,
    bytes_per_pixel: u32,
) {
    let unpadded_bytes_per_row = width * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row_padding = (align - unpadded_bytes_per_row % align) % align;
    let padded_bytes_per_row = unpadded_bytes_per_row + padded_bytes_per_row_padding;

    if source_stride == padded_bytes_per_row {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    } else {
        let mut padded_data = Vec::with_capacity((padded_bytes_per_row * height) as usize);
        for i in 0..height {
            let src_start = (i * source_stride) as usize;
            let src_end = src_start + unpadded_bytes_per_row as usize;
            padded_data.extend_from_slice(&data[src_start..src_end]);
            padded_data.extend(std::iter::repeat(0).take(padded_bytes_per_row_padding as usize));
        }

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &padded_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
}
