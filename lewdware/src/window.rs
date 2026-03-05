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
use pixels::{Pixels, PixelsBuilder, SurfaceTexture};
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
    video::{NextFrame, VideoDecoder, VideoFrame},
};

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

struct VideoRenderer {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    width: u32,
    height: u32,
    ui_texture: wgpu::Texture,
    ui_bind_group: wgpu::BindGroup,
}

impl VideoRenderer {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        ui_width: u32,
        ui_height: u32,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Video Texture"),
            size: wgpu::Extent3d {
                width,
                height,
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

        let ui_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("UI Texture"),
            size: wgpu::Extent3d {
                width: ui_width,
                height: ui_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb, // Pixels usually uses this
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let ui_texture_view = ui_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Video Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Video Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let ui_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("UI Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&ui_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Video Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Video Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shader.wgsl"
            ))),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Video Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            texture,
            bind_group,
            pipeline,
            width,
            height,
            ui_texture,
            ui_bind_group,
        }
    }

    fn update_ui(&self, queue: &wgpu::Queue, data: &[u8], width: u32, height: u32) {
        upload_texture_data(queue, &self.ui_texture, data, width, height, width * 4);
    }
}

enum Surface<'a> {
    Pixels {
        pixels: Pixels<'a>,
        error: Arc<AtomicBool>,
        video_renderer: Option<VideoRenderer>,
    },
    Softbuffer {
        _context: softbuffer::Context<Arc<WinitWindow>>,
        surface: softbuffer::Surface<Arc<WinitWindow>, Arc<WinitWindow>>,
    },
}

