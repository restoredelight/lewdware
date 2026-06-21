use std::{collections::VecDeque, sync::Arc};
use wgpu::util::DeviceExt;

use crate::{
    video::{VideoFrame, VideoPixelFormat},
    wgpu::WgpuState,
    zero_copy::{ImportOpts, ImportedTextures},
};

pub struct GpuRenderer {
    pub opacity_buffer: wgpu::Buffer,
    pub window_bind_group: wgpu::BindGroup,
    pub renderer_type: GpuRendererType,
}

/// Mirrors the `WindowOptions` uniform struct in the shaders. `premultiplied` tells the
/// fragment shaders whether the surface expects premultiplied alpha (`CompositeAlphaMode::
/// PreMultiplied`) or straight alpha (anything else, since e.g. `Opaque` ignores alpha
/// entirely) — see `InnerWindow::premultiplied_alpha`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WindowUniform {
    opacity: f32,
    premultiplied: u32,
    force_opaque: u32,
}

pub enum GpuRendererType {
    Image {
        texture: wgpu::Texture,
        bind_group: wgpu::BindGroup,
    },
    Video(VideoRenderer),
}

impl GpuRenderer {
    pub fn new_image(
        wgpu_state: &WgpuState,
        width: u32,
        height: u32,
        opacity: f32,
        premultiplied_alpha: bool,
        force_opaque: bool,
    ) -> Self {
        let device = &wgpu_state.device;

        let opacity_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Opacity Buffer"),
            contents: bytemuck::bytes_of(&WindowUniform {
                opacity,
                premultiplied: premultiplied_alpha as u32,
                force_opaque: force_opaque as u32,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let window_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Window Bind Group"),
            layout: &wgpu_state.window_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: opacity_buffer.as_entire_binding(),
            }],
        });

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Frame Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Frame Bind Group"),
            layout: &wgpu_state.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&wgpu_state.sampler),
                },
            ],
        });

        Self {
            opacity_buffer,
            window_bind_group,
            renderer_type: GpuRendererType::Image {
                texture,
                bind_group,
            },
        }
    }

    pub fn new_video(
        wgpu_state: &WgpuState,
        format: wgpu::TextureFormat,
        video_width: u32,
        video_height: u32,
        full_range: bool,
        pixel_format: VideoPixelFormat,
        packed_alpha: bool,
        ui_width: u32,
        ui_height: u32,
        opacity: f32,
        premultiplied_alpha: bool,
        force_opaque: bool,
    ) -> Self {
        let device = &wgpu_state.device;

        let opacity_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Opacity Buffer"),
            contents: bytemuck::bytes_of(&WindowUniform {
                opacity,
                premultiplied: premultiplied_alpha as u32,
                force_opaque: force_opaque as u32,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let window_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Window Bind Group"),
            layout: &wgpu_state.window_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: opacity_buffer.as_entire_binding(),
            }],
        });

        let video_renderer = VideoRenderer::new(
            wgpu_state,
            format,
            video_width,
            video_height,
            full_range,
            pixel_format,
            packed_alpha,
            ui_width,
            ui_height,
        );

        Self {
            opacity_buffer,
            window_bind_group,
            renderer_type: GpuRendererType::Video(video_renderer),
        }
    }

    pub fn set_opacity(&self, wgpu_state: &WgpuState, opacity: f32) {
        wgpu_state
            .queue
            .write_buffer(&self.opacity_buffer, 0, bytemuck::cast_slice(&[opacity]));
    }

    /// Upload CPU-side frame buffer data: for Image writes to the frame texture; for Video
    /// writes to the UI overlay texture.
    pub fn upload_frame_buffer(&self, queue: &wgpu::Queue, data: &[u8], width: u32, height: u32) {
        match &self.renderer_type {
            GpuRendererType::Image { texture, .. } => {
                upload_texture_data(queue, texture, data, width, height, width * 4, 4);
            }
            GpuRendererType::Video(video) => {
                video.update_ui(queue, data, width, height);
            }
        }
    }
}

pub struct VideoRenderer {
    frame_textures: VideoFrameTextures,
    video_width: u32,
    video_height: u32,
    // CPU-rendered UI / decorations overlay
    ui_texture: wgpu::Texture,
    ui_bind_group: wgpu::BindGroup,
    ui_pipeline: Arc<wgpu::RenderPipeline>,
    // GPU imported textures ring: keeps last N frames alive for GPU safety.
    // The most recent entry (back) is the active frame for rendering.
    hardware_textures_ring: VecDeque<ImportedTextures>,
}

enum VideoFrameTextures {
    Yuv420p {
        y_texture: wgpu::Texture,
        cb_texture: wgpu::Texture,
        cr_texture: wgpu::Texture,
        bind_group: wgpu::BindGroup,
        pipeline: Arc<wgpu::RenderPipeline>,
    },
    Nv12 {
        y_texture: wgpu::Texture,
        uv_texture: wgpu::Texture,
        bind_group: wgpu::BindGroup,
        pipeline: Arc<wgpu::RenderPipeline>,
    },
}

