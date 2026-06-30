use std::sync::Arc;

use anyhow::Result;
use egui::Ui;
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use egui_wgpu::{RendererOptions, wgpu};
use winit::{dpi::PhysicalSize, event::WindowEvent, window::Window};

use crate::lua::Color;
use crate::wgpu::WgpuState;

/// GPU-accelerated egui renderer that does NOT own a wgpu surface. It renders into the caller's
/// existing `wgpu::RenderPass<'static>` (obtained via [`InnerWindow::draw_wgpu`]).
pub struct EguiGpuRenderer {
    context: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    // Intermediate RGBA render target sized to the egui content area (inner_size).
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    // Bind group to sample the intermediate texture (group 0 of the RGBA pipeline).
    pub bind_group: wgpu::BindGroup,
    // Opacity + premultiplied uniform buffer (group 1 of the RGBA pipeline).
    pub opacity_buffer: wgpu::Buffer,
    pub window_bind_group: wgpu::BindGroup,
    texture_size: PhysicalSize<u32>,
}

impl EguiGpuRenderer {
    pub fn new(
        wgpu_state: &WgpuState,
        window: &Arc<Window>,
        inner_size: PhysicalSize<u32>,
        opacity: f32,
        premultiplied_alpha: bool,
        force_opaque: bool,
        background_color: Option<Color>,
        font_definitions: Option<egui::FontDefinitions>,
    ) -> Result<Self> {
        let context = egui::Context::default();
        let viewport_id = egui::ViewportId::from_hash_of(window.id());
        let state = egui_winit::State::new(
            context.clone(),
            viewport_id,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        let renderer = egui_wgpu::Renderer::new(
            &wgpu_state.device,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            RendererOptions::default(),
        );

        if let Some(font_definitions) = font_definitions {
            context.set_fonts(font_definitions);
        }

        context.request_repaint();

        let window_clone = window.clone();
        context.set_request_repaint_callback(move |_| {
            window_clone.request_redraw();
        });

        let mut visuals = egui::Visuals::light();
        if let Some(c) = background_color {
            let color = egui::Color32::from_rgba_unmultiplied(
                (c.r * 255.0).round() as u8,
                (c.g * 255.0).round() as u8,
                (c.b * 255.0).round() as u8,
                (c.a * 255.0).round() as u8,
            );
            visuals.window_fill = color;
            visuals.panel_fill = color;
        }
        context.set_visuals(visuals);

        let (texture, texture_view, bind_group) =
            create_egui_texture(wgpu_state, inner_size.width, inner_size.height);

        use wgpu::util::DeviceExt;

        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct WindowUniform {
            opacity: f32,
            premultiplied: u32,
            force_opaque: u32,
        }

        let opacity_buffer =
            wgpu_state
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Egui Opacity Buffer"),
                    contents: bytemuck::bytes_of(&WindowUniform {
                        opacity,
                        premultiplied: premultiplied_alpha as u32,
                        force_opaque: force_opaque as u32,
                    }),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });

