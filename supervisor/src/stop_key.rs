use std::collections::HashSet;
use std::thread;

use shared::ipc::control::StopShortcutStatus;
use shared::user_config::{Key, Modifiers};
use tokio::sync::{mpsc, oneshot, watch};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(test, target_os = "linux"))]
mod portal_tests;

use crate::control::ControlMessage;

/// A request to show the desktop's settings for the stop shortcut, answered once the desktop has
/// taken it (or refused).
pub type ConfigureRequest = oneshot::Sender<anyhow::Result<()>>;

/// Owns the configured key for the lifetime of the supervisor. Native listeners read the
/// latest value on each event; compositor backends close and replace their registration.
#[derive(Clone)]
pub struct StopShortcut {
    key: watch::Sender<Key>,
    /// To the compositor backend, which holds the session the desktop's settings belong to. Closed
    /// on backends that bind the key themselves.
    configure: mpsc::Sender<ConfigureRequest>,
}

impl StopShortcut {
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self {
            key: watch::channel(shared::user_config::AppConfig::default().stop_button).0,
            configure: mpsc::channel(1).0,
        }
    }

    pub fn spawn(control_tx: mpsc::Sender<ControlMessage>, key: Key) -> Self {
        let (key_tx, key_rx) = watch::channel(key);
        let (configure_tx, configure_rx) = mpsc::channel(4);
        #[cfg(target_os = "linux")]
        if linux::is_wayland() {
            tokio::spawn(linux::run(control_tx, key_rx, configure_rx));
            return Self {
                key: key_tx,
                configure: configure_tx,
            };
        }
        drop(configure_rx);
        spawn_native_thread(control_tx, key_rx);
        Self {
            key: key_tx,
            configure: configure_tx,
        }
    }

    /// Asks the desktop to show its settings for the stop shortcut (GlobalShortcuts portal
    /// version 2). Fails on backends where Lewdware binds the key itself.
    pub async fn configure(&self) -> anyhow::Result<()> {
        use anyhow::Context as _;
        let (reply, answer) = oneshot::channel();
        self.configure
            .send(reply)
            .await
            .map_err(|_| anyhow::anyhow!("This desktop has no shortcut settings to open"))?;
        tokio::time::timeout(std::time::Duration::from_secs(10), answer)
            .await
            .context("The desktop didn't answer")?
            .map_err(|_| anyhow::anyhow!("The desktop shortcut session closed"))?
    }

    pub fn reload(&self, key: Key) {
        self.key.send_if_modified(|current| {
            if *current == key {
                false
            } else {
                *current = key;
                true
            }
        });
    }

    pub fn refresh(&self) {
        let key = self.key.borrow().clone();
        self.key.send_replace(key);
    }

    pub async fn setup(&self) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            let key = self.key.borrow().clone();
            linux::setup_gnome(&key).await?;
            // The key may have changed while gsettings was running. Refresh the latest value,
            // rather than writing the captured key back over a concurrent config reload.
            self.refresh();
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        anyhow::bail!("Desktop shortcut setup is only used by GNOME on Wayland")
    }
}

/// Reports that the shortcut needs Input Monitoring, then waits for the user to grant it and
/// tells the control loop, which restarts the supervisor to apply it. Never returns: the restart
/// replaces this process.
///
/// A process's answer never changes once it has asked, in either direction, and nor does what its
/// listener hears: macOS applies a grant from the next launch. So each check is a new process
/// (`lewdware-supervisor check-input-monitoring`). As our child, it runs as the same app.
#[cfg(target_os = "macos")]
fn wait_for_input_monitoring(control_tx: &mpsc::Sender<ControlMessage>) -> ! {
    tracing::warn!("Stop shortcut waiting for Input Monitoring permission");
    let _ = control_tx.blocking_send(ControlMessage::StopShortcutChanged(
        StopShortcutStatus::PermissionRequired {
            message: "Needs Input Monitoring permission".into(),
        },
    ));
    while !granted_to_a_new_process() {
        thread::sleep(std::time::Duration::from_secs(2));
    }
    tracing::info!("Input Monitoring permission granted");
    let _ = control_tx.blocking_send(ControlMessage::InputMonitoringGranted);
    loop {
        thread::park();
    }
}

