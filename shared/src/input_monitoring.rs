//! macOS's Input Monitoring permission, which the supervisor's stop shortcut needs to hear keys
//! pressed in other apps. Without it macOS creates the listener but never delivers a key to it,
//! so the shortcut does nothing and nothing says why.
//!
//! It's granted to the supervisor's helper app, "Lewdware Supervisor" (`deploy/macos/
//! build_installer.sh`), so only the supervisor's own calls mean anything: the config app asks
//! it (`Request::RequestInputMonitoring`) and shows the status it reports. A grant belongs to one
//! build: the installer resets it, and ad-hoc signed updates wouldn't match it anyway.
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

/// Asks for the permission. The first time per login, macOS shows its prompt, which also adds this
/// app to the Input Monitoring list; otherwise it just answers whether it's granted.
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
