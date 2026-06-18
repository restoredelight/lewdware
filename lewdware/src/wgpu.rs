use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use wgpu::InstanceDescriptor;
use winit::event_loop::OwnedDisplayHandle;

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
    pub window_bind_group_layout: wgpu::BindGroupLayout,
    pub shader: wgpu::ShaderModule,
    pub pipeline_layout: wgpu::PipelineLayout,
    pub pipelines:
        std::sync::Mutex<std::collections::HashMap<wgpu::TextureFormat, Arc<wgpu::RenderPipeline>>>,

    // YUV420P video renderer resources (4 bindings: Y, Cb, Cr, sampler)
    pub yuv_bind_group_layout: wgpu::BindGroupLayout,
    pub yuv_shader: wgpu::ShaderModule,
    pub yuv_pipeline_layout: wgpu::PipelineLayout,
    // Key: (surface format, full_range)
    pub yuv_pipelines: std::sync::Mutex<
        std::collections::HashMap<(wgpu::TextureFormat, bool), Arc<wgpu::RenderPipeline>>,
    >,
    // Packed-alpha YUV420p (always full-range). Key: surface format.
    pub yuv_packed_alpha_pipelines:
        std::sync::Mutex<std::collections::HashMap<wgpu::TextureFormat, Arc<wgpu::RenderPipeline>>>,

    // NV12 video renderer resources (3 bindings: Y, UV, sampler)
    pub nv12_bind_group_layout: wgpu::BindGroupLayout,
    pub nv12_shader: wgpu::ShaderModule,
    pub nv12_pipeline_layout: wgpu::PipelineLayout,
    // Key: (surface format, full_range)
    pub nv12_pipelines: std::sync::Mutex<
        std::collections::HashMap<(wgpu::TextureFormat, bool), Arc<wgpu::RenderPipeline>>,
    >,
    // Packed-alpha NV12: top half = color, bottom half = alpha (always full-range). Key: surface format.
    pub nv12_packed_alpha_pipelines:
        std::sync::Mutex<std::collections::HashMap<wgpu::TextureFormat, Arc<wgpu::RenderPipeline>>>,
}

