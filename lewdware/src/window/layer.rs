use anyhow::Result;
use winit::event::WindowEvent;

use crate::wgpu::WgpuState;
use crate::window::state::WindowState;
use crate::window::surface::Buffer;
use crate::window::target::RenderTarget;

mod egui;
mod egui_renderer;
mod image;
mod video;

pub use egui::EguiLayer;
pub use image::ImageLayer;
pub use video::VideoLayer;

/// A rectangle within the outer window, in physical pixels.
#[derive(Clone, Copy)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn set_viewport(&self, rpass: &mut wgpu::RenderPass<'static>) {
        rpass.set_viewport(
            self.x as f32,
            self.y as f32,
            self.width as f32,
            self.height as f32,
            0.0,
            1.0,
        );
    }
}

/// Everything a layer needs in order to build, or rebuild, its backend resources.
///
/// Layers take this rather than `&WindowState, &RenderTarget` so their dependencies are visible
/// in the signature — and so a layer can be built without a live window, which is what makes
/// them testable at all.
pub struct LayerInit<'a> {
    pub backend: BackendInit<'a>,
    /// The window's content area, in outer-window physical pixels.
    pub content: Rect,
    pub opacity: f32,
}

pub enum BackendInit<'a> {
    Gpu {
        wgpu: &'a WgpuState,
        /// The format layers must build their pipelines against.
        format: wgpu::TextureFormat,
        premultiplied_alpha: bool,
        force_opaque: bool,
    },
    Cpu,
}

impl<'a> LayerInit<'a> {
    pub fn from_window(state: &WindowState, target: &'a RenderTarget) -> Self {
        let (x, y) = state.inner_offset();
        let size = state.inner_size();

        let backend = if target.is_gpu() {
            BackendInit::Gpu {
                wgpu: target.wgpu_state(),
                format: target.surface_format().unwrap(),
                premultiplied_alpha: target.premultiplied_alpha(),
                force_opaque: target.force_opaque(),
            }
        } else {
            BackendInit::Cpu
        };

        Self {
            backend,
            content: Rect {
                x,
                y,
                width: size.width,
                height: size.height,
            },
            opacity: state.opacity,
        }
    }
}

/// What a layer needs to know about the GPU frame being drawn.
pub struct GpuFrame<'a> {
    pub wgpu: &'a WgpuState,
    /// The shared RGBA blit pipeline for this surface's format.
    pub pipeline: &'a wgpu::RenderPipeline,
    /// The window's content area — where a layer draws unless it says otherwise.
    pub content: Rect,
    pub opacity: f32,
}

/// What a layer needs to know about the CPU frame being drawn.
///
/// No opacity: the software path composites straight into an opaque window buffer, so per-window
/// opacity and fades have never had a visual effect there. Left that way deliberately —
/// softbuffer's support for translucent surfaces is patchy across platforms. Worth revisiting
/// once layers can be composited, since that gives somewhere to blend before presenting.
///
/// Note that `Window:fade()` still runs and still fires `FadeFinish` back to Lua on CPU
/// windows; only the visual effect is missing.
pub struct CpuFrame {
    pub content: Rect,
}

/// What a layer reported during `prepare`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LayerStatus {
    /// Nothing changed; this layer does not need the frame drawn on its account.
    Idle,
    /// This layer has new content and needs the frame drawn.
    Draw,
    /// This layer has run to completion and the window should close (a video ending).
    Finished,
}

/// One piece of drawable content in a window.
///
/// A window currently holds exactly one, composited under its decorations. The dispatch here is
/// what a future multi-layer window would iterate over: `prepare` every layer, draw the frame if
/// any of them asked for it, then draw each in order.
pub enum Layer {
    Image(ImageLayer),
    Video(VideoLayer),
    Egui(EguiLayer),
}

impl Layer {
    /// Rebuild backend-specific resources after the render target switched backend (a wgpu
    /// device loss falling back to software). Backend-independent state — decoded pixels, the
    /// video decoder, dialog contents — is preserved.
    pub fn rebuild(&mut self, state: &WindowState, target: &RenderTarget) -> Result<()> {
        let init = LayerInit::from_window(state, target);

        match self {
            Self::Image(layer) => layer.rebuild(&init),
            Self::Video(layer) => layer.rebuild(&init),
            // egui needs the live window handle for `egui_winit`, so it takes both.
            Self::Egui(layer) => layer.rebuild(state, target),
        }
    }

    pub fn prepare_gpu(
        &mut self,
        state: &WindowState,
        frame: &GpuFrame<'_>,
    ) -> Result<LayerStatus> {
        match self {
            Self::Image(layer) => layer.prepare_gpu(frame),
            Self::Video(layer) => layer.prepare_gpu(frame),
            Self::Egui(layer) => layer.prepare_gpu(state, frame),
        }
    }

    pub fn draw_gpu(&self, rpass: &mut wgpu::RenderPass<'static>, frame: &GpuFrame<'_>) {
        match self {
            Self::Image(layer) => layer.draw_gpu(rpass, frame),
            Self::Video(layer) => layer.draw_gpu(rpass, frame),
            Self::Egui(layer) => layer.draw_gpu(rpass, frame),
        }
    }

    pub fn prepare_cpu(&mut self, frame: &CpuFrame) -> Result<LayerStatus> {
        match self {
            Self::Image(layer) => layer.prepare_cpu(frame),
            Self::Video(layer) => layer.prepare_cpu(frame),
            Self::Egui(layer) => layer.prepare_cpu(frame),
        }
    }

    pub fn draw_cpu(&mut self, buffer: &mut Buffer, frame: &CpuFrame) {
        match self {
            Self::Image(layer) => layer.draw_cpu(buffer, frame),
            Self::Video(layer) => layer.draw_cpu(buffer, frame),
            Self::Egui(layer) => layer.draw_cpu(buffer, frame),
        }
    }

    /// Send anything the layer recorded while painting. Called once the frame is complete, so
    /// the draw closure never has to borrow the window state.
    pub fn flush_events(&mut self, state: &WindowState) {
        if let Self::Egui(layer) = self {
            layer.flush_events(state);
        }
    }

    pub fn handle_event(&mut self, state: &WindowState, event: &WindowEvent) {
        if let Self::Egui(layer) = self {
            layer.handle_event(state, event);
        }
    }
}
