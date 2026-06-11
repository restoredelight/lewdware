use std::{ffi::c_void, sync::Arc};

use ffmpeg_next::ffi;
use wgpu::hal::dx12 as dx12_hal;
use windows::Win32::Graphics::Direct3D12 as d3d12;
use windows::core::Interface; // as_raw(), from_raw_borrowed(), from_raw()

/// Mirror of `AVD3D12VASyncContext` from ffmpeg's hwcontext_d3d12va.h.
#[repr(C)]
pub struct AvD3d12VaSyncContext {
    pub fence: *mut c_void,       // ID3D12Fence*
    pub fence_event: *mut c_void, // HANDLE — must not be omitted; shifts fence_value
    pub fence_value: u64,
}

/// Mirror of `AVD3D12VAFrame` from ffmpeg's hwcontext_d3d12va.h (Windows x64 repr C).
#[repr(C)]
pub struct AvD3d12VaFrame {
    pub texture: *mut c_void,           // ID3D12Resource*
    pub subresource_index: i32,
    _pad: u32,                          // align sync_ctx.fence to 8 bytes
    pub sync_ctx: AvD3d12VaSyncContext, // embedded, NOT a pointer
    pub flags: u32,                     // AVD3D12VAFrameFlags
}

/// First field of `AVD3D12VADeviceContext` (the only one we need to set).
#[repr(C)]
struct AvD3d12VaDeviceCtx {
    device: *mut c_void, // ID3D12Device*
}

/// Allocates an `AVBufferRef` wrapping a D3D12VA hardware device context, with wgpu's
/// `ID3D12Device` injected so decoded textures land on the same device.
///
/// Returns `None` if the DX12 hal is unavailable or `av_hwdevice_ctx_init` fails.
///
/// # Safety
/// `wgpu_device` must outlive the returned buffer.
pub unsafe fn alloc_d3d12va_device_ctx(
    wgpu_device: &Arc<wgpu::Device>,
    hw_type: ffi::AVHWDeviceType,
) -> Option<*mut ffi::AVBufferRef> {
    // Get the raw ID3D12Device pointer without AddRef — lifetime is covered by Arc.
    let raw_d3d12_device: *mut c_void = unsafe {
        let hal = wgpu_device.as_hal::<dx12_hal::Api>()?;
        hal.raw_device().as_raw()
    };

    let ctx_buf = unsafe { ffi::av_hwdevice_ctx_alloc(hw_type) };
    if ctx_buf.is_null() {
        return None;
    }

    // ctx_buf->data points to AVHWDeviceContext; hwctx is at offset 16 (after av_class + type_).
    let hw_device_ctx = unsafe { (*ctx_buf).data } as *mut ffi::AVHWDeviceContext;
    let hwctx_ptr = unsafe { (*hw_device_ctx).hwctx } as *mut AvD3d12VaDeviceCtx;
    unsafe { (*hwctx_ptr).device = raw_d3d12_device };

    let ret = unsafe { ffi::av_hwdevice_ctx_init(ctx_buf) };
    if ret < 0 {
        let mut p = ctx_buf;
        unsafe { ffi::av_buffer_unref(&mut p) };
        return None;
    }

    Some(ctx_buf)
}

/// Textures imported from a D3D12VA frame for zero-copy rendering.
/// `_frame` keeps the ffmpeg decode surface alive until the wgpu texture is dropped.
pub struct D3d12ImportedTextures {
    pub nv12_texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
    pub _frame: Arc<crate::video::D3d12Frame>,
}

/// Import a D3D12VA hardware frame as a wgpu NV12 texture.
/// Issues a GPU-side fence `Wait` on wgpu's queue so the decoder finishes before sampling.
///
/// Returns `None` if the frame has no texture or the wgpu DX12 hal is unavailable.
pub fn try_import_d3d12va_frame(
    device: &wgpu::Device,
    frame: Arc<crate::video::D3d12Frame>,
    width: u32,
    height: u32,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
) -> Option<D3d12ImportedTextures> {
    if frame.texture_raw.is_null() {
        return None;
    }

    // GPU-side fence wait: same device as wgpu so a direct ID3D12CommandQueue::Wait works.
    if !frame.fence_raw.is_null() {
        unsafe {
            if let Some(hal_dev) = device.as_hal::<dx12_hal::Api>() {
                if let Some(fence) = d3d12::ID3D12Fence::from_raw_borrowed(&frame.fence_raw) {
                    let _ = hal_dev.raw_queue().Wait(fence, frame.fence_value);
                }
            }
        }
    }

    // Borrow ID3D12Resource without AddRef, read desc, then clone (AddRef) for ownership.
    // texture_from_raw takes ownership and will Release on drop.
    let (resource, array_size) = unsafe {
        let borrowed = d3d12::ID3D12Resource::from_raw_borrowed(&frame.texture_raw)?;
        let array_size = borrowed.GetDesc().DepthOrArraySize as u32;
        (borrowed.clone(), array_size)
        // `borrowed` drops here as a plain reference — no Release.
    };

    let hal_texture = unsafe {
        dx12_hal::Device::texture_from_raw(
            resource,
            wgpu::TextureFormat::NV12,
            wgpu::TextureDimension::D2,
            wgpu::Extent3d { width, height, depth_or_array_layers: array_size },
            1,
            1,
        )
    };

    let nv12_texture = unsafe {
        device.create_texture_from_hal::<dx12_hal::Api>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width, height, depth_or_array_layers: array_size },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::NV12,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        )
    };

    let y_view = nv12_texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(wgpu::TextureFormat::R8Unorm),
        aspect: wgpu::TextureAspect::Plane0,
        base_array_layer: frame.index,
        array_layer_count: Some(1),
        ..Default::default()
    });

    let uv_view = nv12_texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(wgpu::TextureFormat::Rg8Unorm),
        aspect: wgpu::TextureAspect::Plane1,
        base_array_layer: frame.index,
        array_layer_count: Some(1),
        ..Default::default()
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
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

    Some(D3d12ImportedTextures { nv12_texture, bind_group, _frame: frame })
}
