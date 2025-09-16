use std::{fs::{self, File}, io::{Read, Seek, SeekFrom}, path::Path, thread};

use anyhow::{Result, anyhow};
use pack_format::{config::Metadata, Header, HEADER_SIZE};
use rand::{random_range, seq::IndexedRandom};
use winit::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event_loop::{ActiveEventLoop, EventLoopProxy},
    monitor::MonitorHandle,
    window::{WindowAttributes, WindowLevel},
};

use crate::app::UserEvent;

pub struct WindowOpts {
    pub width: u32,
    pub height: u32,
    pub logical_size: bool,
    pub random_position: bool,
}

pub fn create_window(
    event_loop: &ActiveEventLoop,
    width: u32,
    height: u32,
    logical_size: bool,
) -> Result<winit::window::Window> {
    let monitor = random_monitor(event_loop);

    let position = if let Some(monitor) = monitor {
        let size = monitor.size();
        let monitor_position = monitor.position();
        let scale_factor = monitor.scale_factor();

        let (width, height) = if logical_size {
            let size = LogicalSize::new(width, height).to_physical(scale_factor);
            (size.width, size.height)
        } else {
            (width, height)
        };

        let position = random_window_position(width, height, size.width, size.height);

        PhysicalPosition::new(
            monitor_position.x as f32 + position.x,
            monitor_position.y as f32 + position.y,
        )
    } else {
        println!("Could not find a monitor, using default resolution");
        random_window_position(width, height, 1920, 1080)
    };

    let mut attrs = WindowAttributes::default()
        .with_title("Chaos Window")
        .with_position(position)
        .with_decorations(false)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_resizable(false);

    if logical_size {
        attrs = attrs.with_inner_size(LogicalSize::new(width, height));
    } else {
        attrs = attrs.with_inner_size(PhysicalSize::new(width, height));
    }

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::{WindowAttributesExtX11, WindowType};

        attrs = attrs.with_x11_window_type(vec![WindowType::Notification]);
    }

    #[cfg(target_os = "windows")]
    {
        attrs = attrs.with_skip_taskbar(true);
    }

    event_loop.create_window(attrs).map_err(|err| anyhow!(err))
}

pub fn spawn_panic_thread(event_loop_proxy: EventLoopProxy<UserEvent>) {
    thread::spawn(move || {
        if let Err(err) = rdev::listen(move |event| {
            if event.event_type == rdev::EventType::KeyPress(rdev::Key::Escape)
                && let Err(err) = event_loop_proxy.send_event(UserEvent::PanicButtonPressed)
            {
                eprintln!("Could not send panic button event: {}", err);
            }
        }) {
            eprintln!(
                "Error occurred while trying to setup panic listener: {:?}",
                err
            );
        }
    });
}

pub fn read_pack_metadata<F: Read + Seek>(mut file: F) -> Result<(Header, Metadata)> {
    let header = Header::read_from(&mut file)?;

    file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;

    let mut buf = vec![0u8; header.metadata_length as usize];
    file.read_exact(&mut buf)?;

    let metadata = Metadata::from_buf(&buf)?;

    Ok((header, metadata))
}

fn random_window_position(
    width: u32,
    height: u32,
    monitor_width: u32,
    monitor_height: u32,
) -> PhysicalPosition<f32> {
    let x = if monitor_width > width {
        random_range(0..=(monitor_width - width))
    } else {
        0
    };
    let y = if monitor_height > height {
        random_range(0..=(monitor_height - height))
    } else {
        0
    };

    PhysicalPosition::new(x as f32, y as f32)
}

fn random_monitor(event_loop: &ActiveEventLoop) -> Option<MonitorHandle> {
    let monitors: Vec<_> = event_loop.available_monitors().collect();

    let mut rng = rand::rng();
    monitors.choose(&mut rng).cloned()
}
