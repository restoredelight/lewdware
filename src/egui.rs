use std::sync::{atomic::{AtomicBool, Ordering}, Arc};

use anyhow::Result;
use egui_wgpu::wgpu;
use winit::{event::WindowEvent, window::Window};

pub struct EguiWindow<'a> {
    context: egui::Context,
    viewport_id: egui::ViewportId,
    window: Arc<Window>,
    state: egui_winit::State,
    surface: wgpu::Surface<'a>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: egui_wgpu::Renderer,
    closed: Arc<AtomicBool>,
}

impl<'a> EguiWindow<'a> {
    pub fn new(wgpu_instance: &wgpu::Instance, window: Arc<Window>) -> Result<Self> {
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

        let surface = wgpu_instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(
            wgpu_instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
        )?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;

        
        let closed = Arc::new(AtomicBool::new(false));
        let closed_clone = closed.clone();

        device.on_uncaptured_error(Box::new(move |err| {
            eprintln!("wgpu error: {}", err);
            closed_clone.store(true, Ordering::Relaxed);
        }));

        let surface_caps = surface.get_capabilities(&adapter);
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
        surface.configure(&device, &config);
        let renderer = egui_wgpu::Renderer::new(&device, *surface_format, None, 1, true);

        context.request_repaint();

        Ok(Self {
            context,
            viewport_id,
            window,
            state,
            surface,
            adapter,
            device,
            queue,
            renderer,
            closed,
        })
    }

    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    pub fn handle_event(&mut self, event: &WindowEvent) -> bool {
        let response = self.state.on_window_event(&self.window, event);

        if let WindowEvent::Resized(size) = event {
            let surface_caps = self.surface.get_capabilities(&self.adapter);
            let surface_format = surface_caps
                .formats
                .iter()
                .find(|f| f.is_srgb())
                .unwrap_or(&surface_caps.formats[0]);
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: *surface_format,
                width: size.width,
                height: size.height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            self.surface.configure(&self.device, &config);

            return true;
        }

        response.repaint
    }

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

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        let mut render_pass = unsafe {
            // This is safe as long as we don't use the encoder again
            // before the render pass is complete.
            render_pass.forget_lifetime()
        };

        // Render the egui paint commands
        self.renderer
            .render(&mut render_pass, &paint_jobs, &screen_descriptor);

        drop(render_pass);

        // Submit the command buffer
        self.queue.submit(Some(encoder.finish()));

        output.present();

        Ok(())
    }
}
