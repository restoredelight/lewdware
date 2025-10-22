#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use futures_lite::future::block_on;
use shared::user_config::load_config;
use winit::event_loop::EventLoop;

use crate::{app::ChaosApp, egui::WgpuState, utils::spawn_panic_thread};

mod app;
mod audio;
mod buffer;
mod egui;
mod media;
mod transition;
mod utils;
mod video;
mod window;

fn main() -> Result<()> {
    let config = load_config()?;

    let wgpu_state = block_on(WgpuState::new());

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

    let mut app = ChaosApp::new(wgpu_state, proxy, config)?;
    event_loop.run_app(&mut app)?;

    Ok(())
}
