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
    /// Where this monitor sits in the desktop's coordinate space, in the same (physical) units as
    /// `width`/`height`. Only so the config app can draw the arrangement to scale; the engine
    /// re-reads it fresh at spawn time and never trusts what was stored.
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    /// So the config app can label a region in the logical pixels a mode would see, rather than
    /// the physical ones above.
    #[serde(default = "default_scale_factor")]
    pub scale_factor: f64,
}

fn default_scale_factor() -> f64 {
    1.0
}

/// The sub-area of one monitor that popups are confined to, as fractions of that monitor
/// (`0.0..=1.0`, origin top-left).
///
/// Fractions rather than pixels, for the same reason this module exists at all: the config app and
/// the engine measure the same display differently (see above), and the probe reports physical
/// pixels while the engine places windows in logical ones. A stored pixel rectangle would have to
/// mean one of those, and would be wrong in the other -- as well as breaking the first time the
/// user changed resolution or scale. A fraction means the same thing to both.
///
/// The engine applies this by *shrinking the monitor*: `lewdware::monitor::Monitor` reports the
/// region's size, and the region's origin is added to the window's absolute position. So a mode
/// sees the region as the whole monitor -- `{ percent = 50 }` is the region's centre, a random
/// position lands inside it, and clamping clamps to its edges -- with no mode-side changes. It
/// governs popups only; the wallpaper is still the whole screen.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonitorRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The whole monitor: what every monitor without an entry in `AppConfig::monitor_regions` gets.
impl Default for MonitorRegion {
    fn default() -> Self {
        Self::FULL
    }
}

/// The smallest region the engine will act on, in logical pixels.
///
/// A region is dragged out by hand, so an accidental one-pixel rectangle is a real possibility --
/// and a monitor that small has no room for even a decorated empty window, so every popup would
/// pile up on its origin. Rounding a too-small region up is the recoverable failure; the user can
/// see what happened and drag it bigger.
pub const MIN_REGION_SIZE: u32 = 100;

/// A [`MonitorRegion`] resolved against a monitor's logical size: concrete logical pixels, ready
/// to offset a window position by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedRegion {
    /// Offset from the monitor's top-left corner.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl MonitorRegion {
    pub const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    /// Whether this region leaves the monitor alone.
    pub fn is_full(&self) -> bool {
        *self == Self::FULL
    }

    /// Resolve against a monitor's **logical** size -- the units the engine positions windows in.
    ///
    /// Total function by construction: a region that is inverted, out of range, degenerate or
    /// outright NaN resolves to something drawable rather than failing. Config written by a newer
    /// version, or hand-edited, cannot stop a session from starting.
    pub fn resolve(&self, monitor_width: u32, monitor_height: u32) -> ResolvedRegion {
        let (x, width) = resolve_axis(self.x, self.width, monitor_width);
        let (y, height) = resolve_axis(self.y, self.height, monitor_height);

        ResolvedRegion {
            x,
            y,
            width,
            height,
        }
    }
}

/// One axis of [`MonitorRegion::resolve`]. Returns an offset and an extent that are always within
/// the monitor, and an extent that is always at least [`MIN_REGION_SIZE`] -- unless the monitor
/// itself is smaller, in which case the monitor wins.
fn resolve_axis(offset: f64, extent: f64, total: u32) -> (i32, u32) {
    if !offset.is_finite() || !extent.is_finite() {
        return (0, total);
    }

    let min = MIN_REGION_SIZE.min(total);

    let size = ((extent.clamp(0.0, 1.0) * total as f64).round() as u32).clamp(min, total);
    let position = ((offset.clamp(0.0, 1.0) * total as f64).round() as i32)
        .clamp(0, total.saturating_sub(size) as i32);

    (position, size)
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
            x: 1920,
            y: -200,
            scale_factor: 2.0,
        }];

        let encoded = serde_json::to_string(&monitors).unwrap();
        let decoded: Vec<MonitorInfo> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, monitors);
    }

    /// The geometry fields were added after the first release. A mismatched pair of binaries (a
    /// config app running against an older engine on `PATH`) must still list monitors -- it just
    /// draws them in a row rather than in their real arrangement.
    #[test]
    fn monitor_info_without_geometry_still_decodes() {
        let json = r#"[{"id":"eDP-1","name":"eDP-1","width":1920,"height":1080,"primary":true}]"#;

        let decoded: Vec<MonitorInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(decoded[0].x, 0);
        assert_eq!(decoded[0].y, 0);
        assert_eq!(decoded[0].scale_factor, 1.0);
    }
}

