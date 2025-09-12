use std::{sync::{atomic::{AtomicBool, Ordering}, Arc}, thread};

use anyhow::Result;
use winit::event_loop::EventLoop;

use crate::app::ChaosApp;

mod app;
mod window;
mod media;
mod video;
mod config;
mod egui;
mod audio;
mod buffer;
mod transition;

fn main() -> Result<()> {
    let config = config::load_config("config.json");

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    thread::spawn(move || {
        if let Err(err) = rdev::listen(move |event| {
            if event.event_type == rdev::EventType::KeyPress(rdev::Key::Escape) {
                running_clone.store(false, Ordering::Relaxed);
            }
        }) {
            eprintln!("Error occurred while trying to setup panic listener: {:?}", err);
        }
    });

    let mut event_loop_builder = EventLoop::with_user_event();

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;

        event_loop_builder.with_x11();
    }

    let event_loop = event_loop_builder.build()?;
    let mut app = ChaosApp::new(&event_loop, config, running)?;
    event_loop.run_app(&mut app)?;
    Ok(())
}
