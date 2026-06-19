use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use shared::once;
use tokio::sync::mpsc;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, PhysicalUnit};
use winit::window::Window;

use crate::error::{LewdwareError, MonitorError};
use crate::lua::{self, Coord, Easing, FadeOpts, MoveOpts};
use crate::wgpu::WgpuState;
use crate::window::header::HEADER_HEIGHT;
use crate::window::surface::Buffer;
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
    wgpu_state: Option<Arc<WgpuState>>,
    transparent: bool,
    // Whether the surface's CompositeAlphaMode is PreMultiplied (vs. PostMultiplied/Opaque).
    // Tells the fragment shaders whether they need to pre-scale rgb by alpha themselves.
    premultiplied_alpha: bool,
    force_opaque: bool,
    current_fade: Option<Fade>,
    pub opacity: f32,
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
        wgpu_state: Option<Arc<WgpuState>>,
        decorations: bool,
        title: Option<String>,
        closeable: bool,
        gpu: bool,
        transparent: bool,
        force_opaque: bool,
        opacity: Option<f32>,
        position: LogicalPosition<u32>,
        lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    ) -> Result<Self> {
        let window = Arc::new(window);
        let (inner_size, outer_size) = calculate_size(&window, decorations);

        let mut premultiplied_alpha = false;

        let surface = if let (true, Some(wgpu)) = (gpu, &wgpu_state) {
            if !wgpu.error.load(Ordering::Acquire) {
                let surface = wgpu.instance.create_surface(window.clone())?;
                let surface_caps = surface.get_capabilities(&wgpu.adapter);
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
                        premultiplied_alpha = true;
                        wgpu::CompositeAlphaMode::PreMultiplied
                    } else if surface_caps
                        .alpha_modes
                        .contains(&wgpu::CompositeAlphaMode::PostMultiplied)
                    {
                        wgpu::CompositeAlphaMode::PostMultiplied
                    } else {
                        // Neither mode is available, so the compositor has no way to blend this
                        // window's alpha against the desktop at all (common on plain Win32/DX12
                        // swapchains, non-compositing X11, and software/llvmpipe GL). `Auto` would
                        // only ever resolve to `Opaque` or `Inherit` here anyway, both of which
                        // amount to the same thing in practice, so make that explicit.
                        once!(tracing::error!(
                            "This platform/adapter doesn't support transparent windows (no PreMultiplied/PostMultiplied composite alpha mode available); transparent popups will render opaque"
                        ));

                        wgpu::CompositeAlphaMode::Opaque
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
                surface.configure(&wgpu.device, &surface_config);

                Surface::Wgpu {
                    surface,
                    surface_config,
                }
            } else {
                let (context, surface) = init_softbuffer(window.clone())?;
                Surface::Softbuffer {
                    _context: context,
                    surface,
                }
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
            premultiplied_alpha,
            force_opaque,
            current_fade: None,
            opacity: opacity.unwrap_or(1.0),
        })
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity;
    }

    pub fn wgpu_state(&self) -> &Arc<WgpuState> {
        self.wgpu_state.as_ref().unwrap()
    }

    pub fn surface_format(&self) -> Option<wgpu::TextureFormat> {
        match &self.surface {
            Surface::Wgpu { surface_config, .. } => Some(surface_config.format),
            _ => None,
        }
    }

    pub fn inner_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.inner_size
    }

    pub fn outer_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.outer_size
    }

    pub fn start_render(&mut self) -> Result<()> {
        match &mut self.surface {
            Surface::Wgpu { .. } => {
                if self
                    .wgpu_state
                    .as_ref()
                    .unwrap()
                    .error
                    .load(Ordering::Acquire)
                {
                    tracing::info!("wgpu error; switching to softbuffer");
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

    pub fn with_header_pixmap<F: FnOnce(&tiny_skia::Pixmap)>(&mut self, f: F) {
        if let Some(pixmap) = self.header.as_mut().and_then(|h| h.draw()) {
            f(pixmap);
        }
    }

    pub fn draw_wgpu(
        &mut self,
        draw_fn: impl FnOnce(&mut wgpu::RenderPass<'static>, u32, u32),
    ) -> Result<()> {
        let (x, y) = self.inner_offset();
        match &mut self.surface {
            Surface::Wgpu {
                surface,
                surface_config,
            } => {
                let wgpu = self.wgpu_state.as_ref().unwrap();
                if wgpu.error.load(Ordering::Acquire) {
                    bail!("wgpu error; stopping rendering");
                }

                let output = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(surface_texture) => surface_texture,
                    wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => surface_texture,
                    wgpu::CurrentSurfaceTexture::Timeout => return Ok(()),
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        surface.configure(&wgpu.device, surface_config);
                        return Ok(());
                    }
                    wgpu::CurrentSurfaceTexture::Lost => {
                        *surface = wgpu.instance.create_surface(self.window.clone())?;
                        surface.configure(&wgpu.device, surface_config);
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

                let mut encoder =
                    wgpu.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Wgpu Surface Render Encoder"),
                        });

                let clear = if self.transparent {
                    wgpu::Color::TRANSPARENT
                } else {
                    wgpu::Color::BLACK
                };

                {
                    let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Render Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(clear),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    let mut rpass = rpass.forget_lifetime();

                    rpass.set_viewport(
                        0.0,
                        0.0,
                        surface_config.width as f32,
                        surface_config.height as f32,
                        0.0,
                        1.0,
                    );

                    draw_fn(&mut rpass, x, y);
                }

                wgpu.queue.submit(Some(encoder.finish()));
                output.present();
            }
            _ => bail!("Called draw_wgpu on a non-GPU surface"),
        }

        Ok(())
    }

    pub fn draw_softbuffer(&mut self, draw_fn: impl FnOnce(&mut Buffer)) -> Result<()> {
        let softbuffer_surface = match &mut self.surface {
            Surface::Softbuffer { surface, .. } => surface,
            _ => bail!("Called draw_softbuffer on a non-CPU surface"),
        };

        let buffer_data = softbuffer_surface
            .buffer_mut()
            .map_err(|err| anyhow!("{err}"))?;
        let mut buffer = Buffer::Softbuffer(buffer_data);

        draw_fn(&mut buffer);

        // draw decorations — always written every frame because softbuffer buffers are not
        // guaranteed to retain content across frames (e.g. macOS CALayer backing store).
        if self.decorations {
            buffer.draw_border();
            if let Some(header) = &mut self.header {
                let scale_factor = self.window.scale_factor();
                let border_offset = PhysicalUnit::from_logical::<_, u32>(1, scale_factor).0;
                let pixmap = header.get_pixmap();
                buffer.copy_from_pixmap(pixmap, border_offset, border_offset);
            }
        }

        match buffer {
            Buffer::Softbuffer(b) => b.present().map_err(|err| anyhow!("{err}"))?,
            _ => unreachable!(),
        }

        Ok(())
    }

    fn render_border(&mut self, buffer: &mut Buffer) -> Result<bool> {
        if !self.border_rendered {
            buffer.draw_border();

            self.border_rendered = true;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn render_header(&mut self, buffer: &mut Buffer) -> Result<bool> {
        if let Some(header) = &mut self.header {
            let scale_factor = self.window.scale_factor();
            let border_offset = PhysicalUnit::from_logical::<_, u32>(1, scale_factor).0;

            if let Some(pixmap) = header.draw() {
                buffer.copy_from_pixmap(pixmap, border_offset, border_offset);

                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }

    pub fn inner_offset(&self) -> (u32, u32) {
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

    pub fn render_decorations(&mut self, buffer: &mut Buffer) -> Result<bool> {
        if self.decorations {
            let border = self.render_border(buffer)?;
            let header = self.render_header(buffer)?;
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

        tracing::info!("{:?}", self.position);

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
                    + ((current_move.to.x as f64 - current_move.from.x as f64) * eased_percent)
                        .round() as u32,
                current_move.from.y
                    + ((current_move.to.y as f64 - current_move.from.y as f64) * eased_percent)
                        .round() as u32,
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
                    tracing::error!("{err}");
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
        let (new_opacity, _percent, is_finished, fade_id) =
            if let Some(current_fade) = &self.current_fade {
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
                tracing::error!("{err}");
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

    /// Whether the surface was configured with `CompositeAlphaMode::PreMultiplied`. If so, the
    /// fragment shaders need to pre-scale their rgb output by alpha; otherwise (PostMultiplied,
    /// or Opaque where alpha is ignored by the compositor entirely) they should emit it straight.
    pub fn premultiplied_alpha(&self) -> bool {
        self.premultiplied_alpha
    }

    pub fn force_opaque(&self) -> bool {
        self.force_opaque
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
            tracing::error!("Event receiver closed");
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