// Make a texture with a single colour channel
fn make_r8_texture(device: &wgpu::Device, label: &str, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

// Make a texture with two colour channels
fn make_rg8_texture(device: &wgpu::Device, label: &str, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

impl VideoRenderer {
    pub fn new(
        wgpu_state: &WgpuState,
        format: wgpu::TextureFormat,
        video_width: u32,
        video_height: u32,
        full_range: bool,
        pixel_format: VideoPixelFormat,
        packed_alpha: bool,
        ui_width: u32,
        ui_height: u32,
    ) -> Self {
        let device = &wgpu_state.device;
        // When packed_alpha, the decoded frame is twice the display height.
        let decoded_height = if packed_alpha {
            video_height * 2
        } else {
            video_height
        };
        let chroma_w = (video_width + 1) / 2;
        let chroma_h = (decoded_height + 1) / 2;

        let frame_textures = match pixel_format {
            VideoPixelFormat::Yuv420p => {
                // Y texture covers the full decoded height (2x display height when packed).
                let y_texture = make_r8_texture(device, "Y Plane", video_width, decoded_height);
                // Cb/Cr chroma height = decoded_height / 2 = display height when packed.
                let cb_texture = make_r8_texture(device, "Cb Plane", chroma_w, chroma_h);
                let cr_texture = make_r8_texture(device, "Cr Plane", chroma_w, chroma_h);

                let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
                let cb_view = cb_texture.create_view(&wgpu::TextureViewDescriptor::default());
                let cr_view = cr_texture.create_view(&wgpu::TextureViewDescriptor::default());

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("YUV Bind Group"),
                    layout: &wgpu_state.yuv_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&y_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&cb_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&cr_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&wgpu_state.sampler),
                        },
                    ],
                });

                let pipeline = if packed_alpha {
                    wgpu_state.get_yuv_packed_alpha_pipeline(format)
                } else {
                    wgpu_state.get_yuv_pipeline(format, full_range)
                };
                VideoFrameTextures::Yuv420p {
                    y_texture,
                    cb_texture,
                    cr_texture,
                    bind_group,
                    pipeline,
                }
            }
            VideoPixelFormat::Nv12 => {
                // Y texture covers the full decoded height (2x display height when packed).
                let y_texture = make_r8_texture(device, "Y Plane", video_width, decoded_height);
                // UV chroma height = decoded_height / 2 = display height when packed.
                let uv_texture = make_rg8_texture(device, "UV Plane", chroma_w, chroma_h);

                let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
                let uv_view = uv_texture.create_view(&wgpu::TextureViewDescriptor::default());

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("NV12 Bind Group"),
                    layout: &wgpu_state.nv12_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&y_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&uv_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&wgpu_state.sampler),
                        },
                    ],
                });

                let pipeline = if packed_alpha {
                    wgpu_state.get_nv12_packed_alpha_pipeline(format)
                } else {
                    wgpu_state.get_nv12_pipeline(format, full_range)
                };
                VideoFrameTextures::Nv12 {
                    y_texture,
                    uv_texture,
                    bind_group,
                    pipeline,
                }
            }
        };

        let ui_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("UI Texture"),
            size: wgpu::Extent3d {
                width: ui_width,
                height: ui_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let ui_view = ui_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let ui_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("UI Bind Group"),
            layout: &wgpu_state.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&ui_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&wgpu_state.sampler),
                },
            ],
        });

        let ui_pipeline = wgpu_state.get_pipeline(format);

        Self {
            frame_textures,
            video_width,
            video_height: decoded_height,
            ui_texture,
            ui_bind_group,
            ui_pipeline,
            hardware_textures_ring: VecDeque::new(),
        }
    }

    pub fn update_ui(&self, queue: &wgpu::Queue, data: &[u8], width: u32, height: u32) {
        upload_texture_data(queue, &self.ui_texture, data, width, height, width * 4, 4);
    }

    pub fn update_video(&mut self, wgpu_state: &WgpuState, frame: &VideoFrame) {
        if let Some(imported) = ImportedTextures::try_import_from_frame(
            wgpu_state,
            frame,
            ImportOpts {
                pix_fmt: self.frame_textures.pix_fmt(),
                video_width: self.video_width,
                video_height: self.video_height,
            },
        ) {
            self.hardware_textures_ring.push_back(imported);

            while self.hardware_textures_ring.len() > 3 {
                self.hardware_textures_ring.pop_front();
            }

            return;
        }

        self.hardware_textures_ring.clear();

        if frame.frame.width() == 0 {
            return;
        }

        let queue = &wgpu_state.queue;
        let w = frame.frame.width();
        let h = frame.frame.height();
        let chroma_w = (w + 1) / 2;
        let chroma_h = (h + 1) / 2;

        match &self.frame_textures {
            VideoFrameTextures::Yuv420p {
                y_texture,
                cb_texture,
                cr_texture,
                ..
            } => {
                upload_texture_data(
                    queue,
                    y_texture,
                    frame.frame.data(0),
                    w,
                    h,
                    frame.frame.stride(0) as u32,
                    1,
                );
                upload_texture_data(
                    queue,
                    cb_texture,
                    frame.frame.data(1),
                    chroma_w,
                    chroma_h,
                    frame.frame.stride(1) as u32,
                    1,
                );
                upload_texture_data(
                    queue,
                    cr_texture,
                    frame.frame.data(2),
                    chroma_w,
                    chroma_h,
                    frame.frame.stride(2) as u32,
                    1,
                );
            }
            VideoFrameTextures::Nv12 {
                y_texture,
                uv_texture,
                ..
            } => {
                // NV12: plane 0 = Y (full size, 1 byte/texel), plane 1 = UV (chroma size, 2 bytes/texel)
                upload_texture_data(
                    queue,
                    y_texture,
                    frame.frame.data(0),
                    w,
                    h,
                    frame.frame.stride(0) as u32,
                    1,
                );
                upload_texture_data(
                    queue,
                    uv_texture,
                    frame.frame.data(1),
                    chroma_w,
                    chroma_h,
                    frame.frame.stride(1) as u32,
                    2,
                );
            }
        }
    }

    pub fn video_pipeline_and_bind_group(&self) -> (&wgpu::RenderPipeline, &wgpu::BindGroup) {
        // Prefer the imported bind group from the most recent frame.
        if let Some(latest) = self.hardware_textures_ring.back() {
            let pipeline = match &self.frame_textures {
                VideoFrameTextures::Nv12 { pipeline, .. } => pipeline.as_ref(),
                VideoFrameTextures::Yuv420p { pipeline, .. } => pipeline.as_ref(),
            };

            return (pipeline, latest.bind_group());
        }

        match &self.frame_textures {
            VideoFrameTextures::Yuv420p {
                pipeline,
                bind_group,
                ..
            } => (pipeline, bind_group),
            VideoFrameTextures::Nv12 {
                pipeline,
                bind_group,
                ..
            } => (pipeline, bind_group),
        }
    }

    pub fn ui_bind_group(&self) -> &wgpu::BindGroup {
        &self.ui_bind_group
    }

    pub fn ui_pipeline(&self) -> &wgpu::RenderPipeline {
        &self.ui_pipeline
    }
}

