#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{env::args_os, path::PathBuf};

use anyhow::{Context, Result};
use pollster::block_on;
use shared::user_config::{Mode, load_config};
use winit::event_loop::EventLoop;

use crate::{app::ChaosApp, egui::WgpuState, utils::spawn_panic_thread};

mod app;
mod audio;
mod buffer;
mod egui;
mod error;
mod header;
mod lua;
mod media;
mod monitor;
mod transition;
mod utils;
mod video;
mod window;

fn main() -> Result<()> {
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
        config.mode = Mode::File { path: mode_path, mode };
    }

    let wgpu_state = block_on(WgpuState::new())?;

    let mut event_loop_builder = EventLoop::with_user_event();

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;

        // Wayland doesn't support a bunch of stuff we need (e.g. setting the position of windows).
        // So we use XWayland instead.
        event_loop_builder.with_x11();
    }

    let event_loop = event_loop_builder.build()?;
    let proxy = event_loop.create_proxy();

    spawn_panic_thread(proxy.clone(), config.panic_button.clone());
    #[cfg(not(target_os = "linux"))]
    {
        use crate::utils::create_tray_icon;
        create_tray_icon(proxy.clone())?;
    }

    let mut app = ChaosApp::new(wgpu_state, proxy, config)?;
    event_loop.run_app(&mut app)?;

    Ok(())
}
