use std::{collections::HashSet, thread};

use anyhow::Result;
use rand::{random_range, seq::IndexedRandom};
use shared::user_config::{Key, Modifiers};
use winit::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event_loop::{ActiveEventLoop, EventLoopProxy},
    monitor::MonitorHandle,
    window::{WindowAttributes, WindowLevel},
};

use crate::{
    app::UserEvent,
    lua::{Anchor, Coord, SpawnWindowOpts, WindowProps},
    monitor::Monitors,
};

#[cfg(not(target_os = "linux"))]
pub fn create_tray_icon(event_loop_proxy: EventLoopProxy<UserEvent>) -> Result<()> {
    use tray_icon::{
        TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem},
    };

    let tray_menu = Menu::with_items(&[&MenuItem::new("Panic", true, None)])?;

    TrayIconBuilder::new()
        .with_tooltip("Lewdware")
        .with_menu(Box::new(tray_menu))
        .build()?;

    MenuEvent::set_event_handler(Some(move || {
        let _ = event_loop_proxy.send_event(UserEvent::Exit);
    }));

    Ok(())
}

/// Spawn a thread that will listen for the panic key being pressed, and send
/// [UserEvent::PanicButtonPressed] to the event loop.
pub fn spawn_panic_thread(event_loop_proxy: EventLoopProxy<UserEvent>, target_key: Key) {
    println!("Spawning panic thread");
    thread::spawn(move || {
        let rdev_key = match key_to_rdev(&target_key) {
            Some(x) => x,
            None => {
                eprintln!("Key cannot be matched: {:?}", target_key.code);
                return;
            }
        };

        let mut keys = HashSet::new();

        if let Err(err) = rdev::listen(move |event| {
            if let rdev::EventType::KeyPress(key) = event.event_type {
                keys.insert(key);

                if key == rdev_key {
                    let modifiers = rdev_keys_to_modifiers(&keys);

                    if modifier_matches(&modifiers, &target_key.modifiers)
                        && let Err(err) = event_loop_proxy.send_event(UserEvent::Exit)
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

fn modifier_matches(x: &Modifiers, pattern: &Modifiers) -> bool {
    if pattern.alt && !x.alt {
        return false;
    }
    if pattern.shift && !x.shift {
        return false;
    }

    if !pattern.ctrl && !pattern.meta {
        return !x.ctrl && !x.meta;
    }

    if pattern.ctrl && !x.ctrl {
        return false;
    }
    if pattern.meta && !x.meta {
        return false;
    }

    true
}

/// Extract the modifiers from a set of keys
fn rdev_keys_to_modifiers<'a>(keys: impl IntoIterator<Item = &'a rdev::Key>) -> Modifiers {
    let mut modifiers = Modifiers::default();

    for key in keys.into_iter() {
        match key {
            rdev::Key::Alt => {
                modifiers.alt = true;
            }
            rdev::Key::ControlLeft | rdev::Key::ControlRight => {
                modifiers.ctrl = true;
            }
            rdev::Key::MetaLeft | rdev::Key::MetaRight => {
                modifiers.meta = true;
            }
            rdev::Key::ShiftLeft | rdev::Key::ShiftRight => {
                modifiers.shift = true;
            }
            _ => {}
        }
    }

    modifiers
}

/// When registering a panic button, we get given a string (the key code, as recognized by the
/// browser), which we need to turn into an [rdev::Key] in order to be able to listen for the key
/// properly.
pub fn key_to_rdev(key: &Key) -> Option<rdev::Key> {
    // https://developer.mozilla.org/en-US/docs/Web/API/UI_Events/Keyboard_event_code_values
    match key.code.as_str() {
        "Escape" => Some(rdev::Key::Escape),
        "Digit0" => Some(rdev::Key::Num0),
        "Digit1" => Some(rdev::Key::Num1),
        "Digit2" => Some(rdev::Key::Num2),
        "Digit3" => Some(rdev::Key::Num3),
        "Digit4" => Some(rdev::Key::Num4),
        "Digit5" => Some(rdev::Key::Num5),
        "Digit6" => Some(rdev::Key::Num6),
        "Digit7" => Some(rdev::Key::Num7),
        "Digit8" => Some(rdev::Key::Num8),
        "Digit9" => Some(rdev::Key::Num9),
        "Minus" => Some(rdev::Key::Minus),
        "Equal" => Some(rdev::Key::Equal),
        "Backspace" => Some(rdev::Key::Backspace),
        "Tab" => Some(rdev::Key::Tab),
        "KeyA" => Some(rdev::Key::KeyA),
        "KeyB" => Some(rdev::Key::KeyB),
        "KeyC" => Some(rdev::Key::KeyC),
        "KeyD" => Some(rdev::Key::KeyD),
        "KeyE" => Some(rdev::Key::KeyE),
        "KeyF" => Some(rdev::Key::KeyF),
        "KeyG" => Some(rdev::Key::KeyG),
        "KeyH" => Some(rdev::Key::KeyH),
        "KeyI" => Some(rdev::Key::KeyI),
        "KeyJ" => Some(rdev::Key::KeyJ),
        "KeyK" => Some(rdev::Key::KeyK),
        "KeyL" => Some(rdev::Key::KeyL),
        "KeyM" => Some(rdev::Key::KeyM),
        "KeyN" => Some(rdev::Key::KeyN),
        "KeyO" => Some(rdev::Key::KeyO),
        "KeyP" => Some(rdev::Key::KeyP),
        "KeyQ" => Some(rdev::Key::KeyQ),
        "KeyR" => Some(rdev::Key::KeyR),
        "KeyS" => Some(rdev::Key::KeyS),
        "KeyT" => Some(rdev::Key::KeyT),
        "KeyU" => Some(rdev::Key::KeyU),
        "KeyV" => Some(rdev::Key::KeyV),
        "KeyW" => Some(rdev::Key::KeyW),
        "KeyX" => Some(rdev::Key::KeyX),
        "KeyY" => Some(rdev::Key::KeyY),
        "KeyZ" => Some(rdev::Key::KeyZ),
        "BracketLeft" => Some(rdev::Key::LeftBracket),
        "BracketRight" => Some(rdev::Key::RightBracket),
        "Enter" => Some(rdev::Key::Return),
        "ControlLeft" => Some(rdev::Key::ControlLeft),
        "ControlRight" => Some(rdev::Key::ControlRight),
        "Semicolon" => Some(rdev::Key::SemiColon),
        "Quote" => Some(rdev::Key::Quote),
        "Backquote" => Some(rdev::Key::BackQuote),
        "ShiftLeft" => Some(rdev::Key::ShiftLeft),
        "ShiftRight" => Some(rdev::Key::ShiftRight),
        "Backslash" => Some(rdev::Key::BackSlash),
        "Comma" => Some(rdev::Key::Comma),
        "Period" => Some(rdev::Key::Dot),
        "Slash" => Some(rdev::Key::Slash),
        "AltLeft" => Some(rdev::Key::Alt),
        "Space" => Some(rdev::Key::Space),
        "CapsLock" => Some(rdev::Key::CapsLock),
        "F1" => Some(rdev::Key::F1),
        "F2" => Some(rdev::Key::F2),
        "F3" => Some(rdev::Key::F3),
        "F4" => Some(rdev::Key::F4),
        "F5" => Some(rdev::Key::F5),
        "F6" => Some(rdev::Key::F6),
        "F7" => Some(rdev::Key::F7),
        "F8" => Some(rdev::Key::F8),
        "F9" => Some(rdev::Key::F9),
        "F10" => Some(rdev::Key::F10),
        "F11" => Some(rdev::Key::F11),
        "F12" => Some(rdev::Key::F12),
        "Pause" => Some(rdev::Key::Pause),
        "ScrollLock" => Some(rdev::Key::ScrollLock),
        "Numpad0" => Some(rdev::Key::Kp0),
        "Numpad1" => Some(rdev::Key::Kp1),
        "Numpad2" => Some(rdev::Key::Kp2),
        "Numpad3" => Some(rdev::Key::Kp3),
        "Numpad4" => Some(rdev::Key::Kp4),
        "Numpad5" => Some(rdev::Key::Kp5),
        "Numpad6" => Some(rdev::Key::Kp6),
        "Numpad7" => Some(rdev::Key::Kp7),
        "Numpad8" => Some(rdev::Key::Kp8),
        "Numpad9" => Some(rdev::Key::Kp9),
        "NumpadAdd" => Some(rdev::Key::KpPlus),
        "NumpadSubtract" => Some(rdev::Key::KpMinus),
        "NumpadMultiply" => Some(rdev::Key::KpMultiply),
        "NumpadDivide" => Some(rdev::Key::KpDivide),
        "NumpadEnter" => Some(rdev::Key::KpReturn),
        "IntlBackslash" => Some(rdev::Key::IntlBackslash),
        "PrintScreen" => Some(rdev::Key::PrintScreen),
        "NumLock" => Some(rdev::Key::NumLock),
        "PageUp" => Some(rdev::Key::PageUp),
        "PageDown" => Some(rdev::Key::PageDown),
        "ArrowLeft" => Some(rdev::Key::LeftArrow),
        "ArrowRight" => Some(rdev::Key::RightArrow),
        "ArrowUp" => Some(rdev::Key::UpArrow),
        "ArrowDown" => Some(rdev::Key::DownArrow),
        "Home" => Some(rdev::Key::Home),
        "End" => Some(rdev::Key::End),
        "Insert" => Some(rdev::Key::Insert),
        "Delete" => Some(rdev::Key::Delete),
        "MetaLeft" => Some(rdev::Key::MetaLeft),
        "MetaRight" => Some(rdev::Key::MetaRight),
        _ => None,
    }
}

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

pub fn calculate_media_popup_size(
    width: Option<Coord>,
    height: Option<Coord>,
    media_width: u32,
    media_height: u32,
    monitor_width: u32,
    monitor_height: u32,
) -> (u32, u32) {
    let width = width.map(|width| width.to_pixels(monitor_width));
    let height = height.map(|height| height.to_pixels(monitor_height));

    match (width, height) {
        (None, None) => default_media_popup_size(
            media_width,
            media_height,
            monitor_width,
            monitor_height,
        ),
        (None, Some(height)) => (
            ((height as f64 / media_height as f64) * media_width as f64).round() as u32,
            height,
        ),
        (Some(width), None) => (
            width,
            ((width as f64 / media_width as f64) * media_height as f64).round() as u32,
        ),
        (Some(width), Some(height)) => (width, height),
    }
}

fn default_media_popup_size(
    media_width: u32,
    media_height: u32,
    monitor_width: u32,
    monitor_height: u32,
) -> (u32, u32) {
    let width = media_width as f64;
    let height = media_height as f64;

    let max_width_scale = (monitor_width as f64 * 0.3) / width;
    let max_height_scale = (monitor_height as f64 * 0.5) / height;

    let scale = max_width_scale.min(max_height_scale).min(1.0);

    let width = (width * scale).round();
    let height = (height * scale).round();

    (width as u32, height as u32)
}

pub fn resolve_coord(x: u32, anchor: &Anchor, window_size: u32, offset_start: u32, offset_end: u32) -> u32 {
    match anchor {
        Anchor::TopLeft => x,
        Anchor::Center => x + offset_start + (window_size / 2),
        Anchor::BottomRight => x + offset_start + window_size + offset_end,
    }
}
