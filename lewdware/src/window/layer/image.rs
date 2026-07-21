use anyhow::Result;
use tiny_skia::{IntSize, Pixmap};

use crate::media::ImageData;
use crate::window::gpu_renderer::{GpuRenderer, GpuRendererType};
use crate::window::layer::{BackendInit, CpuFrame, GpuFrame, LayerInit, LayerStatus, Rect};
use crate::window::surface::Buffer;

/// A still image.
///
/// The decoded pixels are kept even on the GPU path: they are what a rebuild after a
/// GPU-to-software fallback re-uploads from, and they are cheap relative to the surface.
pub struct ImageLayer {
    pixmap: Pixmap,
    backend: ImageBackend,
}

enum ImageBackend {
    Gpu(Box<GpuRenderer>),
    Cpu,
}

impl ImageLayer {
    pub fn new(init: &LayerInit<'_>, image: ImageData) -> Result<Self> {
        let width = image.width();
        let height = image.height();
        let pixmap =
            Pixmap::from_vec(image.into_vec(), IntSize::from_wh(width, height).unwrap()).unwrap();

        let backend = Self::build_backend(init, &pixmap);

        Ok(Self { pixmap, backend })
    }

    fn build_backend(init: &LayerInit<'_>, pixmap: &Pixmap) -> ImageBackend {
        let BackendInit::Gpu {
            wgpu,
            premultiplied_alpha,
            force_opaque,
            ..
        } = init.backend
        else {
            return ImageBackend::Cpu;
        };

        let renderer = GpuRenderer::new_image(
            wgpu,
            pixmap.width(),
            pixmap.height(),
            init.opacity,
            premultiplied_alpha,
            force_opaque,
        );
        // The image never changes, so this is the only upload it ever needs.
        renderer.upload_image(&wgpu.queue, pixmap.data(), pixmap.width(), pixmap.height());

        ImageBackend::Gpu(Box::new(renderer))
    }

    /// Rebuild GPU/CPU resources after the render target changed backend.
    pub fn rebuild(&mut self, init: &LayerInit<'_>) -> Result<()> {
        self.backend = Self::build_backend(init, &self.pixmap);
        Ok(())
    }

    /// Where the image sits: its own size, at the content origin. Normally identical to the
    /// content rect, since the window is sized to the image.
    fn rect(&self, content: Rect) -> Rect {
        Rect {
            x: content.x,
            y: content.y,
            width: self.pixmap.width(),
            height: self.pixmap.height(),
        }
    }

    pub fn prepare_gpu(&mut self, frame: &GpuFrame<'_>) -> Result<LayerStatus> {
        if let ImageBackend::Gpu(renderer) = &self.backend {
            renderer.set_opacity(frame.wgpu, frame.opacity);
        }
        Ok(LayerStatus::Draw)
    }

    pub fn draw_gpu(&self, rpass: &mut wgpu::RenderPass<'static>, frame: &GpuFrame<'_>) {
        let ImageBackend::Gpu(renderer) = &self.backend else {
            return;
        };
        let GpuRendererType::Image { bind_group, .. } = &renderer.renderer_type else {
            return;
        };

        rpass.set_pipeline(frame.pipeline);
        rpass.set_bind_group(0, bind_group, &[]);
        rpass.set_bind_group(1, &renderer.window_bind_group, &[]);
        self.rect(frame.content).set_viewport(rpass);
        rpass.draw(0..4, 0..1);
    }

    pub fn prepare_cpu(&mut self, _frame: &CpuFrame) -> Result<LayerStatus> {
        Ok(LayerStatus::Draw)
    }

