#[cfg(target_os = "linux")]
pub fn apply_wayland_preload_safeguards() {
    // Restore original LD_LIBRARY_PATH if running inside an AppImage to prevent
    // bundled library leakage into child processes (like WebKitWebProcess and ffmpeg)
    let is_appimage = std::env::var("APPIMAGE").is_ok() || std::env::var("APPDIR").is_ok();
    if is_appimage {
        if let Ok(old_path) = std::env::var("LD_LIBRARY_PATH_OLD") {
            unsafe {
                std::env::set_var("LD_LIBRARY_PATH", old_path);
            }
        } else {
            unsafe {
                std::env::remove_var("LD_LIBRARY_PATH");
            }
        }
    }

    // 1. Disable DMA-BUF and Compositing Mode rendering paths on Wayland/AppImage
    if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
        unsafe {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
    if std::env::var("WEBKIT_DISABLE_COMPOSITING_MODE").is_err() {
        unsafe {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
    }

    // 2. Preload host's libwayland-client.so.0 if running as an AppImage under Wayland
    let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .is_ok_and(|val| val.trim().eq_ignore_ascii_case("wayland"));
    let has_preload = std::env::var("LD_PRELOAD").is_ok();
    let preload_attempted = std::env::var("LEWDWARE_APPIMAGE_WAYLAND_PRELOAD_ATTEMPTED").is_ok();

    if is_appimage && is_wayland && !has_preload && !preload_attempted {
        let candidate_paths = [
            "/usr/lib64/libwayland-client.so.0",
            "/usr/lib64/libwayland-client.so",
            "/lib64/libwayland-client.so.0",
            "/usr/lib/x86_64-linux-gnu/libwayland-client.so.0",
            "/lib/x86_64-linux-gnu/libwayland-client.so.0",
            "/usr/lib/libwayland-client.so.0",
            "/usr/lib/libwayland-client.so",
        ];

        #[cfg(target_pointer_width = "64")]
        let process_elf_class = 2; // ELFCLASS64
        #[cfg(target_pointer_width = "32")]
        let process_elf_class = 1; // ELFCLASS32

        let mut found_path = None;
        for path in &candidate_paths {
            let p = std::path::Path::new(path);
            if p.is_file() {
                if let Ok(mut file) = std::fs::File::open(p) {
                    use std::io::Read;
                    let mut header = [0u8; 5];
                    if file.read_exact(&mut header).is_ok() {
                        if header[..4] == *b"\x7FELF" && header[4] == process_elf_class {
                            found_path = Some(*path);
                            break;
                        }
                    }
                }
            }
        }

        if let Some(preload_path) = found_path {
            let exe = std::env::var_os("APPIMAGE")
                .map(std::path::PathBuf::from)
                .or_else(|| std::fs::read_link("/proc/self/exe").ok());

            if let Some(exe_path) = exe {
                use std::os::unix::ffi::OsStringExt;
                use std::os::unix::process::CommandExt;

                // Read raw process arguments from /proc/self/cmdline to be UTF-8 safe
                let args = if let Ok(cmdline) = std::fs::read("/proc/self/cmdline") {
                    cmdline
                        .split(|byte| *byte == 0)
                        .filter(|arg| !arg.is_empty())
                        .skip(1)
                        .map(|arg| std::ffi::OsString::from_vec(arg.to_vec()))
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };

                let error = std::process::Command::new(exe_path)
                    .args(args)
                    .env("LD_PRELOAD", preload_path)
                    .env("LEWDWARE_APPIMAGE_WAYLAND_PRELOAD_ATTEMPTED", "1")
                    .exec();
                eprintln!("Lewdware AppImage Wayland preload skipped: failed to re-exec ({error:?})");
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn apply_wayland_preload_safeguards() {}
