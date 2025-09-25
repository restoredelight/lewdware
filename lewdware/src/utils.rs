use std::{
    collections::HashSet,
    io::{Read, Seek, SeekFrom},
    thread,
};

use anyhow::{Result, anyhow};
use pack_format::{HEADER_SIZE, Header, config::Metadata};
use rand::{random_range, seq::IndexedRandom};
use winit::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event_loop::{ActiveEventLoop, EventLoopProxy},
    monitor::MonitorHandle,
    window::{WindowAttributes, WindowLevel},
};

use crate::app::UserEvent;

/// Spawn a window in a random position, on a random monitor.
///
/// * `logical_size`: Whether to interpret `width` and `height` as a logical or physical size.
///   Logical sizes will be scaled using the dpi, while physical sizes will not.
/// * `visible`: Whether to make the window visible initially.
pub fn create_window(
    event_loop: &ActiveEventLoop,
    width: u32,
    height: u32,
    logical_size: bool,
    visible: bool,
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
        .with_resizable(false)
        .with_visible(true);

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
        use winit::platform::windows::WindowAttributesExtWindows;

        attrs = attrs.with_skip_taskbar(true);
    }

    event_loop.create_window(attrs).map_err(|err| anyhow!(err))
}

/// Spawn a thread that will listen for the panic key being pressed, and send
/// [UserEvent::PanicButtonPressed] to the event loop.
pub fn spawn_panic_thread(
    event_loop_proxy: EventLoopProxy<UserEvent>,
    key: egui::Key,
    target_modifiers: egui::Modifiers,
) {
    thread::spawn(move || {
        let target_key = match egui_key_to_rdev(key) {
            Some(x) => x,
            None => {
                eprintln!("Key cannot be matched: {:?}", key);
                return;
            }
        };

        let mut keys = HashSet::new();

        if let Err(err) = rdev::listen(move |event| {
            if let rdev::EventType::KeyPress(key) = event.event_type {
                keys.insert(key);

                if key == target_key {
                    let modifiers = rdev_keys_to_modifiers(&keys);

                    if modifiers.matches_logically(target_modifiers)
                        && let Err(err) = event_loop_proxy.send_event(UserEvent::PanicButtonPressed)
                    {
                        eprintln!("Could not send panic button event: {}", err);
                    }
                }
            } else if let rdev::EventType::KeyRelease(key) = event.event_type {
                keys.remove(&key);
            }
        }) {
            eprintln!(
                "Error occurred while trying to setup panic listener: {:?}",
                err
            );
        }
    });
}

/// Extract the modifiers from a set of keys
fn rdev_keys_to_modifiers<'a>(
    keys: impl IntoIterator<Item = &'a rdev::Key>,
) -> egui::Modifiers {
    let mut modifiers = egui::Modifiers::NONE;

    for key in keys.into_iter() {
        match key {
            rdev::Key::Alt => {
                modifiers |= egui::Modifiers::ALT;
            }
            rdev::Key::ControlLeft | rdev::Key::ControlRight => {
                modifiers |= egui::Modifiers::CTRL;
                #[cfg(not(target_os = "macos"))]
                {
                    // On Windows/Linux, Ctrl is the command key
                    modifiers |= egui::Modifiers::COMMAND;
                }
            }
            rdev::Key::MetaLeft | rdev::Key::MetaRight => {
                #[cfg(target_os = "macos")]
                {
                    // On macOS, Meta is the Command key
                    modifiers |= egui::Modifiers::COMMAND;
                }
            }
            rdev::Key::ShiftLeft | rdev::Key::ShiftRight => {
                modifiers |= egui::Modifiers::SHIFT;
            }
            _ => {}
        }
    }

    modifiers
}

