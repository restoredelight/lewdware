use std::collections::HashSet;
use std::thread;

use shared::user_config::{Key, Modifiers};
use tokio::sync::mpsc;

use crate::control::ControlMessage;

/// Spawn a thread that listens for the panic key being pressed, sending
/// [ControlMessage::PanicKeyPressed] on match. Moved here wholesale from the engine (which used
/// to own this thread directly) -- the supervisor is now the sole owner of the panic key, alive
/// even if a session hangs.
pub fn spawn_panic_thread(control_tx: mpsc::Sender<ControlMessage>, target_key: Key) {
    tracing::info!("Spawning panic thread");
    thread::spawn(move || {
        tracing::info!("Panic thread started");

        // On Windows, rdev installs a WH_KEYBOARD_LL hook whose callback is called as a
        // sent message to this thread. Windows will silently remove the hook if the
        // callback doesn't return within LowLevelHooksTimeout (typically 300ms). Under
        // heavy CPU load, this thread can be starved long enough to hit that timeout.
        // Raising to TIME_CRITICAL ensures it gets scheduled in time.
        #[cfg(target_os = "windows")]
        unsafe {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
            };
            match SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) {
                Ok(()) => tracing::info!("Panic thread priority set to TIME_CRITICAL"),
                Err(e) => tracing::error!("Failed to set panic thread priority: {e}"),
            }
        }

        let rdev_key = match key_to_rdev(&target_key) {
            Some(x) => x,
            None => {
                tracing::error!("Key cannot be matched: {:?}", target_key.code);
                return;
            }
        };

        tracing::info!(
            "Panic listener starting: watching for {:?} with modifiers {:?}",
            rdev_key,
            target_key.modifiers
        );

        let mut keys = HashSet::new();

        if let Err(err) = rdev::listen(move |event| {
            if let rdev::EventType::KeyPress(key) = event.event_type {
                keys.insert(key);

                if key == rdev_key {
                    let modifiers = rdev_keys_to_modifiers(&keys);

                    if modifier_matches(&modifiers, &target_key.modifiers)
                        && let Err(err) = control_tx.blocking_send(ControlMessage::PanicKeyPressed)
                    {
                        tracing::error!("Could not send panic button event: {}", err);
                    }
                }
            } else if let rdev::EventType::KeyRelease(key) = event.event_type {
                keys.remove(&key);
            }
        }) {
            #[cfg(target_vendor = "apple")]
            tracing::error!(
                "Panic key listener failed (this usually means accessibility permission was not granted): {:?}",
                err
            );
            #[cfg(not(target_vendor = "apple"))]
            tracing::error!("Panic key listener failed: {:?}", err);
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
