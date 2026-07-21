use wgpu::util::DeviceExt;
use winit::dpi::{PhysicalPosition, PhysicalSize, PhysicalUnit};

use crate::wgpu::WgpuState;
use crate::window::gpu_renderer::{WindowUniform, upload_texture_data};
use crate::window::header::{HEADER_HEIGHT, Header};
use crate::window::redraw::RedrawRequester;
use crate::window::surface::Buffer;
use crate::window::target::RenderTarget;

/// A window's border and title bar.
///
/// Content renderers never draw decorations themselves: they draw within the content rect (see
/// [`Self::content_origin`]) and this composites the border and header on top. On GPU windows
/// that means a dedicated overlay texture; on CPU windows it means writing into the softbuffer
/// after the content.
///
/// `None` inside means the window is undecorated — every method is then a no-op and the content
/// rect is the whole window.
pub struct Decorations {
    inner: Option<Inner>,
}

struct Inner {
    header: Header,
    /// GPU windows only; attached once the render target exists (see [`Decorations::attach`]).
    overlay: Option<DecorationOverlay>,
    border_offset: u32,
    header_height: u32,
}

impl Decorations {
    pub fn new(
        enabled: bool,
        inner_size: PhysicalSize<u32>,
        scale_factor: f64,
        title: Option<String>,
        closeable: bool,
        redraw: RedrawRequester,
    ) -> Self {
        let (border_offset, header_height) = metrics(scale_factor);

        let inner = enabled.then(|| Inner {
            header: Header::new(redraw, inner_size, scale_factor, title, closeable),
            overlay: None,
            border_offset,
            header_height,
        });

        Self { inner }
    }

    /// Build the GPU overlay texture, if this is a decorated GPU window. Separate from `new`
    /// because the render target — and therefore whether the window is GPU-backed at all — is
    /// only known after the window state has been built.
    pub fn attach(&mut self, target: &RenderTarget, outer_size: PhysicalSize<u32>, opacity: f32) {
        let Some(inner) = &mut self.inner else {
            return;
        };

        inner.overlay = target.is_gpu().then(|| {
            DecorationOverlay::new(
                target.wgpu_state(),
                outer_size.width,
                outer_size.height,
                target.premultiplied_alpha(),
                opacity,
                target.force_opaque(),
            )
        });
    }

    pub fn enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// Origin of the content area within the outer window, in physical pixels.
    pub fn content_origin(&self) -> (u32, u32) {
        match &self.inner {
            Some(inner) => (
                inner.border_offset,
                inner.border_offset + inner.header_height,
            ),
            None => (0, 0),
        }
    }

    /// Refresh the GPU overlay before a frame: re-upload the header if it changed, and track the
    /// window's current opacity.
    ///
    /// Returns whether the header actually changed — callers that only draw on demand (video,
    /// which otherwise redraws only when a new frame decodes) need to force a frame so the
    /// updated decorations reach the screen.
    pub fn prepare_gpu(&mut self, queue: &wgpu::Queue, opacity: f32) -> bool {
        let Some(inner) = &mut self.inner else {
            return false;
        };
        let Some(overlay) = &inner.overlay else {
            return false;
        };

        // The header sits inside the border, so both of its origins are `border_offset` — the
        // header height only shifts the *content* below it, not the header itself.
        let changed = if let Some(pixmap) = inner.header.draw() {
            overlay.upload_header(queue, pixmap, inner.border_offset, inner.border_offset);
            true
        } else {
            false
        };

        overlay.set_opacity(queue, opacity);

        changed
    }

    /// Composite the overlay over the content already in the pass.
    pub fn draw_gpu(&self, rpass: &mut wgpu::RenderPass<'static>, pipeline: &wgpu::RenderPipeline) {
        if let Some(overlay) = self.inner.as_ref().and_then(|inner| inner.overlay.as_ref()) {
            overlay.render(rpass, pipeline);
        }
    }

    /// Write the border and header into a CPU buffer, over the content already in it.
    ///
    /// Unconditional: softbuffer buffers are not guaranteed to retain content across frames (e.g.
    /// macOS CALayer backing store), so nothing carries over from the previous frame.
    pub fn draw_cpu(&mut self, buffer: &mut Buffer) {
        let Some(inner) = &mut self.inner else {
            return;
        };

        buffer.draw_border();

        let pixmap = inner.header.get_pixmap();
        buffer.copy_from_pixmap(pixmap, inner.border_offset, inner.border_offset);
    }

    pub fn set_title(&mut self, text: Option<String>) {
        if let Some(inner) = &mut self.inner {
            inner.header.set_title(text);
        }
    }

