use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, bail};
use egui::Ui;
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

    // Shared quad renderer resources (RGBA)
    pub sampler: wgpu::Sampler,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub shader: wgpu::ShaderModule,
    pub pipeline_layout: wgpu::PipelineLayout,
    pub pipelines: std::sync::Mutex<std::collections::HashMap<wgpu::TextureFormat, Arc<wgpu::RenderPipeline>>>,

    // YUV420P video renderer resources (4 bindings: Y, Cb, Cr, sampler)
    pub yuv_bind_group_layout: wgpu::BindGroupLayout,
    pub yuv_shader: wgpu::ShaderModule,
    pub yuv_pipeline_layout: wgpu::PipelineLayout,
    // Key: (surface format, full_range)
    pub yuv_pipelines: std::sync::Mutex<std::collections::HashMap<(wgpu::TextureFormat, bool), Arc<wgpu::RenderPipeline>>>,

    // NV12 video renderer resources (3 bindings: Y, UV, sampler)
    pub nv12_bind_group_layout: wgpu::BindGroupLayout,
    pub nv12_shader: wgpu::ShaderModule,
    pub nv12_pipeline_layout: wgpu::PipelineLayout,
    // Key: (surface format, full_range)
    pub nv12_pipelines: std::sync::Mutex<std::collections::HashMap<(wgpu::TextureFormat, bool), Arc<wgpu::RenderPipeline>>>,
}