#[cfg(test)]
mod region_tests {
    use super::*;

    #[test]
    fn the_default_region_is_the_whole_monitor() {
        let region = MonitorRegion::default();
        assert!(region.is_full());

        assert_eq!(
            region.resolve(1920, 1080),
            ResolvedRegion {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn fractions_resolve_to_logical_pixels() {
        let region = MonitorRegion {
            x: 0.5,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        };

        assert_eq!(
            region.resolve(1920, 1080),
            ResolvedRegion {
                x: 960,
                y: 0,
                width: 960,
                height: 1080
            }
        );
    }

    /// A region dragged out by hand can be a sliver. It gets rounded up rather than producing a
    /// monitor no window fits on.
    #[test]
    fn a_sliver_is_widened_to_the_minimum() {
        let region = MonitorRegion {
            x: 0.0,
            y: 0.0,
            width: 0.001,
            height: 0.0,
        };

        let resolved = region.resolve(1920, 1080);
        assert_eq!(resolved.width, MIN_REGION_SIZE);
        assert_eq!(resolved.height, MIN_REGION_SIZE);
    }

    /// ...but never past the monitor itself, which would put the region's far edge off-screen.
    #[test]
    fn a_monitor_smaller_than_the_minimum_is_used_whole() {
        let region = MonitorRegion {
            x: 0.0,
            y: 0.0,
            width: 0.1,
            height: 0.1,
        };

        assert_eq!(
            region.resolve(80, 60),
            ResolvedRegion {
                x: 0,
                y: 0,
                width: 80,
                height: 60
            }
        );
    }

    /// The rounded-up minimum must come out of the offset, not hang off the right-hand edge.
    #[test]
    fn a_region_is_pushed_back_inside_the_monitor() {
        let region = MonitorRegion {
            x: 1.0,
            y: 1.0,
            width: 0.25,
            height: 0.25,
        };

        let resolved = region.resolve(1000, 800);
        assert_eq!(resolved.x, 750);
        assert_eq!(resolved.y, 600);
        assert_eq!(resolved.x as u32 + resolved.width, 1000);
        assert_eq!(resolved.y as u32 + resolved.height, 800);
    }

    /// Hand-edited or newer-version config must not be able to stop a session starting.
    #[test]
    fn nonsense_resolves_to_the_whole_monitor() {
        for region in [
            MonitorRegion {
                x: f64::NAN,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            MonitorRegion {
                x: 0.0,
                y: 0.0,
                width: f64::INFINITY,
                height: f64::NEG_INFINITY,
            },
        ] {
            let resolved = region.resolve(1920, 1080);
            assert_eq!(
                (resolved.x, resolved.y),
                (0, 0),
                "{region:?} should fall back to the whole monitor"
            );
        }
    }

    /// Negative and inverted rectangles clamp instead of underflowing.
    #[test]
    fn a_negative_region_clamps() {
        let region = MonitorRegion {
            x: -2.0,
            y: -0.5,
            width: -1.0,
            height: 3.0,
        };

        let resolved = region.resolve(1920, 1080);
        assert_eq!(resolved.x, 0);
        assert_eq!(resolved.y, 0);
        assert_eq!(resolved.width, MIN_REGION_SIZE);
        assert_eq!(resolved.height, 1080);
    }
}
