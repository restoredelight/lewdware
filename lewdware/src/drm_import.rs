/// DMA-BUF zero-copy import: maps VAAPI-decoded DRM PRIME frames directly into
/// Vulkan textures, eliminating the GPU->CPU->GPU round-trip of av_hwframe_transfer_data.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ash::vk;
use ffmpeg_next::ffi;

use crate::video::DrmPrimeFrame;

/// Holds the wgpu textures and bind group for a DMA-BUF imported NV12 frame.
/// Dropping this struct frees the underlying Vulkan memory (closes the DMA-BUF fd)
/// and releases the VAAPI surface back to the decoder pool.
pub struct DrmImportedTextures {
    pub _y_texture: wgpu::Texture,
    pub _uv_texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
    /// Keeps the VAAPI surface alive until the wgpu textures are dropped. Without this,
    /// the decoder could reuse the underlying GEM object for a new frame while the GPU
    /// is still reading the old frame's data through the dup'd DMA-BUF fd.
    _drm_frame: Arc<DrmPrimeFrame>,
}

/// Tries to import a DRM PRIME frame as wgpu textures via Vulkan external memory.
/// Returns `None` if import fails (extension not available, stride mismatch, etc.).
/// Takes `&Arc<DrmPrimeFrame>` so the imported textures can share ownership and keep
/// the VAAPI surface alive for as long as the GPU may be reading from it.
pub fn try_import_drm_prime(
    device: &wgpu::Device,
    drm_prime: &Arc<DrmPrimeFrame>,
    video_width: u32,
    video_height: u32,
    nv12_bind_group_layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
) -> Option<DrmImportedTextures> {
    // Safety: data[0] is a valid AVDRMFrameDescriptor pointer for DRM PRIME frames.
    let desc_ptr =
        unsafe { (*drm_prime.frame.as_ptr()).data[0] } as *const ffi::AVDRMFrameDescriptor;
    if desc_ptr.is_null() {
        return None;
    }
    let desc = unsafe { &*desc_ptr };

    if desc.nb_objects != 1 {
        eprintln!("[drm_import] unexpected nb_objects={}", desc.nb_objects);
        return None;
    }

    let modifier = desc.objects[0].format_modifier;

    // Extract Y and UV plane info from whichever layout is used.
    let (y_offset, y_pitch, uv_offset, uv_pitch) = if desc.nb_layers == 1
        && desc.layers[0].nb_planes == 2
    {
        let l = &desc.layers[0];
        (
            l.planes[0].offset as u64,
            l.planes[0].pitch as u64,
            l.planes[1].offset as u64,
            l.planes[1].pitch as u64,
        )
    } else if desc.nb_layers == 2 && desc.layers[0].nb_planes == 1 && desc.layers[1].nb_planes == 1
    {
        (
            desc.layers[0].planes[0].offset as u64,
            desc.layers[0].planes[0].pitch as u64,
            desc.layers[1].planes[0].offset as u64,
            desc.layers[1].planes[0].pitch as u64,
        )
    } else {
        eprintln!(
            "[drm_import] unhandled layout: nb_layers={} planes=[{}]",
            desc.nb_layers,
            (0..desc.nb_layers as usize)
                .map(|i| desc.layers[i].nb_planes.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        return None;
    };

    let fd = desc.objects[0].fd;
    let total_size = desc.objects[0].size as u64;

    let chroma_w = (video_width + 1) / 2;
    let chroma_h = (video_height + 1) / 2;

    static LOGGED_ATTEMPT: AtomicBool = AtomicBool::new(false);
    if !LOGGED_ATTEMPT.swap(true, Ordering::Relaxed) {
        let is_linear = modifier == 0 || modifier == u64::MAX;
        eprintln!(
            "[drm_import] first attempt: modifier={modifier:#018x} is_linear={is_linear} size={total_size} y_pitch={y_pitch} uv_pitch={uv_pitch} uv_offset={uv_offset}"
        );
    }

    let hal_guard = unsafe { device.as_hal::<wgpu::hal::vulkan::Api>() };
    let hal_device = hal_guard.as_deref()?;

    unsafe {
        import_via_vulkan(
            hal_device,
            device,
            fd,
            total_size,
            video_width,
            video_height,
            chroma_w,
            chroma_h,
            modifier,
            y_offset,
            y_pitch,
            uv_offset,
            uv_pitch,
            nv12_bind_group_layout,
            sampler,
            Arc::clone(drm_prime),
        )
    }
}

unsafe fn import_via_vulkan(
    hal: &wgpu::hal::vulkan::Device,
    wgpu_device: &wgpu::Device,
    fd: i32,
    total_size: u64,
    video_width: u32,
    video_height: u32,
    chroma_w: u32,
    chroma_h: u32,
    modifier: u64,
    y_offset: u64,
    y_pitch: u64,
    uv_offset: u64,
    uv_pitch: u64,
    nv12_bind_group_layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    drm_frame: Arc<DrmPrimeFrame>,
) -> Option<DrmImportedTextures> {
    let ash_device = hal.raw_device();
    let ash_instance = hal.shared_instance().raw_instance();
    let physical_device = hal.raw_physical_device();
    let ext_mem_fd = ash::khr::external_memory_fd::Device::new(ash_instance, ash_device);

    let is_linear = modifier == 0 || modifier == u64::MAX;
    let has_modifier_ext = hal
        .enabled_device_extensions()
        .contains(&ash::vk::EXT_IMAGE_DRM_FORMAT_MODIFIER_NAME);

    if !is_linear && !has_modifier_ext {
        static NOT_SUPPORTED: AtomicBool = AtomicBool::new(false);
        if !NOT_SUPPORTED.swap(true, Ordering::Relaxed) {
            eprintln!(
                "[drm_import] non-linear modifier {modifier:#018x} but VK_EXT_image_drm_format_modifier not enabled — falling back to CPU"
            );
        }
        return None;
    }

    // Create Y and UV plane images.
    // For linear: LINEAR tiling, bind at plane offset. Check stride matches DRM pitch.
    // For tiled: DRM_FORMAT_MODIFIER_EXT tiling, bind at offset 0, plane offset in modifier info.
    let (y_image, uv_image) = if is_linear {
        let y = unsafe {
            create_linear_image(ash_device, vk::Format::R8_UNORM, video_width, video_height)
        }?;
        let y_row_pitch = unsafe {
            ash_device.get_image_subresource_layout(
                y,
                vk::ImageSubresource {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    array_layer: 0,
                },
            )
        }
        .row_pitch as u64;
        if y_row_pitch != y_pitch {
            eprintln!("[drm_import] Y stride mismatch: Vulkan={y_row_pitch} DRM={y_pitch}");
            unsafe { ash_device.destroy_image(y, None) };
            return None;
        }
        let uv = match unsafe {
            create_linear_image(ash_device, vk::Format::R8G8_UNORM, chroma_w, chroma_h)
        } {
            Some(img) => img,
            None => {
                unsafe { ash_device.destroy_image(y, None) };
                return None;
            }
        };
        let uv_row_pitch = unsafe {
            ash_device.get_image_subresource_layout(
                uv,
                vk::ImageSubresource {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    array_layer: 0,
                },
            )
        }
        .row_pitch as u64;
        if uv_row_pitch != uv_pitch {
            eprintln!("[drm_import] UV stride mismatch: Vulkan={uv_row_pitch} DRM={uv_pitch}");
            unsafe {
                ash_device.destroy_image(y, None);
                ash_device.destroy_image(uv, None)
            };
            return None;
        }
        (y, uv)
    } else {
        // Tiled format: use VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT with explicit plane layouts.
        // Plane offsets are baked into the modifier create-info; we bind memory at offset 0.
        let y = match unsafe {
            create_modifier_image(
                ash_device,
                vk::Format::R8_UNORM,
                video_width,
                video_height,
                modifier,
                y_offset,
                y_pitch,
            )
        } {
            Some(img) => img,
            None => return None,
        };
        let uv = match unsafe {
            create_modifier_image(
                ash_device,
                vk::Format::R8G8_UNORM,
                chroma_w,
                chroma_h,
                modifier,
                uv_offset,
                uv_pitch,
            )
        } {
            Some(img) => img,
            None => {
                unsafe { ash_device.destroy_image(y, None) };
                return None;
            }
        };
        (y, uv)
    };

    // Determine bind offsets: linear binds at plane offset within DMA-BUF; modifier images bind at 0.
    let (y_bind_offset, uv_bind_offset) = if is_linear {
        (y_offset, uv_offset)
    } else {
        (0, 0)
    };

    // Memory requirements and DMA-BUF compatible memory types.
    let y_mem_req = unsafe { ash_device.get_image_memory_requirements(y_image) };
    let uv_mem_req = unsafe { ash_device.get_image_memory_requirements(uv_image) };

    let mut fd_props = vk::MemoryFdPropertiesKHR::default();
    let fd_dup = unsafe { libc::dup(fd) };
    if fd_dup < 0 {
        unsafe {
            ash_device.destroy_image(y_image, None);
            ash_device.destroy_image(uv_image, None)
        };
        return None;
    }
    let props_ok = unsafe {
        ext_mem_fd.get_memory_fd_properties(
            vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
            fd_dup,
            &mut fd_props,
        )
    };
    unsafe { libc::close(fd_dup) };
    if props_ok.is_err() {
        unsafe {
            ash_device.destroy_image(y_image, None);
            ash_device.destroy_image(uv_image, None)
        };
        return None;
    }

    let mem_props = unsafe { ash_instance.get_physical_device_memory_properties(physical_device) };
    let compatible =
        y_mem_req.memory_type_bits & uv_mem_req.memory_type_bits & fd_props.memory_type_bits;
    let mem_type = find_memory_type(&mem_props, compatible)?;

    // Import DMA-BUF for Y plane.
    let y_fd = unsafe { libc::dup(fd) };
    if y_fd < 0 {
        unsafe {
            ash_device.destroy_image(y_image, None);
            ash_device.destroy_image(uv_image, None)
        };
        return None;
    }
    let y_memory = {
        let mut import = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
            .fd(y_fd);
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(total_size)
            .memory_type_index(mem_type as u32)
            .push_next(&mut import);
        match unsafe { ash_device.allocate_memory(&alloc, None) } {
            Ok(m) => m,
            Err(_) => {
                unsafe {
                    libc::close(y_fd);
                    ash_device.destroy_image(y_image, None);
                    ash_device.destroy_image(uv_image, None)
                };
                return None;
            }
        }
    };
    if unsafe { ash_device.bind_image_memory(y_image, y_memory, y_bind_offset) }.is_err() {
        unsafe {
            ash_device.free_memory(y_memory, None);
            ash_device.destroy_image(y_image, None);
            ash_device.destroy_image(uv_image, None)
        };
        return None;
    }

    // Import DMA-BUF for UV plane (second dup of same fd).
    let uv_fd = unsafe { libc::dup(fd) };
    if uv_fd < 0 {
        unsafe {
            ash_device.free_memory(y_memory, None);
            ash_device.destroy_image(y_image, None);
            ash_device.destroy_image(uv_image, None)
        };
        return None;
    }
    let uv_memory = {
        let mut import = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
            .fd(uv_fd);
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(total_size)
            .memory_type_index(mem_type as u32)
            .push_next(&mut import);
        match unsafe { ash_device.allocate_memory(&alloc, None) } {
            Ok(m) => m,
            Err(_) => {
                unsafe {
                    libc::close(uv_fd);
                    ash_device.free_memory(y_memory, None);
                    ash_device.destroy_image(y_image, None);
                    ash_device.destroy_image(uv_image, None)
                };
                return None;
            }
        }
    };
    if unsafe { ash_device.bind_image_memory(uv_image, uv_memory, uv_bind_offset) }.is_err() {
        unsafe {
            ash_device.free_memory(uv_memory, None);
            ash_device.free_memory(y_memory, None);
            ash_device.destroy_image(y_image, None);
            ash_device.destroy_image(uv_image, None)
        };
        return None;
    }

    // Wrap as wgpu textures. TextureMemory::Dedicated frees the VkDeviceMemory (closes the DMA-BUF fd) on drop.
    let y_tex_desc = wgpu::TextureDescriptor {
        label: Some("DRM PRIME Y Plane"),
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
        label: Some("DRM PRIME UV Plane"),
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
    let hal_desc_y = wgpu::hal::TextureDescriptor {
        label: y_tex_desc.label,
        size: y_tex_desc.size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUses::RESOURCE,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: vec![],
    };
    let hal_desc_uv = wgpu::hal::TextureDescriptor {
        label: uv_tex_desc.label,
        size: uv_tex_desc.size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg8Unorm,
        usage: wgpu::TextureUses::RESOURCE,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: vec![],
    };

    let y_texture = unsafe {
        let hal_tex = hal.texture_from_raw(
            y_image,
            &hal_desc_y,
            None,
            wgpu::hal::vulkan::TextureMemory::Dedicated(y_memory),
        );
        wgpu_device.create_texture_from_hal::<wgpu::hal::vulkan::Api>(hal_tex, &y_tex_desc)
    };
    let uv_texture = unsafe {
        let hal_tex = hal.texture_from_raw(
            uv_image,
            &hal_desc_uv,
            None,
            wgpu::hal::vulkan::TextureMemory::Dedicated(uv_memory),
        );
        wgpu_device.create_texture_from_hal::<wgpu::hal::vulkan::Api>(hal_tex, &uv_tex_desc)
    };

    let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let uv_view = uv_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let bind_group = wgpu_device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("DRM PRIME NV12 Bind Group"),
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

    static LOGGED_SUCCESS: AtomicBool = AtomicBool::new(false);
    if !LOGGED_SUCCESS.swap(true, Ordering::Relaxed) {
        let path = if is_linear {
            "linear"
        } else {
            "tiled (modifier ext)"
        };
        eprintln!(
            "[drm_import] DMA-BUF zero-copy active ({path}) — GPU→CPU→GPU round-trip eliminated"
        );
    }
    Some(DrmImportedTextures {
        _y_texture: y_texture,
        _uv_texture: uv_texture,
        bind_group,
        _drm_frame: drm_frame,
    })
}

/// LINEAR-tiled VkImage for external DMA-BUF import.
unsafe fn create_linear_image(
    device: &ash::Device,
    format: vk::Format,
    width: u32,
    height: u32,
) -> Option<vk::Image> {
    let mut ext = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::LINEAR)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut ext);
    unsafe { device.create_image(&info, None) }.ok()
}

/// DRM_FORMAT_MODIFIER_EXT-tiled VkImage for tiled DMA-BUF import.
/// `plane_offset` and `row_pitch` encode where this plane lives within the DMA-BUF.
unsafe fn create_modifier_image(
    device: &ash::Device,
    format: vk::Format,
    width: u32,
    height: u32,
    modifier: u64,
    plane_offset: u64,
    row_pitch: u64,
) -> Option<vk::Image> {
    let plane_layout = vk::SubresourceLayout {
        offset: plane_offset,
        size: 0,
        row_pitch,
        array_pitch: 0,
        depth_pitch: 0,
    };
    let mut mod_info = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
        .drm_format_modifier(modifier)
        .plane_layouts(std::slice::from_ref(&plane_layout));
    let mut ext = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut ext)
        .push_next(&mut mod_info);
    unsafe { device.create_image(&info, None) }.ok()
}

fn find_memory_type(
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
) -> Option<usize> {
    (0..mem_props.memory_type_count as usize).find(|&i| type_bits & (1 << i) != 0)
}
