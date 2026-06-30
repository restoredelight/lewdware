use std::{collections::HashSet, thread};

use anyhow::Result;
use shared::user_config::{Key, Modifiers};
use winit::event_loop::EventLoopProxy;

use crate::{app::UserEvent, lua::Coord};

// Create a tray icon that can be used to close the program
#[cfg(not(target_os = "linux"))]
pub fn create_tray_icon(event_loop_proxy: EventLoopProxy<UserEvent>) -> Result<()> {
    use tray_icon::{
        Icon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem},
    };

    let tray_menu = Menu::with_items(&[&MenuItem::new("Panic", true, None)])?;

    #[cfg(target_os = "windows")]
    let icon_bytes = include_bytes!("../assets/tray-windows.ico");
    #[cfg(not(target_os = "windows"))]
    let icon_bytes = include_bytes!("../assets/tray.png");

    let img = image::load_from_memory(icon_bytes)?.into_rgba8();
    let icon = Icon::from_rgba(img.to_vec(), img.width(), img.height())?;

    let tray_icon = TrayIconBuilder::new()
        .with_tooltip("Lewdware")
        .with_menu(Box::new(tray_menu))
        .with_icon(icon)
        .with_icon_as_template(cfg!(target_vendor = "apple"))
        .build()?;

    // The TrayIcon must be kept alive for the icon to remain visible. Since it should
    // live for the entire application lifetime, we intentionally leak it here.
    std::mem::forget(tray_icon);

    MenuEvent::set_event_handler(Some(move |_| {
        let _ = event_loop_proxy.send_event(UserEvent::Exit);
    }));

    Ok(())
}

