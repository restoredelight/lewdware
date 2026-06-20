#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{env::args_os, fs::File, path::PathBuf};

use anyhow::{Context, Result};
use pollster::block_on;
use shared::user_config::{Mode, load_config};
use winit::event_loop::EventLoop;

use crate::{
    app::LewdwareApp,
    utils::{create_tray_icon, handle_sigterm, spawn_panic_thread},
    wgpu::WgpuState,
};

mod app;
mod audio;
mod egui;
mod error;
mod inner_window;
mod lua;
mod media;
mod monitor;
mod utils;
mod video;
mod wgpu;
mod window;
mod zero_copy;

fn main() -> Result<()> {
    let _log_guard = shared::logging::init("lewdware");

    let lock_path = dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("lewdware.lock");
    let lock_file = File::create(&lock_path).context("Failed to create lock file")?;
    if lock_file.try_lock().is_err() {
        tracing::error!("Another instance of lewdware is already running");
        return Ok(());
    }

    utils::raise_fd_limit();
    let mut args = args_os();

    let mut mode_path = None;
    let mut mode = None;
    while let Some(arg) = args.next() {
        if &arg == "--mode-path" {
            mode_path = Some(PathBuf::from(args.next().context("No mode path provided")?));
        }

        if &arg == "--mode" {
            mode = Some(
                args.next()
                    .context("No mode provided")?
                    .to_str()
                    .context("Invalid UTF-8")?
                    .to_string(),
            )
        }
    }

    let mut config = load_config()?;

    if let (Some(mode_path), Some(mode)) = (mode_path, mode) {
        config.mode = Mode::File {
            path: mode_path,
            mode,
        };
    }

    let mut event_loop_builder = EventLoop::with_user_event();

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;

        // Wayland doesn't support a bunch of stuff we need (e.g. setting the position of windows).
        // So we use XWayland instead.
        event_loop_builder.with_x11();
    }

    let event_loop = event_loop_builder.build()?;

    #[cfg(target_vendor = "apple")]
    utils::opt_in_secure_restorable_state();

    let proxy = event_loop.create_proxy();

    let wgpu_state = match block_on(WgpuState::new(event_loop.owned_display_handle())) {
        Ok(state) => Some(std::sync::Arc::new(state)),
        Err(err) => {
            tracing::warn!("GPU initialisation failed, falling back to software rendering: {err}");
            None
        }
    };

    handle_sigterm(proxy.clone());

    spawn_panic_thread(proxy.clone(), config.panic_button.clone());
    create_tray_icon(proxy.clone())?;

    let mut app = LewdwareApp::new(wgpu_state, proxy, config)?;
    event_loop.run_app(&mut app)?;

    Ok(())
}