impl WgpuState {
    pub async fn new(display_handle: OwnedDisplayHandle) -> Result<Self> {
        #[allow(unused_mut)]
        let mut instance_descriptor =
            InstanceDescriptor::new_with_display_handle(Box::new(display_handle));

        #[cfg(target_os = "windows")]
        {
            // A DX12 swapchain made directly from the window's HWND (the default) never
            // reports a PreMultiplied/PostMultiplied composite alpha mode, so transparent
            // windows always render opaque. Routing through a DirectComposition visual
            // instead (which wgpu sets up internally from the same HWND) is the only way to
            // get real per-pixel window transparency on Windows. Applies to every window in
            // this instance, not just transparent ones, and loses RenderDoc support.
            instance_descriptor.backend_options.dx12.presentation_system =
                wgpu::Dx12SwapchainKind::DxgiFromVisual;
        }

        let instance = wgpu::Instance::new(instance_descriptor);

        #[allow(unused_mut)]
        let mut chosen_adapter = None;

        // We need DX12 for zero-copy hardware decoding on Windows
        #[cfg(target_os = "windows")]
        {
            for adapter in instance.enumerate_adapters(wgpu::Backends::PRIMARY).await {
                if adapter.get_info().backend == wgpu::Backend::Dx12 {
                    chosen_adapter = Some(adapter);
                    break;
                }
            }
        }

        let adapter = match chosen_adapter {
            Some(a) => a,
            None => {
                instance
                    .request_adapter(&wgpu::RequestAdapterOptions::default())
                    .await?
            }
        };

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
                    let callback: Box<vulkan::CreateDeviceCallback<'static>> = Box::new(|args| {
                        args.extensions
                            .push(ash::vk::EXT_IMAGE_DRM_FORMAT_MODIFIER_NAME);
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
                tracing::info!("[wgpu] VK_EXT_image_drm_format_modifier enabled");
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
                required_features: if cfg!(target_os = "windows")
                    && adapter
                        .features()
                        .contains(wgpu::Features::TEXTURE_FORMAT_NV12)
                {
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
            tracing::error!("wgpu error: {}", err);

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

        let window_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Window Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shared Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(&window_bind_group_layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shared Quad Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/rgba.wgsl"
            ))),
        });

        let pipelines = std::sync::Mutex::new(std::collections::HashMap::new());

        // YUV resources: 3 R8Unorm plane textures + 1 sampler
        let yuv_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            bind_group_layouts: &[
                Some(&yuv_bind_group_layout),
                Some(&window_bind_group_layout),
            ],
            immediate_size: 0,
        });

        let yuv_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("YUV Quad Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/yuv.wgsl"
            ))),
        });

        let yuv_pipelines = std::sync::Mutex::new(std::collections::HashMap::new());
        let yuv_packed_alpha_pipelines = std::sync::Mutex::new(std::collections::HashMap::new());

        // NV12 resources: Y (R8Unorm) + UV (Rg8Unorm) + sampler
        let nv12_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            bind_group_layouts: &[
                Some(&nv12_bind_group_layout),
                Some(&window_bind_group_layout),
            ],
            immediate_size: 0,
        });

        let nv12_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("NV12 Quad Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/nv12.wgsl"
            ))),
        });

        let nv12_pipelines = std::sync::Mutex::new(std::collections::HashMap::new());
        let nv12_packed_alpha_pipelines = std::sync::Mutex::new(std::collections::HashMap::new());

        Ok(Self {
            instance,
            adapter: Arc::new(adapter),
            device,
            queue: Arc::new(queue),
            error,
            sampler,
            bind_group_layout,
            window_bind_group_layout,
            shader,
            pipeline_layout,
            pipelines,
            yuv_bind_group_layout,
            yuv_shader,
            yuv_pipeline_layout,
            yuv_pipelines,
            yuv_packed_alpha_pipelines,
            nv12_bind_group_layout,
            nv12_shader,
            nv12_pipeline_layout,
            nv12_pipelines,
            nv12_packed_alpha_pipelines,
        })
    }

    pub fn get_yuv_pipeline(
        &self,
        format: wgpu::TextureFormat,
        full_range: bool,
    ) -> Arc<wgpu::RenderPipeline> {
        let mut pipelines = self.yuv_pipelines.lock().unwrap();
        pipelines
            .entry((format, full_range))
            .or_insert_with(|| {
                let entry_point = if full_range {
                    "fs_yuv_full"
                } else {
                    "fs_yuv_limited"
                };
                let pipeline =
                    self.device
                        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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

    pub fn get_nv12_pipeline(
        &self,
        format: wgpu::TextureFormat,
        full_range: bool,
    ) -> Arc<wgpu::RenderPipeline> {
        let mut pipelines = self.nv12_pipelines.lock().unwrap();
        pipelines
            .entry((format, full_range))
            .or_insert_with(|| {
                let entry_point = if full_range {
                    "fs_nv12_full"
                } else {
                    "fs_nv12_limited"
                };
                let pipeline =
                    self.device
                        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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

    pub fn get_yuv_packed_alpha_pipeline(
        &self,
        format: wgpu::TextureFormat,
    ) -> Arc<wgpu::RenderPipeline> {
        let mut pipelines = self.yuv_packed_alpha_pipelines.lock().unwrap();
        pipelines
            .entry(format)
            .or_insert_with(|| {
                let pipeline =
                    self.device
                        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                            label: Some("YUV Packed Alpha Render Pipeline"),
                            layout: Some(&self.yuv_pipeline_layout),
                            vertex: wgpu::VertexState {
                                module: &self.yuv_shader,
                                entry_point: Some("vs_main"),
                                buffers: &[],
                                compilation_options: Default::default(),
                            },
                            fragment: Some(wgpu::FragmentState {
                                module: &self.yuv_shader,
                                entry_point: Some("fs_yuv_packed_alpha"),
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

    pub fn get_nv12_packed_alpha_pipeline(
        &self,
        format: wgpu::TextureFormat,
    ) -> Arc<wgpu::RenderPipeline> {
        let mut pipelines = self.nv12_packed_alpha_pipelines.lock().unwrap();
        pipelines
            .entry(format)
            .or_insert_with(|| {
                let pipeline =
                    self.device
                        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                            label: Some("NV12 Packed Alpha Render Pipeline"),
                            layout: Some(&self.nv12_pipeline_layout),
                            vertex: wgpu::VertexState {
                                module: &self.nv12_shader,
                                entry_point: Some("vs_main"),
                                buffers: &[],
                                compilation_options: Default::default(),
                            },
                            fragment: Some(wgpu::FragmentState {
                                module: &self.nv12_shader,
                                entry_point: Some("fs_nv12_packed_alpha"),
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
                let pipeline =
                    self.device
                        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