#[cfg(target_os = "linux")]
pub fn create_tray_icon(event_loop_proxy: EventLoopProxy<UserEvent>) -> Result<()> {
    use ksni::{Tray, TrayService, menu::StandardItem};

    struct LewdwareTray {
        proxy: EventLoopProxy<UserEvent>,
        icon_theme_path: String,
    }

    impl Tray for LewdwareTray {
        fn title(&self) -> String {
            "Lewdware".into()
        }
        fn icon_name(&self) -> String {
            "lewdware-symbolic".into()
        }
        fn icon_theme_path(&self) -> String {
            self.icon_theme_path.clone()
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            vec![
                StandardItem {
                    label: "Panic".into(),
                    activate: Box::new(|this: &mut Self| {
                        let _ = this.proxy.send_event(UserEvent::Exit);
                    }),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    TrayService::new(LewdwareTray {
        proxy: event_loop_proxy,
        icon_theme_path: install_symbolic_icon().unwrap_or_default(),
    })
    .spawn();
    Ok(())
}

// Writes the symbolic SVG to the user's hicolor icon theme and returns the
// theme root path for the SNI IconThemePath property.
#[cfg(target_os = "linux")]
fn install_symbolic_icon() -> Option<String> {
    let svg = include_bytes!("../../assets/tray-symbolic.svg");
    let apps_dir = dirs::data_local_dir()?.join("icons/hicolor/scalable/apps");
    std::fs::create_dir_all(&apps_dir).ok()?;
    std::fs::write(apps_dir.join("lewdware-symbolic.svg"), svg).ok()?;
    dirs::data_local_dir().map(|p| p.join("icons").to_string_lossy().into_owned())
}

/// Spawn a thread that will listen for the panic key being pressed, and send
/// [UserEvent::PanicButtonPressed] to the event loop.
pub fn spawn_panic_thread(event_loop_proxy: EventLoopProxy<UserEvent>, target_key: Key) {
    tracing::info!("Spawning panic thread");
    thread::spawn(move || {
        tracing::info!("Panic thread started");

        // On Windows, rdev installs a WH_KEYBOARD_LL hook whose callback is called as a
        // sent message to this thread. Windows will silently remove the hook if the
        // callback doesn't return within LowLevelHooksTimeout (typically 300ms). Under
        // heavy CPU load (many video windows), this thread can be starved long enough to
        // hit that timeout. Raising to TIME_CRITICAL ensures it gets scheduled in time.
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

                    if modifier_matches(&modifiers, &target_key.modifiers) {
                        if let Err(err) = event_loop_proxy.send_event(UserEvent::Exit) {
                            tracing::error!("Could not send panic button event: {}", err);
                        }
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

pub fn calculate_media_popup_size(
    width: Option<Coord>,
    height: Option<Coord>,
    media_width: u32,
    media_height: u32,
    monitor_width: u32,
    monitor_height: u32,
) -> (u32, u32) {
    let width = width.map(|width| width.to_pixels(monitor_width).max(0) as u32);
    let height = height.map(|height| height.to_pixels(monitor_height).max(0) as u32);

    match (width, height) {
        (None, None) => {
            default_media_popup_size(media_width, media_height, monitor_width, monitor_height)
        }
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

    let max_width_scale = (monitor_width as f64 / 3.0) / width;
    let max_height_scale = (monitor_height as f64 / 2.0) / height;

    let scale = max_width_scale.min(max_height_scale).min(1.0);

    let width = (width * scale).round();
    let height = (height * scale).round();

    (width as u32, height as u32)
}

// Silence the "Secure coding is automatically enabled for restorable state" warning by explicitly
// opting in. winit doesn't do this itself, so we inject the method into its app delegate class.
//
// Must be called after EventLoop::build() (which creates the NSApplication and sets its delegate)
// and before run_app() (when the method is first queried).
#[cfg(target_vendor = "apple")]
pub fn opt_in_secure_restorable_state() {
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject, Bool, Sel},
        sel,
    };
    use std::ffi::c_char;

    unsafe extern "C" {
        fn class_addMethod(
            cls: *const AnyClass,
            name: Sel,
            imp: unsafe extern "C" fn(),
            types: *const c_char,
        ) -> bool;
    }

    unsafe extern "C" fn returns_yes(_: *mut AnyObject, _: Sel) -> Bool {
        Bool::YES
    }

    unsafe {
        let app_cls = AnyClass::get(c"NSApplication").expect("NSApplication");
        let app: *mut AnyObject = msg_send![app_cls, sharedApplication];
        let delegate: *mut AnyObject = msg_send![app, delegate];
        if delegate.is_null() {
            return;
        }
        class_addMethod(
            (*delegate).class(),
            sel!(applicationSupportsSecureRestorableState:),
            std::mem::transmute::<
                unsafe extern "C" fn(*mut AnyObject, Sel) -> Bool,
                unsafe extern "C" fn(),
            >(returns_yes),
            c"c@:".as_ptr(),
        );
    }
}

// Makes sure we gracefully shut down on SIGTERM
#[cfg(unix)]
pub fn handle_sigterm(event_loop_proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        if let Ok(mut signals) =
            signal_hook::iterator::Signals::new(&[signal_hook::consts::signal::SIGTERM])
        {
            for _sig in signals.forever() {
                let _ = event_loop_proxy.send_event(UserEvent::Exit);
                break;
            }
        }
    });
}

#[cfg(not(unix))]
pub fn handle_sigterm(_: EventLoopProxy<UserEvent>) {}

// lewdware opens lots of file descriptors on Unix systems (Windows, media files, GPU stuff),
// so increase the file descriptor limit so we don't crash with lots of windows open.
#[cfg(unix)]
pub fn raise_fd_limit() {
    unsafe {
        let mut rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) == 0 {
            let target_limit = 65535;
            if rlim.rlim_cur < target_limit {
                rlim.rlim_cur = std::cmp::min(target_limit, rlim.rlim_max);
                if libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) != 0 {
                    tracing::error!("Failed to raise file descriptor limit");
                }
            }
        }
    }
}

#[cfg(not(unix))]
pub fn raise_fd_limit() {}
