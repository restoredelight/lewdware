//! Since the engine always uses X11, its view of the monitors may be different from
//! the config app. 

use serde::{Deserialize, Serialize};

/// One monitor, as the engine sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    // The monitor's position
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default = "default_scale_factor")]
    pub scale_factor: f64,
}

fn default_scale_factor() -> f64 {
    1.0
}

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

/// A [`MonitorRegion`] resolved against a monitor's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedRegion {
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

/// Makes the engine print its monitor list as JSON and exit.
pub const LIST_MONITORS_FLAG: &str = "--list-monitors";

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
