use std::collections::HashMap;

use anyhow::Result;
use winit::event::WindowEvent;

use crate::lua::{DialogElement, DialogElementUpdate, TextStyle};
use crate::media::ImageData;
use crate::video::VideoDecoder;
use crate::window::layer::{
    CpuFrame, EguiLayer, GpuFrame, ImageLayer, Layer, LayerInit, LayerStatus, Rect, VideoLayer,
};
use crate::window::state::WindowState;
use crate::window::target::RenderTarget;

/// A live popup window: its content, where that content is drawn, and everything about the
/// window that isn't either of those.
pub struct Popup {
    // Declared first so it drops first: egui holds an `Arc<Window>` clone that must be released
    // before the state and target let go of theirs.
    layer: Layer,
    pub state: WindowState,
    pub target: RenderTarget,
}

/// What one [`Popup::render`] produced.
#[derive(Default)]
pub struct RenderOutcome {
    /// The wgpu submission, when a GPU frame was actually submitted. Callers that need the frame
    /// on screen before moving the window (see [`RenderTarget::gpu_sync`]) wait on this.
    pub submission: Option<wgpu::SubmissionIndex>,
    /// The content finished (a video ran to its end) and the window should be closed.
    pub finished: bool,
}

impl Popup {
    pub fn new_image(state: WindowState, target: RenderTarget, image: ImageData) -> Result<Self> {
        let layer = Layer::Image(ImageLayer::new(
            &LayerInit::from_window(&state, &target),
            image,
        )?);
        Ok(Self {
            layer,
            state,
            target,
        })
    }

    pub fn new_video(
        state: WindowState,
        target: RenderTarget,
        decoder: VideoDecoder,
    ) -> Result<Self> {
        let layer = Layer::Video(VideoLayer::new(
            &LayerInit::from_window(&state, &target),
            decoder,
        )?);
        state.request_redraw();
        Ok(Self {
            layer,
            state,
            target,
        })
    }

    pub fn new_dialog<I>(
        state: WindowState,
        target: RenderTarget,
        elements: Vec<DialogElement<I>>,
        resolve_image: impl FnMut(I) -> Result<ImageData>,
    ) -> Result<Self> {
        let layer = Layer::Egui(EguiLayer::new_dialog(
            &state,
            &target,
            elements,
            resolve_image,
        )?);
        Ok(Self {
            layer,
            state,
            target,
        })
    }

    pub fn new_text(
        state: WindowState,
        target: RenderTarget,
        text: String,
        style: TextStyle,
    ) -> Result<Self> {
        let layer = Layer::Egui(EguiLayer::new_text(&state, &target, text, style)?);
        Ok(Self {
            layer,
            state,
            target,
        })
    }

    fn content_rect(&self) -> Rect {
        let (x, y) = self.state.inner_offset();
        let size = self.state.inner_size();
        Rect {
            x,
            y,
            width: size.width,
            height: size.height,
        }
    }

    /// Draw one frame: prepare the content layer and the decorations, then — if either has
    /// something new to show — submit a frame with the content underneath the decorations.
    pub fn render(&mut self) -> Result<RenderOutcome> {
        // A wgpu device loss silently swaps the surface for a software one. The layer's GPU
        // resources are meaningless against the new surface, so rebuild them before drawing;
        // without this the window renders nothing ever again.
        if self.target.start_render()? {
            tracing::info!("render target changed backend; rebuilding layers");
            self.layer.rebuild(&self.state, &self.target)?;
            self.state.attach_decorations(&self.target);
        }

        let content = self.content_rect();
        let opacity = self.state.opacity;

        if self.target.is_gpu() {
            self.render_gpu(content, opacity)
        } else {
            self.render_cpu(content)
        }
    }