    pub fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if let Some(inner) = &mut self.inner {
            inner.header.handle_cursor_moved(position);
        }
    }

    pub fn handle_cursor_left(&mut self) {
        if let Some(inner) = &mut self.inner {
            inner.header.handle_cursor_left();
        }
    }

    pub fn handle_mouse_down(&mut self) {
        if let Some(inner) = &mut self.inner {
            inner.header.handle_mouse_down();
        }
    }

    /// Returns whether the close button was activated.
    pub fn handle_mouse_up(&mut self) -> bool {
        match &mut self.inner {
            Some(inner) => inner.header.handle_mouse_up(),
            None => false,
        }
    }
}

/// The physical-pixel size of the border and the title bar at a given scale factor.
fn metrics(scale_factor: f64) -> (u32, u32) {
    (
        PhysicalUnit::from_logical::<_, u32>(1, scale_factor).0,
        PhysicalUnit::from_logical::<_, u32>(HEADER_HEIGHT, scale_factor).0,
    )
}

/// GPU overlay texture for window decorations (border + header). The texture is the same size as
/// the outer window and is composited on top of the content layer using alpha blending.
struct DecorationOverlay {
    texture: wgpu::Texture,
    // Bind group for the overlay texture (group 0, RGBA pipeline).
    bind_group: wgpu::BindGroup,
    // Opacity + premultiplied uniform (group 1, RGBA pipeline).
    opacity_buffer: wgpu::Buffer,
    window_bind_group: wgpu::BindGroup,
    outer_width: u32,
    outer_height: u32,
}

