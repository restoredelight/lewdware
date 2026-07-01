use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use shared::once;
use tokio::sync::mpsc;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, PhysicalUnit};
use winit::window::Window;

use crate::error::LewdwareError;
use crate::lua::{self, Coord, Easing, FadeOpts, MoveOpts};
use crate::wgpu::WgpuState;
use crate::window::header::HEADER_HEIGHT;
use crate::window::opts::WindowOpts;
use crate::window::surface::Buffer;
use crate::window::{header::Header, surface::Surface};

pub struct InnerWindow {
    window: Arc<winit::window::Window>,
    surface: Surface,
    decorations: bool,
    border_rendered: bool,
    header: Option<Header>,
    inner_size: PhysicalSize<u32>,
    outer_size: PhysicalSize<u32>,
    monitor_position: LogicalPosition<i32>,
    monitor_size: LogicalSize<u32>,
    position: LogicalPosition<i32>,
    lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    current_move: Option<Move>,
    last_move_update: Instant,
    wgpu_state: Option<Arc<WgpuState>>,
    transparent: bool,
    // Whether the surface's CompositeAlphaMode is PreMultiplied (vs. PostMultiplied/Opaque).
    // Tells the fragment shaders whether they need to pre-scale rgb by alpha themselves.
    premultiplied_alpha: bool,
    force_opaque: bool,
    current_fade: Option<Fade>,
    last_fade_update: Instant,
    pub opacity: f32,
    background_color: Option<lua::Color>,
}