pub fn upload_texture_data(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    data: &[u8],
    width: u32,
    height: u32,
    source_stride: u32,
    bytes_per_pixel: u32,
) {
    let unpadded_bytes_per_row = width * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row_padding = (align - unpadded_bytes_per_row % align) % align;
    let padded_bytes_per_row = unpadded_bytes_per_row + padded_bytes_per_row_padding;

    if source_stride == padded_bytes_per_row {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    } else {
        let mut padded_data = Vec::with_capacity((padded_bytes_per_row * height) as usize);
        for i in 0..height {
            let src_start = (i * source_stride) as usize;
            let src_end = src_start + unpadded_bytes_per_row as usize;
            padded_data.extend_from_slice(&data[src_start..src_end]);
            padded_data.extend(std::iter::repeat(0).take(padded_bytes_per_row_padding as usize));
        }

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &padded_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
}

impl VideoFrameTextures {
    fn pix_fmt(&self) -> VideoPixelFormat {
        match self {
            VideoFrameTextures::Yuv420p { .. } => VideoPixelFormat::Yuv420p,
            VideoFrameTextures::Nv12 { .. } => VideoPixelFormat::Nv12,
        }
    }
}

/// GPU overlay texture for window decorations (border + header). Used by prompt and choice
/// windows when rendering egui via `EguiGpuRenderer`. The texture is the same size as the outer
/// window and is composited on top of the egui layer using alpha blending.
pub struct DecorationOverlay {
    texture: wgpu::Texture,
    // Bind group for the overlay texture (group 0, RGBA pipeline).
    pub bind_group: wgpu::BindGroup,
    // Opacity + premultiplied uniform (group 1, RGBA pipeline).
    pub opacity_buffer: wgpu::Buffer,
    pub window_bind_group: wgpu::BindGroup,
    outer_width: u32,
    outer_height: u32,
}

impl DecorationOverlay {
    /// Create the decoration overlay. Draws a 1-pixel border into the texture immediately.
    ///
    /// * `border_offset` — physical pixels wide for the border (from `inner_offset().0`).
    pub fn new(
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

    pub fn set_opacity(&self, queue: &wgpu::Queue, opacity: f32) {
        queue.write_buffer(&self.opacity_buffer, 0, bytemuck::cast_slice(&[opacity]));
    }

    /// Upload a header pixmap into the overlay texture at `(origin_x, origin_y)`.
    pub fn upload_header(
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
                v.extend(std::iter::repeat(0u8).take(padding as usize));
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
    pub fn render(&self, rpass: &mut wgpu::RenderPass<'static>, pipeline: &wgpu::RenderPipeline) {
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
