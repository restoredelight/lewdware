use wgpu::util::DeviceExt;
use winit::dpi::{PhysicalPosition, PhysicalSize};

use crate::wgpu::WgpuState;
use crate::window::gpu_renderer::{WindowUniform, upload_texture_data};
use crate::window::header::Header;
use crate::window::redraw::RedrawRequester;
use crate::window::surface::Buffer;
use crate::window::target::RenderTarget;
use crate::window::theme::{BorderRing, Chrome, Metrics, ring_band};

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
    chrome: Chrome,
    border_offset: u32,
    header_height: u32,
}

impl Decorations {
    /// Takes the theme's [`Metrics`] and [`Chrome`] rather than the [`Theme`] itself, so the
    /// geometry and paint can be exercised against sizes no theme has yet — the border and header
    /// arithmetic here is the part that has to hold for the whole catalogue, not just what ships.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        enabled: bool,
        metrics: Metrics,
        chrome: Chrome,
        inner_size: PhysicalSize<u32>,
        scale_factor: f64,
        title: Option<String>,
        closeable: bool,
        redraw: RedrawRequester,
    ) -> Self {
        let (border_offset, header_height) = metrics.physical(scale_factor);

        let inner = enabled.then(|| Inner {
            header: Header::new(
                redraw,
                chrome,
                inner_size,
                scale_factor,
                metrics.header_height,
                title,
                closeable,
            ),
            overlay: None,
            chrome,
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

        let (chrome, border_offset) = (inner.chrome, inner.border_offset);

        inner.overlay = target.is_gpu().then(|| {
            DecorationOverlay::new(
                target.wgpu_state(),
                outer_size.width,
                outer_size.height,
                target.premultiplied_alpha(),
                opacity,
                target.force_opaque(),
                chrome.border,
                border_offset,
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

        buffer.draw_border(inner.chrome.border, inner.border_offset);

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

/// Paint a themed border into a fresh RGBA buffer of `width * height`, leaving the interior fully
/// transparent so the content beneath the overlay shows through.
///
/// The GPU twin of [`Buffer::draw_border`]. Kept separate rather than shared because the two write
/// different pixel formats — RGBA bytes here, softbuffer's packed `0x00RRGGBB` there — but they
/// take the same rings and thickness, and [`ring_band`] gives both the same bands.
fn paint_border_rgba(width: u32, height: u32, rings: &[BorderRing], thickness: u32) -> Vec<u8> {
    let mut data = vec![0u8; (width * height * 4) as usize];
    if rings.is_empty() || thickness == 0 {
        return data;
    }

    let w = width as usize;
    let h = height as usize;

    let put = |data: &mut Vec<u8>, x: usize, y: usize, rgba: [u8; 4]| {
        let index = (y * w + x) * 4;
        data[index..index + 4].copy_from_slice(&rgba);
    };

    for (index, ring) in rings.iter().enumerate() {
        let (start, end) = ring_band(index, rings.len(), thickness);
        let top_left = to_rgba(ring.top_left());
        let bottom_right = to_rgba(ring.bottom_right());

        for inset in start as usize..end as usize {
            if inset * 2 >= w.min(h) {
                break;
            }

            for x in inset..w - inset {
                put(&mut data, x, inset, top_left);
                put(&mut data, x, h - 1 - inset, bottom_right);
            }
            for y in inset..h - inset {
                put(&mut data, inset, y, top_left);
                put(&mut data, w - 1 - inset, y, bottom_right);
            }
        }
    }

    data
}

fn to_rgba(color: crate::lua::Color) -> [u8; 4] {
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [
        channel(color.r),
        channel(color.g),
        channel(color.b),
        channel(color.a),
    ]
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
    /// Create the decoration overlay, painting the themed border into the texture immediately —
    /// it never changes afterwards, unlike the header.
    #[allow(clippy::too_many_arguments)]
    fn new(
        wgpu_state: &WgpuState,
        outer_width: u32,
        outer_height: u32,
        premultiplied_alpha: bool,
        opacity: f32,
        force_opaque: bool,
        border: &[BorderRing],
        border_thickness: u32,
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

        let border_data = paint_border_rgba(outer_width, outer_height, border, border_thickness);
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
    use winit::dpi::LogicalSize;

    use super::*;
    use crate::lua::Color;
    use crate::window::theme::{ALL_THEMES, Appearance, Theme};

    /// Every real theme's metrics, plus edge cases no theme has: a borderless look, and a
    /// thicker bevel than `redmond`'s. The geometry has to hold for the whole catalogue and for
    /// whatever gets added to it.
    fn metric_sets() -> Vec<Metrics> {
        let mut sets: Vec<Metrics> = ALL_THEMES.iter().map(|t| t.metrics()).collect();
        sets.push(Metrics {
            header_height: 32,
            border_width: 0,
        });
        sets.push(Metrics {
            header_height: 24,
            border_width: 4,
        });
        sets
    }

    /// A distinctive ring colour, so a border pixel is never confused with content or header.
    const TEST_RING: u32 = 0xFF12_3456;
    static TEST_RINGS: [BorderRing; 8] = [BorderRing::Uniform(Color {
        r: 0x12 as f32 / 255.0,
        g: 0x34 as f32 / 255.0,
        b: 0x56 as f32 / 255.0,
        a: 1.0,
    }); 8];

    /// `plain`'s chrome with a border of exactly `metrics.border_width` rings, which is the
    /// invariant every real theme must also satisfy (see `border_rings_match_the_border_width`
    /// in `theme.rs`).
    fn chrome_for(metrics: Metrics) -> Chrome {
        Chrome {
            border: &TEST_RINGS[..metrics.border_width as usize],
            ..Theme::Plain.chrome(Appearance::Light)
        }
    }

    fn decorations(metrics: Metrics, scale_factor: f64) -> Decorations {
        Decorations::new(
            true,
            metrics,
            chrome_for(metrics),
            PhysicalSize::new(INNER_W, INNER_H),
            scale_factor,
            None,
            true,
            RedrawRequester::detached(),
        )
    }

    /// The content origin is what every layer positions against, so a rounding change here
    /// silently shifts all window content. It must agree with the metrics it came from, for
    /// every theme and every scale factor.
    #[test]
    fn the_content_origin_clears_the_border_and_header() {
        for metrics in metric_sets() {
            for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
                let (border, header) = metrics.physical(scale);
                assert_eq!(
                    decorations(metrics, scale).content_origin(),
                    (border, border + header),
                    "{metrics:?} at {scale}x"
                );
            }
        }
    }

    #[test]
    fn at_1x_the_content_origin_is_the_logical_metrics() {
        for metrics in metric_sets() {
            assert_eq!(
                decorations(metrics, 1.0).content_origin(),
                (
                    metrics.border_width,
                    metrics.border_width + metrics.header_height
                ),
                "{metrics:?}"
            );
        }
    }

    /// The outer padding a window is sized with and the content origin it is laid out against
    /// were, until this seam existed, computed independently in `lua::api` and here. At integer
    /// scale factors they must compose exactly: content origin, then the content, then the
    /// far border, accounts for the whole window.
    #[test]
    fn outer_padding_and_content_origin_describe_the_same_window() {
        for metrics in metric_sets() {
            for scale in [1u32, 2, 3] {
                let (pad_x, pad_y) = metrics.outer_padding();
                let (outer_w, outer_h) = ((INNER_W + pad_x) * scale, (INNER_H + pad_y) * scale);
                let (border, _) = metrics.physical(scale as f64);
                let (origin_x, origin_y) = decorations(metrics, scale as f64).content_origin();

                assert_eq!(
                    origin_x + INNER_W * scale + border,
                    outer_w,
                    "{metrics:?} width at {scale}x"
                );
                assert_eq!(
                    origin_y + INNER_H * scale + border,
                    outer_h,
                    "{metrics:?} height at {scale}x"
                );
            }
        }
    }

    /// Fractional scale factors round each metric independently, so the exact composition above
    /// cannot be promised — but the content must still fit inside the window it was sized for,
    /// or it would be clipped or drawn over the border.
    #[test]
    fn content_fits_inside_the_window_at_fractional_scale_factors() {
        for metrics in metric_sets() {
            for scale in [1.25, 1.5, 1.75, 2.25, 2.5] {
                let (pad_x, pad_y) = metrics.outer_padding();
                let outer: PhysicalSize<u32> =
                    LogicalSize::new(INNER_W + pad_x, INNER_H + pad_y).to_physical(scale);
                let inner: PhysicalSize<u32> =
                    LogicalSize::new(INNER_W, INNER_H).to_physical(scale);
                let (origin_x, origin_y) = decorations(metrics, scale).content_origin();

                assert!(
                    origin_x + inner.width <= outer.width,
                    "{metrics:?} overflows width at {scale}x"
                );
                assert!(
                    origin_y + inner.height <= outer.height,
                    "{metrics:?} overflows height at {scale}x"
                );
            }
        }
    }

    #[test]
    fn undecorated_windows_put_content_at_the_origin() {
        let decorations = Decorations { inner: None };
        assert_eq!(decorations.content_origin(), (0, 0));
        assert!(!decorations.enabled());
    }

    /// Stands in for already-drawn content, so anything decorations overwrite is visible.
    const CONTENT: u32 = 0x00FF_00FF;

    const INNER_W: u32 = 40;
    const INNER_H: u32 = 30;

    fn outer_size(metrics: Metrics) -> (u32, u32) {
        let (pad_x, pad_y) = metrics.outer_padding();
        (INNER_W + pad_x, INNER_H + pad_y)
    }

    /// Composite decorations over a buffer pre-filled as if content had already been drawn,
    /// exactly as the software render path does. At 1x, so logical and physical agree.
    fn composite_over_content(metrics: Metrics) -> Vec<u32> {
        let (outer_w, outer_h) = outer_size(metrics);

        let mut decorations = Decorations::new(
            true,
            metrics,
            chrome_for(metrics),
            PhysicalSize::new(INNER_W, INNER_H),
            1.0,
            Some("title".to_owned()),
            true,
            RedrawRequester::detached(),
        );

        let mut pixels = vec![CONTENT; (outer_w * outer_h) as usize];
        let mut buffer = Buffer::new(&mut pixels, outer_w, outer_h);
        decorations.draw_cpu(&mut buffer);

        pixels
    }

    fn at(pixels: &[u32], outer_w: u32, x: u32, y: u32) -> u32 {
        pixels[(y * outer_w + x) as usize]
    }

    /// The border is painted over every pixel of the width the layout reserved for it — not just
    /// its outermost ring, which is all the hardcoded 1px stroke used to cover.
    #[test]
    fn the_border_fills_the_width_it_reserved() {
        for metrics in metric_sets().into_iter().filter(|m| m.border_width > 0) {
            let pixels = composite_over_content(metrics);
            let (outer_w, outer_h) = outer_size(metrics);
            let border = metrics.border_width;

            for inset in 0..border {
                for x in inset..outer_w - inset {
                    assert_eq!(
                        at(&pixels, outer_w, x, inset),
                        TEST_RING,
                        "{metrics:?} top ring {inset} at x={x}"
                    );
                    assert_eq!(
                        at(&pixels, outer_w, x, outer_h - 1 - inset),
                        TEST_RING,
                        "{metrics:?} bottom ring {inset} at x={x}"
                    );
                }
                for y in inset..outer_h - inset {
                    assert_eq!(
                        at(&pixels, outer_w, inset, y),
                        TEST_RING,
                        "{metrics:?} left ring {inset} at y={y}"
                    );
                    assert_eq!(
                        at(&pixels, outer_w, outer_w - 1 - inset, y),
                        TEST_RING,
                        "{metrics:?} right ring {inset} at y={y}"
                    );
                }
            }
        }
    }

    /// A bevel takes one colour on the top and left edges and another on the bottom and right —
    /// the whole point of the ring vocabulary, and what a Win95 3D edge is made of.
    #[test]
    fn a_bevel_ring_splits_its_edges() {
        const LIGHT: u32 = 0xFFFF_FFFF;
        const DARK: u32 = 0xFF80_8080;
        static BEVEL: [BorderRing; 1] = [BorderRing::Bevel {
            top_left: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            bottom_right: Color {
                r: 128.0 / 255.0,
                g: 128.0 / 255.0,
                b: 128.0 / 255.0,
                a: 1.0,
            },
        }];

        let metrics = Metrics {
            header_height: 10,
            border_width: 1,
        };
        let (outer_w, outer_h) = outer_size(metrics);
        let chrome = Chrome {
            border: &BEVEL,
            ..Theme::Plain.chrome(Appearance::Light)
        };

        let mut decorations = Decorations::new(
            true,
            metrics,
            chrome,
            PhysicalSize::new(INNER_W, INNER_H),
            1.0,
            None,
            true,
            RedrawRequester::detached(),
        );

        let mut pixels = vec![CONTENT; (outer_w * outer_h) as usize];
        let mut buffer = Buffer::new(&mut pixels, outer_w, outer_h);
        decorations.draw_cpu(&mut buffer);

        assert_eq!(at(&pixels, outer_w, outer_w / 2, 0), LIGHT, "top edge");
        assert_eq!(at(&pixels, outer_w, 0, outer_h / 2), LIGHT, "left edge");
        assert_eq!(
            at(&pixels, outer_w, outer_w / 2, outer_h - 1),
            DARK,
            "bottom edge"
        );
        assert_eq!(
            at(&pixels, outer_w, outer_w - 1, outer_h / 2),
            DARK,
            "right edge"
        );
    }

    /// The header is inset by the border on *both* axes — it is not pushed down by its own
    /// height, which is what `content_origin` adds for the content beneath it.
    #[test]
    fn the_header_occupies_the_band_between_border_and_content() {
        for metrics in metric_sets() {
            let pixels = composite_over_content(metrics);
            let (outer_w, _) = outer_size(metrics);
            let (_, content_top) = decorations(metrics, 1.0).content_origin();
            let border = metrics.border_width;

            // Every interior pixel of the header band was painted over.
            for y in border..content_top {
                for x in border..outer_w - border {
                    assert_ne!(
                        at(&pixels, outer_w, x, y),
                        CONTENT,
                        "{metrics:?} header did not cover ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn decorations_leave_the_content_area_alone() {
        for metrics in metric_sets() {
            let pixels = composite_over_content(metrics);
            let (outer_w, _) = outer_size(metrics);
            let (content_x, content_y) = decorations(metrics, 1.0).content_origin();

            for y in content_y..content_y + INNER_H {
                for x in content_x..content_x + INNER_W {
                    assert_eq!(
                        at(&pixels, outer_w, x, y),
                        CONTENT,
                        "{metrics:?} content clobbered at ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn undecorated_windows_draw_nothing() {
        let (outer_w, outer_h) = outer_size(Metrics::PLAIN);
        let mut decorations = Decorations { inner: None };
        let mut pixels = vec![CONTENT; (outer_w * outer_h) as usize];
        let mut buffer = Buffer::new(&mut pixels, outer_w, outer_h);
        decorations.draw_cpu(&mut buffer);

        assert!(pixels.iter().all(|&p| p == CONTENT));
    }
}