struct Move {
    id: u64,
    from: LogicalPosition<i32>,
    to: LogicalPosition<i32>,
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

impl InnerWindow {
    pub fn new(
        window: Arc<Window>,
        opts: &WindowOpts,
        wgpu_state: Option<Arc<WgpuState>>,
        lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    ) -> Result<Self> {
        let decorations = opts.decorations;
        let gpu = opts.gpu;
        let transparent = opts.transparent;
        let force_opaque = opts.force_opaque;
        // Use opts directly rather than window.inner_size(): request_inner_size() is
        // async on X11, so a recycled pool window still reports its previous size here.
        let scale_factor = window.scale_factor();
        let outer_size: PhysicalSize<u32> =
            LogicalSize::new(opts.outer_width, opts.outer_height).to_physical(scale_factor);
        let inner_size: PhysicalSize<u32> =
            LogicalSize::new(opts.width, opts.height).to_physical(scale_factor);

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
                        once!(tracing::error!(
                            "This platform/adapter doesn't support transparent windows \
                             (no PreMultiplied/PostMultiplied composite alpha mode \
                             available); transparent popups will render opaque"
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
                    present_mode: wgpu::PresentMode::AutoNoVsync,
                    alpha_mode,
                    view_formats: vec![],
                    desired_maximum_frame_latency: 1,
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

        let header = decorations.then(|| {
            Header::new(
                window.clone(),
                inner_size.clone(),
                scale_factor,
                opts.title.clone(),
                opts.closeable,
            )
        });

        let monitor_position = LogicalPosition::new(
            opts.position.x - opts.x,
            opts.position.y - opts.y,
        );
        let monitor_size = LogicalSize::new(opts.monitor.width, opts.monitor.height);

        Ok(Self {
            window,
            surface,
            decorations,
            border_rendered: false,
            header,
            inner_size,
            outer_size,
            monitor_position,
            monitor_size,
            position: LogicalPosition::new(opts.x, opts.y),
            lua_event_tx,
            current_move: None,
            last_move_update: Instant::now(),
            wgpu_state,
            transparent,
            premultiplied_alpha,
            force_opaque,
            current_fade: None,
            last_fade_update: Instant::now(),
            opacity: opts.opacity,
            background_color: opts.background_color,
        })
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity;
    }

    pub fn background_color(&self) -> Option<lua::Color> {
        self.background_color
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

    /// For GPU windows: block until the GPU finishes all submitted work and the DRI3 present
    /// has been submitted to the X server. Call this after a real content draw (e.g. image) so
    /// that XMoveWindow arrives after the frame, ensuring KWin composites content not black.
    ///
    /// For CPU windows this is a no-op — XShmPutImage and XMoveWindow share the X11 connection
    /// so ordering is already guaranteed.
    /// Block until the GPU finishes the submission identified by `idx` and the DRI3 present has
    /// been submitted to the X server. If `idx` is `None` (no submission was made, e.g. due to a
    /// swapchain timeout) this is a no-op.
    pub fn gpu_sync(&self, idx: Option<wgpu::SubmissionIndex>) {
        if let (Some(wgpu), Some(idx)) = (&self.wgpu_state, idx) {
            let _ = wgpu.device.poll(wgpu::PollType::Wait {
                submission_index: Some(idx),
                timeout: None,
            });
        }
    }

    /// For GPU windows: submit a transparent clear frame, then sync (see [`Self::gpu_sync`]).
    /// Use before `set_visible(true)` for windows that have not rendered any real content yet
    /// (video, prompt, choice) so KWin sees transparent pixels rather than uninitialized black.
    ///
    /// For CPU windows this is a no-op.
    pub fn pre_show(&mut self) -> Result<()> {
        if self.is_gpu() {
            let idx = self.draw_wgpu(|_, _, _| {})?;
            self.gpu_sync(idx);
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
    ) -> Result<Option<wgpu::SubmissionIndex>> {
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
                    wgpu::CurrentSurfaceTexture::Timeout => return Ok(None),
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        surface.configure(&wgpu.device, surface_config);
                        self.window.request_redraw();
                        return Ok(None);
                    }
                    wgpu::CurrentSurfaceTexture::Lost => {
                        *surface = wgpu.instance.create_surface(self.window.clone())?;
                        surface.configure(&wgpu.device, surface_config);
                        self.window.request_redraw();
                        return Ok(None);
                    }
                    wgpu::CurrentSurfaceTexture::Occluded => return Ok(None),
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

                let idx = wgpu.queue.submit(Some(encoder.finish()));
                output.present();
                Ok(Some(idx))
            }
            _ => bail!("Called draw_wgpu on a non-GPU surface"),
        }
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

        let monitor_size = self.monitor_size;

        let x: Option<i32> = match opts.x {
            Some(Coord::Pixel(x)) => Some(opts.anchor.resolve(x, size.width)),
            Some(Coord::Percent { percent }) => Some(opts.anchor.resolve(
                ((percent * monitor_size.width as f64) / 100.0).round() as i32,
                size.width,
            )),
            None => None,
        };

        let y: Option<i32> = match opts.y {
            Some(Coord::Pixel(y)) => Some(opts.anchor.resolve(y, size.height)),
            Some(Coord::Percent { percent }) => Some(opts.anchor.resolve(
                ((percent * monitor_size.height as f64) / 100.0).round() as i32,
                size.height,
            )),
            None => None,
        };

        let clamp = opts.clamp;
        let new_position = if opts.relative {
            LogicalPosition::new(
                if clamp { (self.position.x + x.unwrap_or(0)).max(0) } else { self.position.x + x.unwrap_or(0) },
                if clamp { (self.position.y + y.unwrap_or(0)).max(0) } else { self.position.y + y.unwrap_or(0) },
            )
        } else {
            LogicalPosition::new(
                if clamp { x.map(|v| v.max(0)).unwrap_or(self.position.x) } else { x.unwrap_or(self.position.x) },
                if clamp { y.map(|v| v.max(0)).unwrap_or(self.position.y) } else { y.unwrap_or(self.position.y) },
            )
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
                        .round() as i32,
                current_move.from.y
                    + ((current_move.to.y as f64 - current_move.from.y as f64) * eased_percent)
                        .round() as i32,
            );

            let complete = percent >= 1.0;

            // Throttle visual updates to ~30 fps; always apply the final position on completion
            // so the window lands exactly on the wall edge before the next move starts.
            if new_position != self.position
                && (complete
                    || self.last_move_update.elapsed() >= Duration::from_millis(33))
            {
                self.window.set_outer_position(LogicalPosition::new(
                    self.monitor_position.x + new_position.x,
                    self.monitor_position.y + new_position.y,
                ));
                self.position = new_position;
                self.last_move_update = Instant::now();
            }

            if complete {
                if let Err(err) = self.lua_event_tx.send(lua::Event::MoveFinish {
                    id: self.window.id(),
                    move_id: current_move.id,
                    x: self.position.x,
                    y: self.position.y,
                }) {
                    tracing::error!("{err}");
                }

                self.current_move = None;
            }
        }
    }

