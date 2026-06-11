use std::sync::Arc;

#[cfg(target_os = "windows")]
use crate::d3d12_import::D3d12ImportedTextures;
#[cfg(target_os = "linux")]
use crate::drm_import::DrmImportedTextures;
#[cfg(target_os = "macos")]
use crate::vtb_import::VtbImportedTextures;
use crate::{
    wgpu::WgpuState,
    video::{VideoFrame, VideoPixelFormat},
};

pub struct VideoRenderer {
    frame_textures: VideoFrameTextures,
    video_width: u32,
    video_height: u32,
    // CPU-rendered UI / decorations overlay
    ui_texture: wgpu::Texture,
    ui_bind_group: wgpu::BindGroup,
    ui_pipeline: Arc<wgpu::RenderPipeline>,
    // DMA-BUF imported textures ring (Linux only): keeps last N frames alive for GPU safety.
    // The most recent entry (back) is the active frame for rendering.
    #[cfg(target_os = "linux")]
    drm_ring: std::collections::VecDeque<DrmImportedTextures>,
    // IOSurface-backed Metal textures ring (macOS only): same purpose as drm_ring.
    #[cfg(target_os = "macos")]
    vtb_ring: std::collections::VecDeque<VtbImportedTextures>,
    // D3D12VA-decoded NV12 textures ring (Windows only): same purpose as drm_ring.
    #[cfg(target_os = "windows")]
    d3d12_ring: std::collections::VecDeque<D3d12ImportedTextures>,
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
        ui_width: u32,
        ui_height: u32,
    ) -> Self {
        let device = &wgpu_state.device;
        let chroma_w = (video_width + 1) / 2;
        let chroma_h = (video_height + 1) / 2;

        let frame_textures = match pixel_format {
            VideoPixelFormat::Yuv420p => {
                let y_texture = make_r8_texture(device, "Y Plane", video_width, video_height);
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

                let pipeline = wgpu_state.get_yuv_pipeline(format, full_range);
                VideoFrameTextures::Yuv420p {
                    y_texture,
                    cb_texture,
                    cr_texture,
                    bind_group,
                    pipeline,
                }
            }
            VideoPixelFormat::Nv12 => {
                let y_texture = make_r8_texture(device, "Y Plane", video_width, video_height);
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

                let pipeline = wgpu_state.get_nv12_pipeline(format, full_range);
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
            video_height,
            ui_texture,
            ui_bind_group,
            ui_pipeline,
            #[cfg(target_os = "linux")]
            drm_ring: std::collections::VecDeque::new(),
            #[cfg(target_os = "macos")]
            vtb_ring: std::collections::VecDeque::new(),
            #[cfg(target_os = "windows")]
            d3d12_ring: std::collections::VecDeque::new(),
        }
    }

    pub fn update_ui(&self, queue: &wgpu::Queue, data: &[u8], width: u32, height: u32) {
        upload_texture_data(queue, &self.ui_texture, data, width, height, width * 4, 4);
    }

    pub fn update_video(&mut self, wgpu_state: &WgpuState, frame: &VideoFrame) {
        // On Linux, try zero-copy DRM PRIME import first.
        #[cfg(target_os = "linux")]
        if let Some(drm_prime) = &frame.drm_prime {
            if let VideoFrameTextures::Nv12 { .. } = &self.frame_textures {
                if let Some(imported) = crate::drm_import::try_import_drm_prime(
                    &wgpu_state.device,
                    drm_prime,
                    self.video_width,
                    self.video_height,
                    &wgpu_state.nv12_bind_group_layout,
                    &wgpu_state.sampler,
                ) {
                    self.drm_ring.push_back(imported);
                    // Keep at most 3 frames in the ring so GPU can finish with older ones.
                    while self.drm_ring.len() > 3 {
                        self.drm_ring.pop_front();
                    }
                    return;
                }
            }
        }

        // CPU path: clear the DRM ring (DRM PRIME not in use for this frame).
        #[cfg(target_os = "linux")]
        self.drm_ring.clear();

        // On macOS, try VideoToolbox IOSurface zero-copy import.
        #[cfg(target_os = "macos")]
        if let Some(vtb_frame) = &frame.vtb_frame {
            if let VideoFrameTextures::Nv12 { .. } = &self.frame_textures {
                if let Some(imported) = crate::vtb_import::try_import_vtb_frame(
                    &wgpu_state.device,
                    vtb_frame,
                    self.video_width,
                    self.video_height,
                    &wgpu_state.nv12_bind_group_layout,
                    &wgpu_state.sampler,
                ) {
                    self.vtb_ring.push_back(imported);
                    while self.vtb_ring.len() > 3 {
                        self.vtb_ring.pop_front();
                    }
                    return;
                }
            }
        }
        // CPU path: clear the VTB ring (IOSurface not in use for this frame).
        #[cfg(target_os = "macos")]
        self.vtb_ring.clear();

        // On Windows, try D3D12VA zero-copy NV12 import.
        #[cfg(target_os = "windows")]
        if let Some(d3d12_frame) = &frame.d3d12va_frame {
            if let VideoFrameTextures::Nv12 { .. } = &self.frame_textures {
                if let Some(imported) = crate::d3d12_import::try_import_d3d12va_frame(
                    &wgpu_state.device,
                    d3d12_frame.clone(),
                    self.video_width,
                    self.video_height,
                    &wgpu_state.nv12_bind_group_layout,
                    &wgpu_state.sampler,
                ) {
                    self.d3d12_ring.push_back(imported);
                    while self.d3d12_ring.len() > 3 {
                        self.d3d12_ring.pop_front();
                    }
                    return;
                }
            }
        }
        // CPU path: clear the D3D12 ring (zero-copy not in use for this frame).
        #[cfg(target_os = "windows")]
        self.d3d12_ring.clear();

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
        // On Linux, prefer the DRM-imported bind group from the most recent frame.
        #[cfg(target_os = "linux")]
        if let Some(latest) = self.drm_ring.back() {
            let pipeline = match &self.frame_textures {
                VideoFrameTextures::Nv12 { pipeline, .. } => pipeline.as_ref(),
                VideoFrameTextures::Yuv420p { pipeline, .. } => pipeline.as_ref(),
            };
            return (pipeline, &latest.bind_group);
        }
        // On macOS, prefer the IOSurface-backed bind group from the most recent frame.
        #[cfg(target_os = "macos")]
        if let Some(latest) = self.vtb_ring.back() {
            let pipeline = match &self.frame_textures {
                VideoFrameTextures::Nv12 { pipeline, .. } => pipeline.as_ref(),
                VideoFrameTextures::Yuv420p { pipeline, .. } => pipeline.as_ref(),
            };
            return (pipeline, &latest.bind_group);
        }
        // On Windows, prefer the D3D12VA-imported bind group from the most recent frame.
        #[cfg(target_os = "windows")]
        if let Some(latest) = self.d3d12_ring.back() {
            let pipeline = match &self.frame_textures {
                VideoFrameTextures::Nv12 { pipeline, .. } => pipeline.as_ref(),
                VideoFrameTextures::Yuv420p { pipeline, .. } => pipeline.as_ref(),
            };
            return (pipeline, &latest.bind_group);
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
