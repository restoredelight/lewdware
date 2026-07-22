//! Monitor identity, shared between the engine and the config app.
//!
//! These two processes see different monitors. The engine forces winit onto X11/XWayland
//! (`lewdware/src/main.rs`: Wayland can't position windows), while the config app is a native
//! Wayland Tauri app. On a fractional-scaled Wayland session the same display comes back as:
//!
//! | | name | size | scale |
//! |---|---|---|---|
//! | tao, native Wayland | `eDP-1-0x1405` | 5487x3087 | 3 |
//! | winit, XWayland | `eDP-1` | 3840x2160 | 2.09375 |
//!
//! Neither the name nor the geometry matches, so `AppConfig::disabled_monitors` written by the
//! config app never matched what the engine compared it against, and disabling a monitor silently
//! did nothing. Rather than trying to reconcile two views, the config app asks the engine for its
//! list (`lewdware-engine --list-monitors`) and stores those identities verbatim -- the engine is
//! the process that will act on them, so it is the only view that can't be wrong.

use serde::{Deserialize, Serialize};

/// One monitor, as the engine sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonitorInfo {
    /// What `AppConfig::disabled_monitors` stores. Whatever winit calls the monitor, falling back
    /// to its geometry when winit reports no name at all.
    pub id: String,
    /// For display. Same as `id` unless there was no name to use.
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

/// The argument that makes the engine print its monitor list as JSON and exit.
pub const LIST_MONITORS_FLAG: &str = "--list-monitors";

/// A stable-ish identity for a monitor winit won't name.
///
/// X11 geometry syntax, so it's recognisable if a user ever reads their `config.json`.
pub fn geometry_id(width: u32, height: u32, x: i32, y: i32) -> String {
    format!("{width}x{height}+{x}+{y}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_ids_use_x11_syntax() {
        assert_eq!(geometry_id(3840, 2160, 0, 0), "3840x2160+0+0");
        assert_eq!(geometry_id(1920, 1080, 3840, -200), "1920x1080+3840+-200");
    }

    /// The config app deserialises exactly what the engine prints, so the wire shape is load
    /// bearing across two separate binaries.
    #[test]
    fn monitor_info_round_trips_as_json() {
        let monitors = vec![MonitorInfo {
            id: "eDP-1".to_owned(),
            name: "eDP-1".to_owned(),
            width: 3840,
            height: 2160,
            primary: true,
        }];

        let encoded = serde_json::to_string(&monitors).unwrap();
        let decoded: Vec<MonitorInfo> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, monitors);
    }
}