#[cfg(target_os = "macos")]
fn granted_to_a_new_process() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let mut check = std::process::Command::new(exe);
    // Run as Lewdware.app's main executable: keep it in the supervisor role.
    if std::env::args().nth(1).as_deref() == Some(shared::binaries::SUPERVISOR_ROLE_FLAG) {
        check.arg(shared::binaries::SUPERVISOR_ROLE_FLAG);
    }
    check
        .arg("check-input-monitoring")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Spawn a thread that listens for the stop shortcut being pressed, sending
/// [ControlMessage::StopKeyPressed] on match. Moved here wholesale from the engine (which used
/// to own this thread directly) -- the supervisor is now the sole owner of the shortcut, alive
/// even if a session hangs.
fn spawn_native_thread(control_tx: mpsc::Sender<ControlMessage>, mut target: watch::Receiver<Key>) {
    let runtime = tokio::runtime::Handle::current();
    tracing::info!("Spawning stop-shortcut thread");
    thread::spawn(move || {
        tracing::info!("Stop-shortcut thread started");

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
                Ok(()) => tracing::info!("Stop-shortcut thread priority set to TIME_CRITICAL"),
                Err(e) => tracing::error!("Failed to set stop-shortcut thread priority: {e}"),
            }
        }

        loop {
            target.borrow_and_update();

            // macOS: without Input Monitoring the listener below never hears a key, yet would
            // report `Listening`. Say what's missing instead, so the config app can ask for it.
            #[cfg(target_os = "macos")]
            if !shared::input_monitoring::granted() {
                wait_for_input_monitoring(&control_tx);
            }

            let _ = control_tx.blocking_send(ControlMessage::StopShortcutChanged(
                StopShortcutStatus::Listening {
                    backend: if cfg!(target_os = "linux") {
                        "X11"
                    } else {
                        "Native"
                    }
                    .into(),
                },
            ));

            let mut keys = HashSet::new();
            let event_target = target.clone();

            let event_tx = control_tx.clone();
            if let Err(err) = rdev::listen(move |event| {
                if let rdev::EventType::KeyPress(key) = event.event_type {
                    keys.insert(key);

                    let target_key = event_target.borrow().clone();
                    if Some(key) == key_to_rdev(&target_key) {
                        let modifiers = rdev_keys_to_modifiers(&keys);

                        if modifier_matches(&modifiers, &target_key.modifiers)
                            && let Err(err) = event_tx.blocking_send(ControlMessage::StopKeyPressed)
                        {
                            tracing::error!("Could not send stop-shortcut event: {}", err);
                        }
                    }
                } else if let rdev::EventType::KeyRelease(key) = event.event_type {
                    keys.remove(&key);
                }
            }) {
                let _ = control_tx.blocking_send(ControlMessage::StopShortcutChanged(
                    StopShortcutStatus::Failed {
                        message: format!("Stop shortcut listener failed: {err:?}"),
                    },
                ));
                #[cfg(target_vendor = "apple")]
                tracing::error!(
                    "Stop shortcut listener failed (this usually means accessibility permission was not granted): {:?}",
                    err
                );
                #[cfg(not(target_vendor = "apple"))]
                tracing::error!("Stop shortcut listener failed: {:?}", err);
            }
            // rdev returned (usually a missing display/permission). A retry or key edit may
            // recover it; wait for that request rather than repeatedly opening the display.
            if runtime.block_on(target.changed()).is_err() {
                return;
            }
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

/// When registering a stop shortcut, we get given a string (the key code, as recognized by the
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
