//! macOS's Input Monitoring permission, which the supervisor's stop shortcut needs to hear keys
//! pressed in other apps. Without it macOS creates the listener but never delivers a key to it,
//! so the shortcut does nothing and nothing says why.
//!
//! macOS grants it to Lewdware.app, which the supervisor and the config app both run as, so the
//! config app's checks and requests apply to the supervisor too. A grant belongs to one build:
//! ad-hoc signed updates turn it off again.
//!
//! Elsewhere there's nothing to grant: [`granted`] is always true.

#[cfg(target_os = "macos")]
mod sys {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        pub fn CGPreflightListenEventAccess() -> bool;
        pub fn CGRequestListenEventAccess() -> bool;
    }
}

/// System Settings' Input Monitoring list.
pub const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent";

/// Whether this app may listen to keys pressed in other apps.
pub fn granted() -> bool {
    #[cfg(target_os = "macos")]
    // Safety: takes nothing, and only reads the permission.
    return unsafe { sys::CGPreflightListenEventAccess() };
    #[cfg(not(target_os = "macos"))]
    true
}

/// Asks for the permission. The first time, macOS shows its prompt, which also adds Lewdware to
/// the Input Monitoring list; after that it returns whether it's granted without asking. Call
/// it on the main thread.
pub fn request() -> bool {
    #[cfg(target_os = "macos")]
    // Safety: takes nothing.
    return unsafe { sys::CGRequestListenEventAccess() };
    #[cfg(not(target_os = "macos"))]
    true
}

/// Opens System Settings at the Input Monitoring list.
pub fn open_settings() -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(SETTINGS_URL)
        .spawn()?;
    Ok(())
}