impl<'a> Surface<'a> {
    fn buffer(&mut self) -> Result<Buffer<'_>> {
        match self {
            Surface::Pixels { pixels, .. } => {
                let width = pixels.texture().width();
                let height = pixels.texture().height();

                let dest = PixmapMut::from_bytes(pixels.frame_mut(), width, height)
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

        let buffer = match self {
            Buffer::Pixmap(_) => panic!("Buffer must be a softbuffer buffer"),
            Buffer::Softbuffer(buffer) => buffer,
        };

        for (i, row) in src.chunks_exact(width as usize).enumerate() {
            let index = offset + (dst_width * i as u32 + x) as usize;

            buffer[index..(index + row.len())].copy_from_slice(row);
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
        // Initialize the GPU texture for the video
        inner_window
            .init_video_texture(video_player.native_width(), video_player.native_height())?;

        // If we are already on softbuffer (e.g. wgpu initialization failed), tell the decoder to resize
        if let Surface::Softbuffer { .. } = &inner_window.surface {
            video_player
                .set_output_size(inner_window.inner_size.width, inner_window.inner_size.width);
        }

        video_player.play();

        inner_window.window.request_redraw();

        Ok(Self {
            inner_window,
            video_player,
            last_frame_time: Instant::now(),
            duration: None,
            loop_video,
            paused: false,
        })
    }

    pub fn update(&mut self) -> Result<bool> {
        let mut render = false;

        self.inner_window.start_render()?;

        render = render || self.inner_window.render_decorations()?;

        match self.video_player.next_frame() {
            NextFrame::Ready(frame) => {
                // println!("Rendering frame");
                self.inner_window.render_frame(&frame)?;
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
        let egui_window = EguiCPUWindow::new(inner_window.window.clone())?;

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
        let egui_window = EguiCPUWindow::new(inner_window.window.clone())?;

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
    // Store the wgpu context to allow creating resources later
    wgpu_ctx: Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)>,
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
        wgpu_state: &WgpuState,
        decorations: bool,
        title: Option<String>,
        closeable: bool,
        gpu: bool,
        position: LogicalPosition<u32>,
        lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    ) -> Result<Self> {
        let window = Arc::new(window);
        let (inner_size, outer_size) = calculate_size(&window, decorations);

        let (surface, wgpu_ctx) = if gpu && !wgpu_state.error.load(Ordering::Acquire) {
            let surface_texture =
                SurfaceTexture::new(outer_size.width, outer_size.height, window.clone());

            (
                Surface::Pixels {
                    pixels: PixelsBuilder::new(
                        outer_size.width,
                        outer_size.height,
                        surface_texture,
                    )
                    .blend_state(wgpu::BlendState::ALPHA_BLENDING) // Enable blending for UI over Video
                    .build_with_instance(
                        &wgpu_state.instance,
                        &wgpu_state.adapter,
                        &wgpu_state.device,
                        &wgpu_state.queue,
                    )?,
                    error: wgpu_state.error.clone(),
                    video_renderer: None,
                },
                Some((&wgpu_state.device, &wgpu_state.queue)),
            )
        } else {
            let (context, surface) = init_softbuffer(window.clone())?;

            (
                Surface::Softbuffer {
                    _context: context,
                    surface,
                },
                None,
            )
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
            wgpu_ctx: wgpu_ctx.map(|(device, queue)| (device.clone(), queue.clone())),
        })
    }

    pub fn init_video_texture(&mut self, width: u32, height: u32) -> Result<()> {
        if let Surface::Pixels {
            video_renderer,
            pixels,
            ..
        } = &mut self.surface
        {
            if let Some((device, _queue)) = &self.wgpu_ctx {
                *video_renderer = Some(VideoRenderer::new(
                    device,
                    pixels.render_texture_format(),
                    width,
                    height,
                    pixels.texture().width(),
                    pixels.texture().height(),
                ));
            }
        }
        Ok(())
    }

    fn start_render(&mut self) -> Result<()> {
        match &mut self.surface {
            Surface::Pixels {
                pixels: _, error, ..
            } => {
                if error.load(Ordering::Acquire) {
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
            Surface::Pixels {
                pixels,
                error,
                video_renderer,
            } => {
                if error.load(Ordering::Acquire) {
                    bail!("wgpu error; stopping rendering");
                }

                let width = self.inner_size.width;
                let height = self.inner_size.height;

                if let Some(video) = video_renderer {
                    if let Some((_, queue)) = &self.wgpu_ctx {
                        video.update_ui(
                            queue,
                            pixels.frame(),
                            pixels.texture().width(),
                            pixels.texture().height(),
                        );
                    }
                }

                pixels.render_with(|encoder, render_target, _context| {
                    if let Some(video) = video_renderer {
                        // Render the video first
                        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Video Render Pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: render_target,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load, // Or Clear if this is the very first thing
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                        });

                        rpass.set_pipeline(&video.pipeline);
                        rpass.set_bind_group(0, &video.bind_group, &[]);
                        // Set viewport to the inner window area
                        rpass.set_viewport(
                            x as f32,
                            y as f32,
                            width as f32,
                            height as f32,
                            0.0,
                            1.0,
                        );
                        rpass.draw(0..4, 0..1);

                        // Draw UI on top
                        rpass.set_bind_group(0, &video.ui_bind_group, &[]);
                        // Reset viewport to full screen
                        rpass.set_viewport(
                            0.0,
                            0.0,
                            render_target.texture().width() as f32,
                            render_target.texture().height() as f32,
                            0.0,
                            1.0,
                        );
                        rpass.draw(0..4, 0..1);
                    }
                    Ok(())
                })?;
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
        // If we have a hardware video renderer, update its texture
        if let Surface::Pixels {
            video_renderer: Some(video),
            ..
        } = &self.surface
        {
            if let Some((_, queue)) = &self.wgpu_ctx {
                let data = frame.frame.data(0);
                upload_texture_data(
                    queue,
                    &video.texture,
                    data,
                    video.width,
                    video.height,
                    frame.frame.stride(0) as u32,
                );
            }

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
            let mut buffer = match self.surface.buffer()? {
                Buffer::Pixmap(_) => panic!("Buffer must be a softbuffer buffer"),
                Buffer::Softbuffer(buffer) => buffer,
            };

            buffer.fill(0);

            let buffer_ref = &mut BufferMutRef::new(
                bytemuck::cast_slice_mut(&mut buffer),
                self.inner_size.width as usize,
                self.inner_size.height as usize,
            );

            f(buffer_ref)?;
        }

        Ok(())
    }

    fn render_decorations(&mut self) -> Result<bool> {
        if self.decorations {
            Ok(self.render_border()? || self.render_header()?)
        } else {
            Ok(false)
        }
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
) {
    let bytes_per_pixel = 4;
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
