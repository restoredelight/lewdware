use std::{
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, anyhow};
use egui::Color32;
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use egui_wgpu::{RendererOptions, wgpu};
use winit::{event::WindowEvent, window::Window};

/// A struct handling rendering onto a winit window using egui.
pub struct EguiWindow<'a> {
    context: egui::Context,
    window: Arc<Window>,
    state: egui_winit::State,
    surface: wgpu::Surface<'a>,
    adapter: Arc<wgpu::Adapter>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    renderer: egui_wgpu::Renderer,
    surface_config: wgpu::SurfaceConfiguration,
    repaint_requested: Arc<AtomicBool>,
}

/// A struct holding wgpu resources that should be shared between windows.
pub struct WgpuState {
    pub instance: wgpu::Instance,
    pub adapter: Arc<wgpu::Adapter>,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub error: Arc<AtomicBool>,
}

impl WgpuState {
    pub async fn new() -> Result<Self> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            })
            .await?;

        let device = Arc::new(device);

        let error = Arc::new(AtomicBool::new(false));
        let error_clone = error.clone();

        device.on_uncaptured_error(Arc::new(move |err| {
            // #[cfg(debug_assertions)]
            eprintln!("wgpu error: {}", err);

            error_clone.store(true, Ordering::Release);
        }));

        Ok(Self {
            instance,
            adapter: Arc::new(adapter),
            device,
            queue: Arc::new(queue),
            error,
        })
    }
}

impl<'a> EguiWindow<'a> {
    pub fn new(wgpu_state: &WgpuState, window: Arc<Window>) -> Result<Self> {
        let context = egui::Context::default();
        let viewport_id = egui::ViewportId::from_hash_of(window.id());
        let state = egui_winit::State::new(
            context.clone(),
            viewport_id,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        let surface = wgpu_state.instance.create_surface(window.clone())?;

        let surface_caps = surface.get_capabilities(&wgpu_state.adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .unwrap_or(&surface_caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *surface_format,
            width: window.inner_size().width,
            height: window.inner_size().height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&wgpu_state.device, &config);
        let renderer = egui_wgpu::Renderer::new(
            &wgpu_state.device,
            *surface_format,
            RendererOptions::default(),
        );

        context.request_repaint();

        let repaint_requested = Arc::new(AtomicBool::new(false));
        let repaint_requested_clone = repaint_requested.clone();

        context.set_request_repaint_callback(move |_| {
            repaint_requested_clone.store(true, Ordering::Release);
        });

        Ok(Self {
            context,
            window,
            state,
            surface,
            adapter: wgpu_state.adapter.clone(),
            device: wgpu_state.device.clone(),
            queue: wgpu_state.queue.clone(),
            renderer,
            surface_config: config,
            repaint_requested,
        })
    }

    /// Handle a window event. All window events should be passed into this function, aside from
    /// [WindowEvent::CloseRequested] and [WindowEvent::RedrawRequested].
    pub fn handle_event(&mut self, event: &WindowEvent) {
        let response = self.state.on_window_event(&self.window, event);

        if let WindowEvent::Resized(size) = event {
            self.surface_config.width = size.width;
            self.surface_config.height = size.height;
            self.surface.configure(&self.device, &self.surface_config);

            self.window.request_redraw();
            return;
        }

        if response.repaint {
            self.window.request_redraw();
        }
    }

    /// Redraw the egui window. This should be called whenever the window receives the
    /// [WindowEvent::RedrawRequested] event.
    ///
    /// * `run_ui`: This is where you should define the egui UI of the window.
    pub fn redraw(&mut self, run_ui: impl FnMut(&egui::Context)) -> Result<()> {
        let raw_input = self.state.take_egui_input(&self.window);

        let full_output = self.context.run(raw_input, run_ui);

        self.state
            .handle_platform_output(&self.window, full_output.platform_output);

        for (id, image_delta) in &full_output.textures_delta.set {
            self.renderer
                .update_texture(&self.device, &self.queue, *id, image_delta);
        }

        for &id in &full_output.textures_delta.free {
            self.renderer.free_texture(&id);
        }

        let pixels_per_point = egui_winit::pixels_per_point(&self.context, &self.window);

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [
                self.window.inner_size().width,
                self.window.inner_size().height,
            ],
            pixels_per_point,
        };

        let paint_jobs = self
            .context
            .tessellate(full_output.shapes, pixels_per_point);

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        self.renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );

        let output = self.surface.get_current_texture()?;

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        // This is safe as long as we don't use the encoder again
        // before the render pass is complete.
        let mut render_pass = render_pass.forget_lifetime();

        // Render the egui paint commands
        self.renderer
            .render(&mut render_pass, &paint_jobs, &screen_descriptor);

        drop(render_pass);

        // Submit the command buffer
        self.queue.submit(Some(encoder.finish()));

        output.present();

        if self.repaint_requested.swap(false, Ordering::AcqRel) {
            self.window.request_redraw();
        }

        Ok(())
    }
}