/// When registering a panic button, we get given an [egui::Key], which we need to turn into an
/// [rdev::Key] in order to be able to listen for the key properly. Some egui keys don't have rdev
/// equivalents (mainly because rdev only represents physical keys, while egui also represents
/// logical keys), in which case the function returns [None].
pub fn egui_key_to_rdev(key: egui::Key) -> Option<rdev::Key> {
    match key {
        egui::Key::ArrowDown => Some(rdev::Key::DownArrow),
        egui::Key::ArrowLeft => Some(rdev::Key::LeftArrow),
        egui::Key::ArrowRight => Some(rdev::Key::RightArrow),
        egui::Key::ArrowUp => Some(rdev::Key::UpArrow),
        egui::Key::Escape => Some(rdev::Key::Escape),
        egui::Key::Tab => Some(rdev::Key::Tab),
        egui::Key::Backspace => Some(rdev::Key::Backspace),
        egui::Key::Enter => Some(rdev::Key::Return),
        egui::Key::Space => Some(rdev::Key::Space),
        egui::Key::Insert => Some(rdev::Key::Insert),
        egui::Key::Delete => Some(rdev::Key::Delete),
        egui::Key::Home => Some(rdev::Key::Home),
        egui::Key::End => Some(rdev::Key::End),
        egui::Key::PageUp => Some(rdev::Key::PageUp),
        egui::Key::PageDown => Some(rdev::Key::PageDown),
        egui::Key::Comma => Some(rdev::Key::Comma),
        egui::Key::Backslash => Some(rdev::Key::BackSlash),
        egui::Key::Slash => Some(rdev::Key::Slash),
        egui::Key::OpenBracket => Some(rdev::Key::LeftBracket),
        egui::Key::CloseBracket => Some(rdev::Key::RightBracket),
        egui::Key::Minus => Some(rdev::Key::Minus),
        egui::Key::Period => Some(rdev::Key::Dot),
        egui::Key::Plus => Some(rdev::Key::KpPlus),
        egui::Key::Equals => Some(rdev::Key::Equal),
        egui::Key::Semicolon => Some(rdev::Key::SemiColon),
        egui::Key::Quote => Some(rdev::Key::Quote),
        egui::Key::Num0 => Some(rdev::Key::Num0),
        egui::Key::Num1 => Some(rdev::Key::Num1),
        egui::Key::Num2 => Some(rdev::Key::Num2),
        egui::Key::Num3 => Some(rdev::Key::Num3),
        egui::Key::Num4 => Some(rdev::Key::Num4),
        egui::Key::Num5 => Some(rdev::Key::Num5),
        egui::Key::Num6 => Some(rdev::Key::Num6),
        egui::Key::Num7 => Some(rdev::Key::Num7),
        egui::Key::Num8 => Some(rdev::Key::Num8),
        egui::Key::Num9 => Some(rdev::Key::Num9),
        egui::Key::A => Some(rdev::Key::KeyA),
        egui::Key::B => Some(rdev::Key::KeyB),
        egui::Key::C => Some(rdev::Key::KeyC),
        egui::Key::D => Some(rdev::Key::KeyD),
        egui::Key::E => Some(rdev::Key::KeyE),
        egui::Key::F => Some(rdev::Key::KeyF),
        egui::Key::G => Some(rdev::Key::KeyG),
        egui::Key::H => Some(rdev::Key::KeyH),
        egui::Key::I => Some(rdev::Key::KeyI),
        egui::Key::J => Some(rdev::Key::KeyJ),
        egui::Key::K => Some(rdev::Key::KeyK),
        egui::Key::L => Some(rdev::Key::KeyL),
        egui::Key::M => Some(rdev::Key::KeyM),
        egui::Key::N => Some(rdev::Key::KeyN),
        egui::Key::O => Some(rdev::Key::KeyO),
        egui::Key::P => Some(rdev::Key::KeyP),
        egui::Key::Q => Some(rdev::Key::KeyQ),
        egui::Key::R => Some(rdev::Key::KeyR),
        egui::Key::S => Some(rdev::Key::KeyS),
        egui::Key::T => Some(rdev::Key::KeyT),
        egui::Key::U => Some(rdev::Key::KeyU),
        egui::Key::V => Some(rdev::Key::KeyV),
        egui::Key::W => Some(rdev::Key::KeyW),
        egui::Key::X => Some(rdev::Key::KeyX),
        egui::Key::Y => Some(rdev::Key::KeyY),
        egui::Key::Z => Some(rdev::Key::KeyZ),
        egui::Key::F1 => Some(rdev::Key::F1),
        egui::Key::F2 => Some(rdev::Key::F2),
        egui::Key::F3 => Some(rdev::Key::F3),
        egui::Key::F4 => Some(rdev::Key::F4),
        egui::Key::F5 => Some(rdev::Key::F5),
        egui::Key::F6 => Some(rdev::Key::F6),
        egui::Key::F7 => Some(rdev::Key::F7),
        egui::Key::F8 => Some(rdev::Key::F8),
        egui::Key::F9 => Some(rdev::Key::F9),
        egui::Key::F10 => Some(rdev::Key::F10),
        egui::Key::F11 => Some(rdev::Key::F11),
        egui::Key::F12 => Some(rdev::Key::F12),
        _ => None,
    }
}

/// Read the header and metadata of a pack file.
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