impl WgpuState {
    pub async fn new() -> Result<Self> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await?;
        // On Linux, inject VK_EXT_image_drm_format_modifier when available so that
        // drm_import.rs can import tiled (e.g. Intel Y-tiled) DMA-BUF surfaces zero-copy.
        #[cfg(target_os = "linux")]
        let (device, queue) = {
            use wgpu::hal::vulkan;

            let has_drm_mod_ext = unsafe { adapter.as_hal::<vulkan::Api>() }
                .map(|hal| {
                    hal.physical_device_capabilities()
                        .supports_extension(ash::vk::EXT_IMAGE_DRM_FORMAT_MODIFIER_NAME)
                })
                .unwrap_or(false);

            if has_drm_mod_ext {
                let open_device = {
                    let hal = unsafe { adapter.as_hal::<vulkan::Api>() }
                        .expect("Vulkan adapter on Linux");
                    let callback: Box<vulkan::CreateDeviceCallback<'static>> =
                        Box::new(|mut args| {
                            args.extensions.push(ash::vk::EXT_IMAGE_DRM_FORMAT_MODIFIER_NAME);
                        });
                    unsafe {
                        hal.open_with_callback(
                            wgpu::Features::empty(),
                            &wgpu::Limits::default(),
                            &wgpu::MemoryHints::MemoryUsage,
                            Some(callback),
                        )
                    }
                    .map_err(|e| anyhow::anyhow!("Vulkan device creation failed: {e:?}"))?
                };
                eprintln!("[wgpu] VK_EXT_image_drm_format_modifier enabled");
                unsafe {
                    adapter.create_device_from_hal::<vulkan::Api>(
                        open_device,
                        &wgpu::DeviceDescriptor {
                            memory_hints: wgpu::MemoryHints::MemoryUsage,
                            ..Default::default()
                        },
                    )
                }
                .map_err(|e| anyhow::anyhow!("{e}"))?
            } else {
                adapter
                    .request_device(&wgpu::DeviceDescriptor {
                        memory_hints: wgpu::MemoryHints::MemoryUsage,
                        ..Default::default()
                    })
                    .await?
            }
        };
        #[cfg(not(target_os = "linux"))]
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: if cfg!(target_os = "windows") {
                    wgpu::Features::TEXTURE_FORMAT_NV12
                } else {
                    wgpu::Features::empty()
                },
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shared Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shared Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shared Quad Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shader.wgsl"
            ))),
        });

        let pipelines = std::sync::Mutex::new(std::collections::HashMap::new());

        // YUV resources: 3 R8Unorm plane textures + 1 sampler
        let yuv_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("YUV Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let yuv_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("YUV Pipeline Layout"),
            bind_group_layouts: &[Some(&yuv_bind_group_layout)],
            immediate_size: 0,
        });

        let yuv_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("YUV Quad Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "video_shader.wgsl"
            ))),
        });

        let yuv_pipelines = std::sync::Mutex::new(std::collections::HashMap::new());

        // NV12 resources: Y (R8Unorm) + UV (Rg8Unorm) + sampler
        let nv12_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("NV12 Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let nv12_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("NV12 Pipeline Layout"),
            bind_group_layouts: &[Some(&nv12_bind_group_layout)],
            immediate_size: 0,
        });

        let nv12_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("NV12 Quad Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "nv12_shader.wgsl"
            ))),
        });

        let nv12_pipelines = std::sync::Mutex::new(std::collections::HashMap::new());

        Ok(Self {
            instance,
            adapter: Arc::new(adapter),
            device,
            queue: Arc::new(queue),
            error,
            sampler,
            bind_group_layout,
            shader,
            pipeline_layout,
            pipelines,
            yuv_bind_group_layout,
            yuv_shader,
            yuv_pipeline_layout,
            yuv_pipelines,
            nv12_bind_group_layout,
            nv12_shader,
            nv12_pipeline_layout,
            nv12_pipelines,
        })
    }

    pub fn get_yuv_pipeline(&self, format: wgpu::TextureFormat, full_range: bool) -> Arc<wgpu::RenderPipeline> {
        let mut pipelines = self.yuv_pipelines.lock().unwrap();
        pipelines
            .entry((format, full_range))
            .or_insert_with(|| {
                let entry_point = if full_range { "fs_yuv_full" } else { "fs_yuv_limited" };
                let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("YUV Render Pipeline"),
                    layout: Some(&self.yuv_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.yuv_shader,
                        entry_point: Some("vs_main"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.yuv_shader,
                        entry_point: Some(entry_point),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: None,
                        unclipped_depth: false,
                        polygon_mode: wgpu::PolygonMode::Fill,
                        conservative: false,
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                });
                Arc::new(pipeline)
            })
            .clone()
    }

    pub fn get_nv12_pipeline(&self, format: wgpu::TextureFormat, full_range: bool) -> Arc<wgpu::RenderPipeline> {
        let mut pipelines = self.nv12_pipelines.lock().unwrap();
        pipelines
            .entry((format, full_range))
            .or_insert_with(|| {
                let entry_point = if full_range { "fs_nv12_full" } else { "fs_nv12_limited" };
                let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("NV12 Render Pipeline"),
                    layout: Some(&self.nv12_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.nv12_shader,
                        entry_point: Some("vs_main"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.nv12_shader,
                        entry_point: Some(entry_point),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: None,
                        unclipped_depth: false,
                        polygon_mode: wgpu::PolygonMode::Fill,
                        conservative: false,
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                });
                Arc::new(pipeline)
            })
            .clone()
    }

    pub fn get_pipeline(&self, format: wgpu::TextureFormat) -> Arc<wgpu::RenderPipeline> {
        let mut pipelines = self.pipelines.lock().unwrap();
        pipelines
            .entry(format)
            .or_insert_with(|| {
                let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Shared Quad Render Pipeline"),
                    layout: Some(&self.pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.shader,
                        entry_point: Some("vs_main"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.shader,
                        entry_point: Some("fs_main"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: None,
                        unclipped_depth: false,
                        polygon_mode: wgpu::PolygonMode::Fill,
                        conservative: false,
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                });
                Arc::new(pipeline)
            })
            .clone()
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
    pub fn redraw(&mut self, run_ui: impl FnMut(&mut Ui)) -> Result<()> {
        let raw_input = self.state.take_egui_input(&self.window);

        let full_output = self.context.run_ui(raw_input, run_ui);

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

        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(surface_texture) => surface_texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => surface_texture,
            wgpu::CurrentSurfaceTexture::Timeout => return Ok(()),
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(());
            }
            _ => return Ok(()),
        };

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
            multiview_mask: None,
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
}

impl EguiCPUWindow {
    pub fn new(window: Arc<Window>, gpu: bool, transparent: bool) -> Result<Self> {
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

        let color_order = if gpu {
            ColorFieldOrder::Rgba
        } else {
            ColorFieldOrder::Bgra
        };
        let renderer = EguiSoftwareRender::new(color_order);

        context.request_repaint();

        let window_clone = window.clone();

        context.set_request_repaint_callback(move |_| {
            window_clone.request_redraw();
        });

        let mut visuals = egui::Visuals::light();
        if transparent {
            visuals.window_fill = egui::Color32::TRANSPARENT;
            visuals.panel_fill = egui::Color32::TRANSPARENT;
        }

        context.set_visuals(visuals);

        Ok(Self {
            context,
            window,
            state,
            renderer,
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
    pub fn redraw(
        &mut self,
        buffer: &mut BufferMutRef,
        run_ui: impl FnMut(&egui::Context),
    ) -> Result<()> {
        let raw_input = self.state.take_egui_input(&self.window);

        let full_output = self.context.run(raw_input, run_ui);

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