    fn render_gpu(&mut self, content: Rect, opacity: f32) -> Result<RenderOutcome> {
        let wgpu = self.target.wgpu_state().clone();
        let pipeline = self.target.rgba_pipeline();

        let frame = GpuFrame {
            wgpu: &wgpu,
            pipeline: &pipeline,
            content,
            opacity,
        };

        let status = self.layer.prepare_gpu(&self.state, &frame)?;
        if status == LayerStatus::Finished {
            return Ok(RenderOutcome {
                submission: None,
                finished: true,
            });
        }

        // A header change must force a frame too: otherwise it would sit in the overlay texture
        // unseen until the content next happened to change.
        let decorations_changed = self
            .state
            .decorations_mut()
            .prepare_gpu(&wgpu.queue, opacity);

        if status != LayerStatus::Draw && !decorations_changed {
            return Ok(RenderOutcome::default());
        }

        let layer = &self.layer;
        let decorations = self.state.decorations();
        let submission = self.target.draw_wgpu(|rpass| {
            layer.draw_gpu(rpass, &frame);
            decorations.draw_gpu(rpass, &pipeline);
        })?;

        self.layer.flush_events(&self.state);

        Ok(RenderOutcome {
            submission,
            finished: false,
        })
    }

    fn render_cpu(&mut self, content: Rect) -> Result<RenderOutcome> {
        let frame = CpuFrame { content };

        let status = self.layer.prepare_cpu(&frame)?;
        if status == LayerStatus::Finished {
            return Ok(RenderOutcome {
                submission: None,
                finished: true,
            });
        }

        // Unlike the GPU path there is no persistent overlay to compare against: a softbuffer
        // frame is rebuilt from scratch every time, so draw whenever we are asked to.
        let layer = &mut self.layer;
        let decorations = self.state.decorations_mut();
        self.target.draw_softbuffer(|buffer| {
            layer.draw_cpu(buffer, &frame);
            decorations.draw_cpu(buffer);
        })?;

        self.layer.flush_events(&self.state);

        Ok(RenderOutcome::default())
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        self.layer.handle_event(&self.state, event);
    }

    /// Consume this popup, dropping its content resources and returning the window's two halves.
    pub fn into_parts(self) -> (WindowState, RenderTarget) {
        (self.state, self.target)
    }

    // ── Content-specific operations, routed from Lua ──────────────────────────
    //
    // Each returns the "not applicable" value when the popup's content is a different kind, which
    // is what the caller already does for a request aimed at the wrong window type.

    pub fn set_text(&mut self, text: String) -> bool {
        let Layer::Egui(layer) = &mut self.layer else {
            return false;
        };

        if layer.set_text(text) {
            self.state.request_redraw();
            true
        } else {
            false
        }
    }

    pub fn update_element(&mut self, id: &str, props: DialogElementUpdate) -> bool {
        let Layer::Egui(layer) = &mut self.layer else {
            return false;
        };

        if layer.update_element(id, props) {
            self.state.request_redraw();
            true
        } else {
            false
        }
    }

    pub fn value(&self, id: &str) -> Option<String> {
        match &self.layer {
            Layer::Egui(layer) => layer.value(id),
            _ => None,
        }
    }

    pub fn values(&self) -> HashMap<String, String> {
        match &self.layer {
            Layer::Egui(layer) => layer.values(),
            _ => HashMap::new(),
        }
    }

    pub fn is_video(&self) -> bool {
        matches!(self.layer, Layer::Video(_))
    }

    pub fn pause(&mut self) -> bool {
        let Layer::Video(layer) = &mut self.layer else {
            return false;
        };
        layer.pause();
        true
    }

    pub fn play(&mut self) -> bool {
        let Layer::Video(layer) = &mut self.layer else {
            return false;
        };
        layer.play();
        true
    }

    pub fn set_volume(&mut self, volume: f32) -> bool {
        let Layer::Video(layer) = &mut self.layer else {
            return false;
        };
        layer.set_volume(volume);
        true
    }

    pub fn start_volume_fade(&mut self, id: u64, opts: Option<crate::lua::VolumeFadeOpts>) -> bool {
        let Layer::Video(layer) = &mut self.layer else {
            return false;
        };
        layer.start_volume_fade(id, opts);
        true
    }

    pub fn update_volume_fade(&mut self) -> Option<u64> {
        let Layer::Video(layer) = &mut self.layer else {
            return None;
        };
        layer.update_volume_fade()
    }

    pub fn is_fading_volume(&self) -> bool {
        let Layer::Video(layer) = &self.layer else {
            return false;
        };
        layer.is_fading_volume()
    }

    pub fn set_loop(&self, loop_video: bool) -> bool {
        let Layer::Video(layer) = &self.layer else {
            return false;
        };
        layer.set_loop(loop_video);
        true
    }
}