impl DecorationOverlay {
    /// Create the decoration overlay. Draws a 1-pixel border into the texture immediately.
    fn new(
        wgpu_state: &WgpuState,
        outer_width: u32,
        outer_height: u32,
        premultiplied_alpha: bool,
        opacity: f32,
        force_opaque: bool,
    ) -> Self {
        let device = &wgpu_state.device;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Decoration Overlay Texture"),
            size: wgpu::Extent3d {
                width: outer_width,
                height: outer_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Decoration Overlay Bind Group"),
            layout: &wgpu_state.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&wgpu_state.sampler),
                },
            ],
        });

        let opacity_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Decoration Opacity Buffer"),
            contents: bytemuck::bytes_of(&WindowUniform {
                opacity,
                premultiplied: premultiplied_alpha as u32,
                force_opaque: force_opaque as u32,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let window_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Decoration Window Bind Group"),
            layout: &wgpu_state.window_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: opacity_buffer.as_entire_binding(),
            }],
        });

        // Draw 1-pixel black border into the texture immediately.
        let mut border_data = vec![0u8; (outer_width * outer_height * 4) as usize];
        let black = [0u8, 0, 0, 255];
        for x in 0..outer_width as usize {
            border_data[x * 4..x * 4 + 4].copy_from_slice(&black);
            let bot = ((outer_height as usize - 1) * outer_width as usize + x) * 4;
            border_data[bot..bot + 4].copy_from_slice(&black);
        }
        for y in 0..outer_height as usize {
            let left = y * outer_width as usize * 4;
            border_data[left..left + 4].copy_from_slice(&black);
            let right = (y * outer_width as usize + outer_width as usize - 1) * 4;
            border_data[right..right + 4].copy_from_slice(&black);
        }
        upload_texture_data(
            &wgpu_state.queue,
            &texture,
            &border_data,
            outer_width,
            outer_height,
            outer_width * 4,
            4,
        );

        Self {
            texture,
            bind_group,
            opacity_buffer,
            window_bind_group,
            outer_width,
            outer_height,
        }
    }

    fn set_opacity(&self, queue: &wgpu::Queue, opacity: f32) {
        queue.write_buffer(&self.opacity_buffer, 0, bytemuck::cast_slice(&[opacity]));
    }

    /// Upload a header pixmap into the overlay texture at `(origin_x, origin_y)`.
    fn upload_header(
        &self,
        queue: &wgpu::Queue,
        pixmap: &tiny_skia::Pixmap,
        origin_x: u32,
        origin_y: u32,
    ) {
        let width = pixmap.width();
        let height = pixmap.height();
        let data = pixmap.data();
        let bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padding = (align - bytes_per_row % align) % align;
        let padded_bpr = bytes_per_row + padding;

        let padded: Vec<u8> = if padding == 0 {
            data.to_vec()
        } else {
            let mut v = Vec::with_capacity((padded_bpr * height) as usize);
            for row in data.chunks_exact(bytes_per_row as usize) {
                v.extend_from_slice(row);
                v.extend(std::iter::repeat_n(0u8, padding as usize));
            }
            v
        };

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: origin_x,
                    y: origin_y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &padded,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Blit the decoration overlay into the active render pass (full outer-window viewport).
    fn render(&self, rpass: &mut wgpu::RenderPass<'static>, pipeline: &wgpu::RenderPipeline) {
        rpass.set_pipeline(pipeline);
        rpass.set_bind_group(0, &self.bind_group, &[]);
        rpass.set_bind_group(1, &self.window_bind_group, &[]);
        rpass.set_viewport(
            0.0,
            0.0,
            self.outer_width as f32,
            self.outer_height as f32,
            0.0,
            1.0,
        );
        rpass.draw(0..4, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The content origin is what every layer positions against, so a rounding change here
    /// silently shifts all window content.
    fn content_origin(scale_factor: f64) -> (u32, u32) {
        let (border, header) = metrics(scale_factor);
        (border, border + header)
    }

    #[test]
    fn at_1x_the_content_clears_the_border_and_header() {
        assert_eq!(metrics(1.0), (1, HEADER_HEIGHT));
        assert_eq!(content_origin(1.0), (1, 1 + HEADER_HEIGHT));
    }

    #[test]
    fn integer_scaling_multiplies_both_metrics() {
        assert_eq!(metrics(2.0), (2, HEADER_HEIGHT * 2));
        assert_eq!(content_origin(2.0), (2, 2 + HEADER_HEIGHT * 2));
    }

    /// Fractional scale factors are where this is most likely to drift: the border must never
    /// round to zero, or the content would be drawn over it.
    #[test]
    fn fractional_scaling_keeps_a_visible_border() {
        for scale in [1.25, 1.5, 1.75, 2.5] {
            let (border, header) = metrics(scale);
            assert!(border >= 1, "border vanished at {scale}x");
            assert!(header >= HEADER_HEIGHT, "header shrank at {scale}x");
        }
    }

    #[test]
    fn undecorated_windows_put_content_at_the_origin() {
        let decorations = Decorations { inner: None };
        assert_eq!(decorations.content_origin(), (0, 0));
        assert!(!decorations.enabled());
    }

    const BLACK: u32 = 0xFF00_0000;
    /// Stands in for already-drawn content, so anything decorations overwrite is visible.
    const CONTENT: u32 = 0x00FF_00FF;

    const INNER_W: u32 = 40;
    const INNER_H: u32 = 30;
    // 1px border each side; the header sits between the top border and the content.
    const OUTER_W: u32 = INNER_W + 2;
    const OUTER_H: u32 = INNER_H + HEADER_HEIGHT + 2;

    /// Composite decorations over a buffer pre-filled as if content had already been drawn,
    /// exactly as the software render path does.
    fn composite_over_content() -> Vec<u32> {
        let mut decorations = Decorations::new(
            true,
            PhysicalSize::new(INNER_W, INNER_H),
            1.0,
            Some("title".to_owned()),
            true,
            RedrawRequester::detached(),
        );

        let mut pixels = vec![CONTENT; (OUTER_W * OUTER_H) as usize];
        let mut buffer = Buffer::new(&mut pixels, OUTER_W, OUTER_H);
        decorations.draw_cpu(&mut buffer);

        pixels
    }

    fn at(pixels: &[u32], x: u32, y: u32) -> u32 {
        pixels[(y * OUTER_W + x) as usize]
    }

    #[test]
    fn decorations_frame_the_window() {
        let pixels = composite_over_content();

        for x in 0..OUTER_W {
            assert_eq!(at(&pixels, x, 0), BLACK, "top border at x={x}");
            assert_eq!(at(&pixels, x, OUTER_H - 1), BLACK, "bottom border at x={x}");
        }
        for y in 0..OUTER_H {
            assert_eq!(at(&pixels, 0, y), BLACK, "left border at y={y}");
            assert_eq!(at(&pixels, OUTER_W - 1, y), BLACK, "right border at y={y}");
        }
    }

    /// The header is inset by the border on *both* axes — it is not pushed down by its own
    /// height, which is what `content_origin` adds for the content beneath it.
    #[test]
    fn the_header_occupies_the_band_between_border_and_content() {
        let pixels = composite_over_content();
        let (_, content_top) = Decorations::new(
            true,
            PhysicalSize::new(INNER_W, INNER_H),
            1.0,
            None,
            true,
            RedrawRequester::detached(),
        )
        .content_origin();

        // Every interior pixel of the header band was painted over.
        for y in 1..content_top {
            for x in 1..OUTER_W - 1 {
                assert_ne!(
                    at(&pixels, x, y),
                    CONTENT,
                    "header did not cover ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn decorations_leave_the_content_area_alone() {
        let pixels = composite_over_content();
        let (content_x, content_y) = Decorations::new(
            true,
            PhysicalSize::new(INNER_W, INNER_H),
            1.0,
            None,
            true,
            RedrawRequester::detached(),
        )
        .content_origin();

        for y in content_y..content_y + INNER_H {
            for x in content_x..content_x + INNER_W {
                assert_eq!(
                    at(&pixels, x, y),
                    CONTENT,
                    "content clobbered at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn undecorated_windows_draw_nothing() {
        let mut decorations = Decorations { inner: None };
        let mut pixels = vec![CONTENT; (OUTER_W * OUTER_H) as usize];
        let mut buffer = Buffer::new(&mut pixels, OUTER_W, OUTER_H);
        decorations.draw_cpu(&mut buffer);

        assert!(pixels.iter().all(|&p| p == CONTENT));
    }
}