pub struct EguiCPUWindow {
    context: egui::Context,
    window: Arc<Window>,
    state: egui_winit::State,
    renderer: EguiSoftwareRender,
    _softbuffer_context: softbuffer::Context<Arc<Window>>,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
}

impl EguiCPUWindow {
    pub fn new(window: Arc<Window>) -> Result<Self> {
        let context = egui::Context::default();
        let viewport_id = egui::ViewportId::from_hash_of(window.id());
        let state = egui_winit::State::new(
            context.clone(),
            viewport_id,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        let renderer = EguiSoftwareRender::new(ColorFieldOrder::Bgra);

        let softbuffer_context =
            softbuffer::Context::new(window.clone()).map_err(|err| anyhow!("{}", err))?;
        let surface = softbuffer::Surface::new(&softbuffer_context, window.clone())
            .map_err(|err| anyhow!("{}", err))?;

        context.request_repaint();

        let window_clone = window.clone();

        context.set_request_repaint_callback(move |_| {
            window_clone.request_redraw();
        });

        let mut visuals = egui::Visuals::light();

        visuals.window_fill = Color32::TRANSPARENT;
        visuals.panel_fill = Color32::TRANSPARENT;

        context.set_visuals(visuals);

        Ok(Self {
            context,
            window,
            state,
            renderer,
            _softbuffer_context: softbuffer_context,
            surface,
        })
    }

    /// Handle a window event. All window events should be passed into this function, aside from
    /// [WindowEvent::CloseRequested] and [WindowEvent::RedrawRequested].
    pub fn handle_event(&mut self, event: &WindowEvent) {
        let response = self.state.on_window_event(&self.window, event);

        if response.repaint {
            self.window.request_redraw();
        }
    }

    /// Redraw the egui window. This should be called whenever the window receives the
    /// [WindowEvent::RedrawRequested] event.
    ///
    /// * `run_ui`: This is where you should define the egui UI of the window.
    pub fn redraw(&mut self, run_ui: impl FnMut(&egui::Context)) -> Result<()> {
        let size = self.window.inner_size();
        self.surface
            .resize(
                NonZeroU32::new(size.width).context("Window has 0 width")?,
                NonZeroU32::new(size.height).context("Window has 0 height")?,
            )
            .map_err(|err| anyhow!("{err}"))?;

        let raw_input = self.state.take_egui_input(&self.window);

        let full_output = self.context.run(raw_input, run_ui);

        self.state
            .handle_platform_output(&self.window, full_output.platform_output);

        let primitives = self
            .context
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        let mut buffer = self.surface.buffer_mut().map_err(|err| anyhow!("{err}"))?;
        buffer.fill(0);

        let buffer_ref = &mut BufferMutRef::new(
            bytemuck::cast_slice_mut(&mut buffer),
            size.width as usize,
            size.height as usize,
        );

        self.renderer.render(
            buffer_ref,
            &primitives,
            &full_output.textures_delta,
            full_output.pixels_per_point,
        );

        buffer.present().map_err(|err| anyhow!("{err}"))?;

        Ok(())
    }
}
