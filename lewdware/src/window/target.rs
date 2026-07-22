use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result, anyhow, bail};
use shared::once;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::window::Window;

use crate::wgpu::WgpuState;
use crate::window::opts::WindowOpts;
use crate::window::redraw::RedrawRequester;
use crate::window::surface::{Buffer, Surface};

/// Where a window's pixels go: either a wgpu swapchain surface or a softbuffer CPU surface.
///
/// This owns everything backend-specific — surface configuration, frame acquisition, the shader
/// alpha flags derived from the surface's composite-alpha mode — and nothing about *what* is
/// drawn. Callers hand it a draw closure via [`Self::draw_wgpu`] / [`Self::draw_softbuffer`].
pub struct RenderTarget {
    /// `None` for an offscreen target, which has no window to recreate a surface against.
    window: Option<Arc<Window>>,
    redraw: Option<RedrawRequester>,
    surface: Surface,
    wgpu_state: Option<Arc<WgpuState>>,
    outer_size: PhysicalSize<u32>,
    clear_color: wgpu::Color,
    premultiplied_alpha: bool,
    force_opaque: bool,
}

impl RenderTarget {
    pub fn new(
        window: Arc<Window>,
        opts: &WindowOpts,
        wgpu_state: Option<Arc<WgpuState>>,
        redraw: RedrawRequester,
    ) -> Result<Self> {
        let gpu = opts.popup_opts.gpu;
        let transparent = opts.popup_opts.transparent;
        let force_opaque = opts.popup_opts.force_opaque;
        // Use opts directly rather than window.inner_size(): request_inner_size() is
        // async on X11, so a recycled pool window still reports its previous size here.
        let scale_factor = window.scale_factor();
        let outer_size: PhysicalSize<u32> =
            LogicalSize::new(opts.popup_opts.outer_width, opts.popup_opts.outer_height)
                .to_physical(scale_factor);

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

        let clear_color = if transparent {
            wgpu::Color::TRANSPARENT
        } else {
            wgpu::Color::BLACK
        };

        Ok(Self {
            window: Some(window),
            redraw: Some(redraw),
            surface,
            wgpu_state,
            outer_size,
            clear_color,
            premultiplied_alpha,
            force_opaque,
        })
    }

    pub fn is_gpu(&self) -> bool {
        self.surface.is_gpu()
    }

    pub fn wgpu_state(&self) -> &Arc<WgpuState> {
        self.wgpu_state.as_ref().unwrap()
    }

    pub fn surface_format(&self) -> Option<wgpu::TextureFormat> {
        match &self.surface {
            Surface::Wgpu { surface_config, .. } => Some(surface_config.format),
            #[cfg(test)]
            Surface::Offscreen { format, .. } => Some(*format),
            _ => None,
        }
    }