        let window_bind_group = wgpu_state
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Egui Window Bind Group"),
                layout: &wgpu_state.window_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: opacity_buffer.as_entire_binding(),
                }],
            });

        Ok(Self {
            context,
            state,
            renderer,
            texture,
            texture_view,
            bind_group,
            opacity_buffer,
            window_bind_group,
            texture_size: inner_size,
        })
    }

    /// Update the per-window opacity uniform (only writes the opacity field, not premultiplied).
    pub fn set_opacity(&self, queue: &wgpu::Queue, opacity: f32) {
        queue.write_buffer(&self.opacity_buffer, 0, bytemuck::cast_slice(&[opacity]));
    }

    /// Handle a window event. The caller is responsible for translating cursor positions to the
    /// egui content area before calling this (subtract the inner offset from cursor events).
    pub fn handle_event(&mut self, window: &Arc<Window>, event: &WindowEvent) {
        let response = self.state.on_window_event(window, event);
        if response.repaint {
            window.request_redraw();
        }
    }

    /// Render the egui UI into the internal intermediate texture.
    ///
    /// This submits its own command buffer. After this returns, `self.bind_group` holds the
    /// freshly rendered frame and can be blitted into the window surface via `draw_wgpu`.
    ///
    /// * `inner_size` — physical pixel size of the egui content area (excluding decorations).
    pub fn render_to_texture(
        &mut self,
        wgpu_state: &WgpuState,
        window: &Arc<Window>,
        inner_size: PhysicalSize<u32>,
        run_ui: impl FnMut(&mut Ui),
    ) -> Result<()> {
        // Recreate texture if the content area size changed.
        if self.texture_size != inner_size {
            let (texture, view, bind_group) =
                create_egui_texture(wgpu_state, inner_size.width, inner_size.height);
            self.texture = texture;
            self.texture_view = view;
            self.bind_group = bind_group;
            self.texture_size = inner_size;
        }

        let mut raw_input = self.state.take_egui_input(window);

        let pixels_per_point = egui_winit::pixels_per_point(&self.context, window);
        raw_input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(
                inner_size.width as f32 / pixels_per_point,
                inner_size.height as f32 / pixels_per_point,
            ),
        ));

        let full_output = self.context.run_ui(raw_input, run_ui);

        self.state
            .handle_platform_output(window, full_output.platform_output);

        for (id, image_delta) in &full_output.textures_delta.set {
            self.renderer
                .update_texture(&wgpu_state.device, &wgpu_state.queue, *id, image_delta);
        }
        for &id in &full_output.textures_delta.free {
            self.renderer.free_texture(&id);
        }

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [inner_size.width, inner_size.height],
            pixels_per_point,
        };

        let paint_jobs = self
            .context
            .tessellate(full_output.shapes, pixels_per_point);

        let mut encoder =
            wgpu_state
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("egui_texture_encoder"),
                });

        let mut extra_cbs = self.renderer.update_buffers(
            &wgpu_state.device,
            &wgpu_state.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );

        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui_texture_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // SAFETY: encoder is not used again until after rpass is dropped below.
            let mut rpass = rpass.forget_lifetime();
            self.renderer
                .render(&mut rpass, &paint_jobs, &screen_descriptor);
        }

        extra_cbs.push(encoder.finish());
        wgpu_state.queue.submit(extra_cbs);

        Ok(())
    }
}

fn create_egui_texture(
    wgpu_state: &WgpuState,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
    let texture = wgpu_state.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Egui Intermediate Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let bind_group = wgpu_state
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Egui Texture Bind Group"),
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

    (texture, view, bind_group)
}

pub struct EguiCPUWindow {
    context: egui::Context,
    window: Arc<Window>,
    state: egui_winit::State,
    renderer: EguiSoftwareRender,
}

impl EguiCPUWindow {
    pub fn new(
        window: Arc<Window>,
        background_color: Option<Color>,
        font_definitions: Option<egui::FontDefinitions>,
    ) -> Result<Self> {
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

        if let Some(font_definitions) = font_definitions {
            context.set_fonts(font_definitions);
        }

        context.request_repaint();

        let window_clone = window.clone();
        context.set_request_repaint_callback(move |_| {
            window_clone.request_redraw();
        });

        let mut visuals = egui::Visuals::light();
        if let Some(c) = background_color {
            let color = egui::Color32::from_rgba_unmultiplied(
                (c.r * 255.0).round() as u8,
                (c.g * 255.0).round() as u8,
                (c.b * 255.0).round() as u8,
                (c.a * 255.0).round() as u8,
            );
            visuals.window_fill = color;
            visuals.panel_fill = color;
        }
        context.set_visuals(visuals);

        Ok(Self {
            context,
            window,
            state,
            renderer,
        })
    }

    /// Handle a window event. Cursor positions should already be translated to the egui content
    /// area before calling this.
    pub fn handle_event(&mut self, event: &WindowEvent) {
        let response = self.state.on_window_event(&self.window, event);
        if response.repaint {
            self.window.request_redraw();
        }
    }

    pub fn redraw(
        &mut self,
        buffer: &mut BufferMutRef,
        run_ui: impl FnMut(&mut egui::Ui),
    ) -> Result<()> {
        let raw_input = self.state.take_egui_input(&self.window);

        let full_output = self.context.run_ui(raw_input, run_ui);

        self.state
            .handle_platform_output(&self.window, full_output.platform_output);

        let primitives = self
            .context
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        self.renderer.render(
            buffer,
            &primitives,
            &full_output.textures_delta,
            full_output.pixels_per_point,
        );

        Ok(())
    }
}
