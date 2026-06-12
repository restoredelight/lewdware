use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use egui_software_backend::BufferMutRef;
use tiny_skia::Pixmap;
use tokio::sync::mpsc;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, PhysicalUnit};
use winit::window::Window;

use crate::wgpu::WgpuState;
use crate::error::{LewdwareError, MonitorError};
use crate::lua::{self, Coord, Easing, MoveOpts, FadeOpts};
use crate::video::{VideoFrame, VideoPixelFormat};
use crate::window::header::HEADER_HEIGHT;
use crate::window::surface::Buffer;
use crate::window::video_renderer::{VideoRenderer, upload_texture_data};
use crate::window::{header::Header, surface::Surface};

pub struct InnerWindow<'a> {
    window: Arc<winit::window::Window>,
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
    current_fade: Option<Fade>,
    opacity: f32,
}

struct Move {
    id: u64,
    from: LogicalPosition<u32>,
    to: LogicalPosition<u32>,
    duration: Duration,
    start: Instant,
    easing: Easing,
}

struct Fade {
    id: u64,
    from: f32,
    to: f32,
    duration: Duration,
    start: Instant,
    easing: Easing,
}

impl<'a> InnerWindow<'a> {
    pub fn new(
        window: Window,
        wgpu_state: Arc<WgpuState>,
        decorations: bool,
        title: Option<String>,
        closeable: bool,
        gpu: bool,
        transparent: bool,
        opacity: Option<f32>,
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
                if surface_caps
                    .alpha_modes
                    .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
                {
                    wgpu::CompositeAlphaMode::PreMultiplied
                } else if surface_caps
                    .alpha_modes
                    .contains(&wgpu::CompositeAlphaMode::PostMultiplied)
                {
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

            let bind_group = wgpu_state
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
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

            use wgpu::util::DeviceExt;
            let opacity_val = opacity.unwrap_or(1.0);
            let opacity_buffer = wgpu_state.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Opacity Buffer"),
                contents: bytemuck::cast_slice(&[opacity_val]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let window_bind_group = wgpu_state.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Window Bind Group"),
                layout: &wgpu_state.window_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: opacity_buffer.as_entire_binding(),
                    },
                ],
            });

            Surface::Wgpu {
                surface,
                surface_config,
                frame_buffer,
                texture,
                bind_group,
                opacity_buffer,
                window_bind_group,
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
            current_fade: None,
            opacity: opacity.unwrap_or(1.0),
        })
    }

    pub fn init_video_texture(
        &mut self,
        width: u32,
        height: u32,
        full_range: bool,
        pixel_format: VideoPixelFormat,
        packed_alpha: bool,
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
                packed_alpha,
                surface_config.width,
                surface_config.height,
            ));
        }
        Ok(())
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity;
        if let Surface::Wgpu { opacity_buffer, .. } = &self.surface {
            self.wgpu_state.queue.write_buffer(opacity_buffer, 0, bytemuck::cast_slice(&[opacity]));
            self.request_redraw();
        }
    }

    pub fn start_render(&mut self) -> Result<()> {
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

    pub fn present(&mut self) -> Result<()> {
        let (x, y) = self.inner_offset();

        match &mut self.surface {
            Surface::Wgpu {
                surface,
                surface_config,
                frame_buffer,
                texture,
                bind_group,
                window_bind_group,
                video_renderer,
                ..
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
                        *surface = self
                            .wgpu_state
                            .instance
                            .create_surface(self.window.clone())?;

                        surface.configure(&self.wgpu_state.device, surface_config);

                        return Ok(());
                    }
                    wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
                    wgpu::CurrentSurfaceTexture::Validation => {
                        bail!("Validation error")
                    }
                };

                let view = output
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                let mut encoder = self.wgpu_state.device.create_command_encoder(
                    &wgpu::CommandEncoderDescriptor {
                        label: Some("Wgpu Surface Render Encoder"),
                    },
                );

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

                    // Video: YUV/NV12->RGB via dedicated pipeline, scaled to inner viewport
                    let (vid_pipeline, vid_bind_group) = video.video_pipeline_and_bind_group();
                    rpass.set_pipeline(vid_pipeline);
                    rpass.set_bind_group(0, vid_bind_group, &[]);
                    rpass.set_bind_group(1, &*window_bind_group, &[]);
                    rpass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
                    rpass.draw(0..4, 0..1);

                    // UI overlay: RGBA pipeline, full surface
                    rpass.set_pipeline(video.ui_pipeline());
                    rpass.set_bind_group(0, video.ui_bind_group(), &[]);
                    rpass.set_bind_group(1, &*window_bind_group, &[]);
                    rpass.set_viewport(
                        0.0,
                        0.0,
                        surface_config.width as f32,
                        surface_config.height as f32,
                        0.0,
                        1.0,
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
                    rpass.set_bind_group(1, &*window_bind_group, &[]);
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

    pub fn render_pixmap(&mut self, pixmap: &Pixmap) -> Result<()> {
        let (x, y) = self.inner_offset();

        self.surface.buffer()?.copy_from_pixmap(pixmap, x, y);

        Ok(())
    }

    pub fn render_frame(&mut self, frame: &VideoFrame) -> Result<()> {
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

    pub fn render_with_softbuffer_buffer(
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

    pub fn render_decorations(&mut self) -> Result<bool> {
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

            let eased_percent = current_move.easing.apply(percent);

            let new_position = LogicalPosition::new(
                current_move.from.x
                    + ((current_move.to.x as f64 - current_move.from.x as f64) * eased_percent).round() as u32,
                current_move.from.y
                    + ((current_move.to.y as f64 - current_move.from.y as f64) * eased_percent).round() as u32,
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

    pub fn start_fade(&mut self, id: u64, opts: FadeOpts) -> Result<(), LewdwareError> {
        let to = if opts.relative {
            self.opacity + opts.opacity
        } else {
            opts.opacity
        };

        let fade_obj = Fade {
            id,
            from: self.opacity,
            to,
            duration: Duration::from_millis(opts.duration),
            start: Instant::now(),
            easing: opts.easing,
        };

        self.current_fade = Some(fade_obj);

        Ok(())
    }

    pub fn is_fading(&self) -> bool {
        self.current_fade.is_some()
    }

    pub fn update_fade(&mut self) {
        let (new_opacity, percent, is_finished, fade_id) = if let Some(current_fade) = &self.current_fade {
            let percent = current_fade
                .start
                .elapsed()
                .div_duration_f64(current_fade.duration)
                .min(1.0);

            let eased_percent = current_fade.easing.apply(percent);

            let new_opacity = current_fade.from
                + ((current_fade.to - current_fade.from) as f64 * eased_percent) as f32;

            (new_opacity, percent, percent >= 1.0, current_fade.id)
        } else {
            return;
        };

        if new_opacity != self.opacity {
            self.set_opacity(new_opacity);
        }

        if is_finished {
            if let Err(err) = self.lua_event_tx.send(lua::Event::FadeFinish {
                id: self.window.id(),
                fade_id,
            }) {
                eprintln!("{err}");
            }

            self.current_fade = None;
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

    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    pub fn transparent(&self) -> bool {
        self.transparent
    }

    pub fn decorations(&self) -> bool {
        self.decorations
    }

    pub fn lua_event_tx(&self) -> &mpsc::UnboundedSender<lua::Event> {
        &self.lua_event_tx
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

fn calculate_size(
    window: &Arc<Window>,
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

fn init_softbuffer(
    window: Arc<Window>,
) -> Result<(
    softbuffer::Context<Arc<Window>>,
    softbuffer::Surface<Arc<Window>, Arc<Window>>,
)> {
    let context = softbuffer::Context::new(window.clone()).map_err(|err| anyhow!("{}", err))?;
    let surface =
        softbuffer::Surface::new(&context, window.clone()).map_err(|err| anyhow!("{}", err))?;

    Ok((context, surface))
}
