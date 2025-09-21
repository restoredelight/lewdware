#![windows_subsystem = "windows"]

use anyhow::Result;
use winit::event_loop::EventLoop;

use crate::app_switcher::AppSwitcher;

mod app;
mod app_switcher;
mod audio;
mod buffer;
mod config;
mod egui;
mod media;
mod transition;
mod utils;
mod video;
mod window;

fn main() -> Result<()> {
    let config = config::load_config()?;

    let mut event_loop_builder = EventLoop::with_user_event();

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;

        event_loop_builder.with_x11();
    }

    let event_loop = event_loop_builder.build()?;
    let mut app = AppSwitcher::new(&event_loop, config);
    event_loop.run_app(&mut app)?;

    Ok(())
}