    pub fn draw_cpu(&mut self, buffer: &mut Buffer, frame: &CpuFrame) {
        let rect = self.rect(frame.content);
        buffer.copy_from_pixmap(&self.pixmap, rect.x, rect.y);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use winit::dpi::PhysicalSize;

    use super::*;
    use crate::media::ImageData;
    use crate::wgpu::WgpuState;
    use crate::window::decorations::Decorations;
    use crate::window::layer::Rect;
    use crate::window::redraw::RedrawRequester;
    use crate::window::target::RenderTarget;

    const IMG_W: u32 = 8;
    const IMG_H: u32 = 6;
    const BORDER: u32 = 1;
    // Matches the decorated layout: border on every side, header above the content.
    const OUTER_W: u32 = IMG_W + BORDER * 2;
    const OUTER_H: u32 = IMG_H + HEADER_PX + BORDER * 2;
    const HEADER_PX: u32 = crate::window::HEADER_HEIGHT;

    const CONTENT: Rect = Rect {
        x: BORDER,
        y: BORDER + HEADER_PX,
        width: IMG_W,
        height: IMG_H,
    };

    /// A headless device. `None` when no adapter is available, so the tests skip rather than
    /// fail on a machine without one.
    fn wgpu_state() -> Option<Arc<WgpuState>> {
        pollster::block_on(WgpuState::headless()).ok().map(Arc::new)
    }

    fn solid_image(rgba: [u8; 4]) -> ImageData {
        let pixels: Vec<u8> = rgba
            .iter()
            .copied()
            .cycle()
            .take((IMG_W * IMG_H * 4) as usize)
            .collect();
        ImageData::from_raw(IMG_W, IMG_H, pixels).unwrap()
    }

    fn pixel_at(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * OUTER_W + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    }

    /// Render an image layer through the production `draw_wgpu` path into an offscreen target,
    /// with decorations composited on top, and read the result back.
    fn render_decorated_image(rgba: [u8; 4]) -> Option<Vec<u8>> {
        let wgpu = wgpu_state()?;

        let mut target = RenderTarget::offscreen(
            wgpu.clone(),
            PhysicalSize::new(OUTER_W, OUTER_H),
            wgpu::Color::TRANSPARENT,
        );

        let init = LayerInit {
            backend: BackendInit::Gpu {
                wgpu: &wgpu,
                format: target.surface_format().unwrap(),
                premultiplied_alpha: false,
                force_opaque: false,
            },
            content: CONTENT,
            opacity: 1.0,
        };

        let layer = ImageLayer::new(&init, solid_image(rgba)).unwrap();

        let mut decorations = Decorations::new(
            true,
            PhysicalSize::new(IMG_W, IMG_H),
            1.0,
            None,
            true,
            RedrawRequester::detached(),
        );
        decorations.attach(&target, PhysicalSize::new(OUTER_W, OUTER_H), 1.0);
        decorations.prepare_gpu(&wgpu.queue, 1.0);

        let pipeline = target.rgba_pipeline();
        let frame = GpuFrame {
            wgpu: &wgpu,
            pipeline: &pipeline,
            content: CONTENT,
            opacity: 1.0,
        };

        target
            .draw_wgpu(|rpass| {
                layer.draw_gpu(rpass, &frame);
                decorations.draw_gpu(rpass, &pipeline);
            })
            .unwrap();

        Some(target.read_back())
    }

    /// Render an image layer alone into a target larger than it, so anything drawn outside its
    /// rect is visible against the transparent clear colour.
    fn render_image_only(target_size: PhysicalSize<u32>, content: Rect) -> Option<Vec<u8>> {
        let wgpu = wgpu_state()?;

        let mut target =
            RenderTarget::offscreen(wgpu.clone(), target_size, wgpu::Color::TRANSPARENT);

        let init = LayerInit {
            backend: BackendInit::Gpu {
                wgpu: &wgpu,
                format: target.surface_format().unwrap(),
                premultiplied_alpha: false,
                force_opaque: false,
            },
            content,
            opacity: 1.0,
        };

        let layer = ImageLayer::new(&init, solid_image([255, 0, 0, 255])).unwrap();

        let pipeline = target.rgba_pipeline();
        let frame = GpuFrame {
            wgpu: &wgpu,
            pipeline: &pipeline,
            content,
            opacity: 1.0,
        };

        target
            .draw_wgpu(|rpass| layer.draw_gpu(rpass, &frame))
            .unwrap();

        Some(target.read_back())
    }

    /// The image must be confined to its rect rather than stretched over the whole window — the
    /// difference between an outer-sized texture blitted 1:1 and a viewport-positioned one.
    ///
    /// Uses a target twice the image's size with no decorations, so an image that overflows has
    /// nowhere to hide: a decorated window sized exactly to its image is fully covered by
    /// content plus chrome, and would pass either way.
    #[test]
    fn image_does_not_spill_outside_its_rect() {
        let size = PhysicalSize::new(IMG_W * 2, IMG_H * 2);
        let content = Rect {
            x: 2,
            y: 3,
            width: IMG_W,
            height: IMG_H,
        };

        let Some(pixels) = render_image_only(size, content) else {
            eprintln!("no wgpu adapter; skipping");
            return;
        };

        let at = |x: u32, y: u32| {
            let i = ((y * size.width + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
        };

        for y in 0..size.height {
            for x in 0..size.width {
                let inside = x >= content.x
                    && x < content.x + content.width
                    && y >= content.y
                    && y < content.y + content.height;
                let [r, g, b, a] = at(x, y);

                if inside {
                    assert!(
                        r > 200 && g < 55 && b < 55 && a > 200,
                        "({x}, {y}) inside the rect was {:?}, expected red",
                        [r, g, b, a]
                    );
                } else {
                    assert!(
                        a < 55,
                        "({x}, {y}) outside the rect was {:?}, expected untouched",
                        [r, g, b, a]
                    );
                }
            }
        }
    }

    /// The image must land inside the content rect, not fill the whole window — this is what the
    /// switch from an outer-sized texture to a viewport-positioned one changed.
    #[test]
    fn image_fills_the_content_rect() {
        let Some(pixels) = render_decorated_image([255, 0, 0, 255]) else {
            eprintln!("no wgpu adapter; skipping");
            return;
        };

        for y in CONTENT.y..CONTENT.y + CONTENT.height {
            for x in CONTENT.x..CONTENT.x + CONTENT.width {
                let [r, g, b, a] = pixel_at(&pixels, x, y);
                assert!(
                    r > 200 && g < 55 && b < 55 && a > 200,
                    "content ({x}, {y}) was {:?}, expected red",
                    [r, g, b, a]
                );
            }
        }
    }

    #[test]
    fn decorations_are_composited_over_the_image() {
        let Some(pixels) = render_decorated_image([255, 0, 0, 255]) else {
            eprintln!("no wgpu adapter; skipping");
            return;
        };

        // Border: opaque black on every edge.
        for x in 0..OUTER_W {
            for y in [0, OUTER_H - 1] {
                let [r, g, b, a] = pixel_at(&pixels, x, y);
                assert!(
                    r < 55 && g < 55 && b < 55 && a > 200,
                    "border ({x}, {y}) was {:?}, expected black",
                    [r, g, b, a]
                );
            }
        }

        // The header band sits above the content and must not be showing the image through.
        for y in BORDER..BORDER + HEADER_PX {
            for x in BORDER..OUTER_W - BORDER {
                let [r, g, b, _] = pixel_at(&pixels, x, y);
                assert!(
                    !(r > 200 && g < 55 && b < 55),
                    "image leaked into the header at ({x}, {y})"
                );
            }
        }
    }
}