    pub fn start_fade(&mut self, id: u64, opts: FadeOpts) -> Result<(), LewdwareError> {
        let to = opts.opacity;

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
        let (new_opacity, is_finished, fade_id) =
            if let Some(current_fade) = &self.current_fade {
                let percent = current_fade
                    .start
                    .elapsed()
                    .div_duration_f64(current_fade.duration)
                    .min(1.0);

                let eased_percent = current_fade.easing.apply(percent);

                let new_opacity = current_fade.from
                    + ((current_fade.to - current_fade.from) as f64 * eased_percent) as f32;

                (new_opacity, percent >= 1.0, current_fade.id)
            } else {
                return;
            };

        if new_opacity != self.opacity
            && (is_finished
                || self.last_fade_update.elapsed() >= Duration::from_millis(33))
        {
            self.set_opacity(new_opacity);
            self.window.request_redraw();
            self.last_fade_update = Instant::now();
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
        #[cfg(target_os = "linux")]
        {
            if visible {
                // Move back to the correct absolute position before showing.
                self.window.set_outer_position(LogicalPosition::new(
                    self.monitor_position.x + self.position.x,
                    self.monitor_position.y + self.position.y,
                ));
                self.window.set_visible(true);
                // Recycled (always-mapped) windows are moved with XMoveWindow, which does not
                // restack. Raise explicitly so the window appears above other windows in its layer.
                x11_raise(&self.window);
            } else {
                // XUnmapWindow on a Dock window triggers KWin strut relayout (same freeze as
                // XDestroyWindow). Move offscreen instead of unmapping.
                self.window
                    .set_outer_position(LogicalPosition::new(-32000i32, -32000i32));
            }
        }
        #[cfg(not(target_os = "linux"))]
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

    /// Consume this `InnerWindow` and return the underlying `Arc<Window>` for pool reuse.
    /// `Drop` fires normally, sending `WindowClosed` to Lua and releasing GPU/CPU surfaces.
    pub fn into_arc_window(self) -> Arc<Window> {
        self.window.clone()
    }
}

impl Drop for InnerWindow {
    fn drop(&mut self) {
        if let Err(_) = self.lua_event_tx.send(lua::Event::WindowClosed {
            id: self.window.id(),
        }) {
            // The Lua thread has already shut down (e.g. we're in the middle of quitting the
            // app), so there's nothing listening for this event. Not an error.
            tracing::debug!("Couldn't send WindowClosed event: Lua thread has shut down");
        }
    }
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

/// Raise `window` to the top of the X11 stacking order without unmapping it.
///
/// `XMoveWindow` (used to park/unpark pooled windows) does not restack, so recycled windows
/// would silently sit below any window mapped since they were last visible. `XRaiseWindow`
/// fixes this without triggering the KWin strut relayout that XMapWindow/XUnmapWindow does.
#[cfg(target_os = "linux")]
fn x11_raise(window: &Window) {
    use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

    let (Ok(wh), Ok(dh)) = (window.window_handle(), window.display_handle()) else {
        return;
    };
    let (RawWindowHandle::Xlib(xlib_win), RawDisplayHandle::Xlib(xlib_dpy)) =
        (wh.as_raw(), dh.as_raw())
    else {
        return;
    };
    let Some(display) = xlib_dpy.display else { return };

    let Ok(xlib) = x11_dl::xlib::Xlib::open() else { return };
    unsafe {
        (xlib.XRaiseWindow)(display.as_ptr().cast(), xlib_win.window);
        (xlib.XFlush)(display.as_ptr().cast());
    }
}
