use std::sync::Arc;

use ffmpeg_next::{ffi, frame::Video};

use crate::{
    video::{VideoFrame, VideoPixelFormat},
    wgpu::WgpuState,
};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::preferred_hw_type;
#[cfg(target_os = "macos")]
use {macos::VtbFrame as PlatformFrame, macos::VtbImportedTextures as PlatformImportedTextures};

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "windows")]
pub use windows::{initialize_hardware_device, preferred_hw_type};
#[cfg(target_os = "windows")]
use {
    windows::D3d12Frame as PlatformFrame,
    windows::D3d12ImportedTextures as PlatformImportedTextures,
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::preferred_hw_type;
#[cfg(target_os = "linux")]
use {
    linux::DrmImportedTextures as PlatformImportedTextures, linux::DrmPrimeFrame as PlatformFrame,
};

pub struct ImportedTextures(PlatformImportedTextures);

impl ImportedTextures {
    pub fn try_import_from_frame(
        wgpu_state: &WgpuState,
        frame: &VideoFrame,
        opts: ImportOpts,
    ) -> Option<Self> {
        if let Some(textures) =
            PlatformImportedTextures::try_import_from_frame(wgpu_state, frame, opts)
        {
            Some(Self(textures))
        } else {
            None
        }
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        self.0.bind_group()
    }
}

pub struct HardwareFrame(Arc<PlatformFrame>);

impl HardwareFrame {
    pub fn from_decoder_frame(decoded: &mut Video) -> Option<Self> {
        PlatformFrame::from_decoder_frame(decoded).map(|x| Self(Arc::new(x)))
    }
}

pub struct ImportOpts {
    pub pix_fmt: VideoPixelFormat,
    pub video_width: u32,
    pub video_height: u32,
}

#[cfg(not(target_os = "windows"))]
pub fn initialize_hardware_device(
    _wgpu_device: &Arc<wgpu::Device>,
    hw_type: ffi::AVHWDeviceType,
) -> Option<*mut ffi::AVBufferRef> {
    let mut hw_device_ctx: *mut ffi::AVBufferRef = std::ptr::null_mut();

    let ret = unsafe {
        ffi::av_hwdevice_ctx_create(
            &mut hw_device_ctx,
            hw_type,
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        )
    };

    if ret < 0 || hw_device_ctx.is_null() {
        return None;
    }

    Some(hw_device_ctx)
}
