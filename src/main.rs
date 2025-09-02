use std::{sync::{atomic::{AtomicBool, Ordering}, Arc}, thread};

use anyhow::Result;
use winit::event_loop::EventLoopBuilder;

use crate::{app::ChaosApp, media::MediaManager};

mod app;
mod window;
mod media;
mod video;
mod config;
mod egui;
mod audio;
mod buffer;

fn main() -> Result<()> {
    let config = config::load_config("config.json");

    let media_manager = MediaManager::open("pack.md")?;

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    thread::spawn(move || {
        rdev::listen(move |event| {
            if event.event_type == rdev::EventType::KeyPress(rdev::Key::Escape) {
                running_clone.store(false, Ordering::Relaxed);
            }
        }).unwrap();
    });

    let mut event_loop_builder = EventLoopBuilder::default();

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;

        event_loop_builder.with_x11();
    }

    let event_loop = event_loop_builder.build()?;
    let mut app = ChaosApp::new(media_manager, config, running);
    event_loop.run_app(&mut app)?;
    Ok(())
}