    /// The shared RGBA blit pipeline for this surface's format. Panics on a CPU target.
    pub fn rgba_pipeline(&self) -> std::sync::Arc<wgpu::RenderPipeline> {
        self.wgpu_state()
            .get_pipeline(self.surface_format().unwrap())
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

    /// Prepare the surface for a frame. Returns whether the backend changed — a wgpu device
    /// loss falls back to software here, and everything the caller built against the old
    /// backend (layer resources, the decoration overlay) is invalid from that point on.
    #[must_use = "a backend change invalidates every GPU resource built against the old surface"]
    pub fn start_render(&mut self) -> Result<bool> {
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
                    let (context, surface) =
                        init_softbuffer(self.window.clone().expect("windowed target"))?;

                    self.surface = Surface::Softbuffer {
                        _context: context,
                        surface,
                    };

                    self.start_render()?;
                    return Ok(true);
                }
            }
            #[cfg(test)]
            Surface::Offscreen { .. } => {}
            Surface::Softbuffer { _context, surface } => {
                surface
                    .resize(
                        NonZeroU32::new(self.outer_size.width).context("Window has 0 width")?,
                        NonZeroU32::new(self.outer_size.height).context("Window has 0 height")?,
                    )
                    .map_err(|err| anyhow!("{}", err))?;
            }
        }

        Ok(false)
    }

    /// For GPU windows: block until the GPU finishes the submission identified by `idx` and the
    /// DRI3 present has been submitted to the X server. Call this after a real content draw (e.g.
    /// image) so that XMoveWindow arrives after the frame, ensuring KWin composites content not
    /// black. If `idx` is `None` (no submission was made, e.g. due to a swapchain timeout) this is
    /// a no-op.
    ///
    /// For CPU windows this is a no-op — XShmPutImage and XMoveWindow share the X11 connection
    /// so ordering is already guaranteed.
    pub fn gpu_sync(&self, idx: Option<wgpu::SubmissionIndex>) {
        if let (Some(wgpu), Some(idx)) = (&self.wgpu_state, idx) {
            let _ = wgpu.device.poll(wgpu::PollType::Wait {
                submission_index: Some(idx),
                timeout: None,
            });
        }
    }

    /// For GPU windows: submit a transparent clear frame, then sync (see [`Self::gpu_sync`]).
    /// Use before `show()` for windows that have not rendered any real content yet (video,
    /// prompt, choice) so KWin sees transparent pixels rather than uninitialized black.
    ///
    /// For CPU windows this is a no-op.
    pub fn pre_show(&mut self) -> Result<()> {
        if self.is_gpu() {
            let idx = self.draw_wgpu(|_| {})?;
            self.gpu_sync(idx);
        }
        Ok(())
    }

    /// Acquire a swapchain frame, open a render pass cleared to the window's clear colour, and
    /// hand it to `draw_fn`. The viewport starts out covering the whole outer window; callers
    /// that draw into a sub-rect are responsible for narrowing it.
    /// Acquire a swapchain frame, open a render pass cleared to the window's clear colour, and
    /// hand it to `draw_fn`. The viewport starts out covering the whole outer window; callers
    /// that draw into a sub-rect are responsible for narrowing it.
    pub fn draw_wgpu(
        &mut self,
        draw_fn: impl FnOnce(&mut wgpu::RenderPass<'static>),
    ) -> Result<Option<wgpu::SubmissionIndex>> {
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
                        self.request_redraw();
                        return Ok(None);
                    }
                    wgpu::CurrentSurfaceTexture::Lost => {
                        *surface = wgpu
                            .instance
                            .create_surface(self.window.clone().expect("windowed target"))?;
                        surface.configure(&wgpu.device, surface_config);
                        self.request_redraw();
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

                let idx = record_frame(
                    wgpu,
                    &view,
                    self.clear_color,
                    surface_config.width,
                    surface_config.height,
                    draw_fn,
                );

                output.present();
                Ok(Some(idx))
            }
            #[cfg(test)]
            Surface::Offscreen { texture, .. } => {
                let wgpu = self.wgpu_state.as_ref().unwrap();
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

                Ok(Some(record_frame(
                    wgpu,
                    &view,
                    self.clear_color,
                    self.outer_size.width,
                    self.outer_size.height,
                    draw_fn,
                )))
            }
            _ => bail!("Called draw_wgpu on a non-GPU surface"),
        }
    }

    fn request_redraw(&self) {
        if let Some(redraw) = &self.redraw {
            redraw.request_redraw();
        }
    }

    /// Map the softbuffer back buffer, hand it to `draw_fn`, then present it.
    ///
    /// The closure is responsible for everything that must appear in the frame, decorations
    /// included — softbuffer buffers are not guaranteed to retain content across frames (e.g.
    /// macOS CALayer backing store), so nothing carries over from the previous call.
    pub fn draw_softbuffer(&mut self, draw_fn: impl FnOnce(&mut Buffer)) -> Result<()> {
        let softbuffer_surface = match &mut self.surface {
            Surface::Softbuffer { surface, .. } => surface,
            _ => bail!("Called draw_softbuffer on a non-CPU surface"),
        };

        let mut back_buffer = softbuffer_surface
            .buffer_mut()
            .map_err(|err| anyhow!("{err}"))?;
        let width = back_buffer.width().get();
        let height = back_buffer.height().get();

        {
            let mut buffer = Buffer::new(&mut back_buffer, width, height);
            draw_fn(&mut buffer);
        }

        back_buffer.present().map_err(|err| anyhow!("{err}"))?;

        Ok(())
    }
}

#[cfg(test)]
impl RenderTarget {
    /// A GPU target backed by a texture instead of a window's swapchain, so a frame can be
    /// rendered and read back headlessly. Uses the same `draw_wgpu` path as a real window.
    pub fn offscreen(
        wgpu_state: Arc<WgpuState>,
        size: PhysicalSize<u32>,
        clear_color: wgpu::Color,
    ) -> Self {
        const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

        let texture = wgpu_state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Offscreen Target"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        Self {
            window: None,
            redraw: None,
            surface: Surface::Offscreen {
                texture,
                format: FORMAT,
            },
            wgpu_state: Some(wgpu_state),
            outer_size: size,
            clear_color,
            premultiplied_alpha: false,
            force_opaque: false,
        }
    }

    /// Read the rendered texture back as RGBA rows, tightly packed.
    pub fn read_back(&self) -> Vec<u8> {
        let Surface::Offscreen { texture, .. } = &self.surface else {
            panic!("read_back on a non-offscreen target");
        };
        let wgpu = self.wgpu_state.as_ref().unwrap();

        let width = self.outer_size.width;
        let height = self.outer_size.height;
        let unpadded = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;

        let staging = wgpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Offscreen Readback"),
            size: (padded * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = wgpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        wgpu.queue.submit(Some(encoder.finish()));

        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = wgpu.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });

        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((unpadded * height) as usize);
        for row in 0..height {
            let start = (row * padded) as usize;
            pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
        }

        drop(mapped);
        staging.unmap();

        pixels
    }
}

/// Clear `view`, open a render pass over the whole target, run `draw_fn`, and submit.
fn record_frame(
    wgpu: &WgpuState,
    view: &wgpu::TextureView,
    clear_color: wgpu::Color,
    width: u32,
    height: u32,
    draw_fn: impl FnOnce(&mut wgpu::RenderPass<'static>),
) -> wgpu::SubmissionIndex {
    let mut encoder = wgpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Wgpu Surface Render Encoder"),
        });

    {
        let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color),
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

        rpass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);

        draw_fn(&mut rpass);
    }

    wgpu.queue.submit(Some(encoder.finish()))
}

type SoftbufferContextAndSurface = (
    softbuffer::Context<Arc<Window>>,
    softbuffer::Surface<Arc<Window>, Arc<Window>>,
);

fn init_softbuffer(window: Arc<Window>) -> Result<SoftbufferContextAndSurface> {
    let context = softbuffer::Context::new(window.clone()).map_err(|err| anyhow!("{}", err))?;
    let surface =
        softbuffer::Surface::new(&context, window.clone()).map_err(|err| anyhow!("{}", err))?;

    Ok((context, surface))
}
