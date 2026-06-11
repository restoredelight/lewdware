/// VideoToolbox + IOSurface zero-copy: imports CVPixelBuffer-backed frames
/// directly as Metal textures, eliminating the GPU->CPU->GPU round-trip.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use objc2::{msg_send_id, rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureType,
    MTLTextureUsage,
};
use objc2_io_surface::IOSurfaceRef;

use crate::video::VtbFrame;

/// Holds the wgpu textures and bind group for an IOSurface-imported NV12 frame.
/// Dropping this releases the Metal textures and the CVPixelBuffer reference.
pub struct VtbImportedTextures {
    pub y_texture: wgpu::Texture,
    pub uv_texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
    /// Keeps the CVPixelBuffer (and its IOSurface) alive until wgpu is done with the textures.
    _vtb_frame: Arc<VtbFrame>,
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferGetIOSurface(buffer: *const std::ffi::c_void) -> *mut IOSurfaceRef;
}

/// Tries to import a VideoToolbox CVPixelBuffer frame as wgpu textures via Metal IOSurface.
/// Returns `None` if the device is not Metal or if IOSurface is unavailable.
pub fn try_import_vtb_frame(
    device: &wgpu::Device,
    vtb_frame: &Arc<VtbFrame>,
    video_width: u32,
    video_height: u32,
    nv12_bind_group_layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
) -> Option<VtbImportedTextures> {
    let iosurface = unsafe { CVPixelBufferGetIOSurface(vtb_frame.pixel_buf) };
    if iosurface.is_null() {
        return None;
    }

    let hal_guard = unsafe { device.as_hal::<wgpu::hal::metal::Api>() };
    let hal_device = hal_guard.as_deref()?;
    let metal_device = hal_device.raw_device();

    let chroma_w = (video_width + 1) / 2;
    let chroma_h = (video_height + 1) / 2;

    // Create Metal textures backed by the IOSurface planes (no copy).
    let y_metal = unsafe {
        create_iosurface_texture(
            metal_device,
            iosurface,
            MTLPixelFormat::R8Unorm,
            video_width,
            video_height,
            0,
        )?
    };
    let uv_metal = unsafe {
        create_iosurface_texture(
            metal_device,
            iosurface,
            MTLPixelFormat::RG8Unorm,
            chroma_w,
            chroma_h,
            1,
        )?
    };

    let y_tex_desc = wgpu::TextureDescriptor {
        label: Some("VTB Y Plane"),
        size: wgpu::Extent3d {
            width: video_width,
            height: video_height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };
    let uv_tex_desc = wgpu::TextureDescriptor {
        label: Some("VTB UV Plane"),
        size: wgpu::Extent3d {
            width: chroma_w,
            height: chroma_h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };

    let y_texture = unsafe {
        let hal_tex = wgpu::hal::metal::Device::texture_from_raw(
            y_metal,
            wgpu::TextureFormat::R8Unorm,
            MTLTextureType::Type2D,
            1,
            1,
            wgpu::hal::CopyExtent { width: video_width, height: video_height, depth: 1 },
        );
        device.create_texture_from_hal::<wgpu::hal::metal::Api>(hal_tex, &y_tex_desc)
    };
    let uv_texture = unsafe {
        let hal_tex = wgpu::hal::metal::Device::texture_from_raw(
            uv_metal,
            wgpu::TextureFormat::Rg8Unorm,
            MTLTextureType::Type2D,
            1,
            1,
            wgpu::hal::CopyExtent { width: chroma_w, height: chroma_h, depth: 1 },
        );
        device.create_texture_from_hal::<wgpu::hal::metal::Api>(hal_tex, &uv_tex_desc)
    };

    let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let uv_view = uv_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("VTB NV12 Bind Group"),
        layout: nv12_bind_group_layout,
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
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });

    static LOGGED: AtomicBool = AtomicBool::new(false);
    if !LOGGED.swap(true, Ordering::Relaxed) {
        eprintln!("[vtb_import] VideoToolbox IOSurface zero-copy active");
    }

    Some(VtbImportedTextures {
        y_texture,
        uv_texture,
        bind_group,
        _vtb_frame: Arc::clone(vtb_frame),
    })
}

/// Creates a Metal texture backed by a specific IOSurface plane (no pixel copy).
unsafe fn create_iosurface_texture(
    device: &Retained<ProtocolObject<dyn MTLDevice>>,
    iosurface: *mut IOSurfaceRef,
    pixel_format: MTLPixelFormat,
    width: u32,
    height: u32,
    plane: usize,
) -> Option<Retained<ProtocolObject<dyn MTLTexture>>> {
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            pixel_format,
            width as usize,
            height as usize,
            false,
        )
    };
    desc.setUsage(MTLTextureUsage::ShaderRead);
    // Shared storage: IOSurface memory is always CPU+GPU accessible.
    desc.setStorageMode(MTLStorageMode::Shared);

    // `newTextureWithDescriptor:iosurface:plane:` is not in objc2-metal 0.3.2 bindings,
    // so we call it via raw message send.
    unsafe {
        msg_send_id![
            device,
            newTextureWithDescriptor: &*desc,
            iosurface: iosurface,
            plane: plane
        ]
    }
}
