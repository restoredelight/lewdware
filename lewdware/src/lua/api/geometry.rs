//! Where and how big a popup is: coordinates, anchors, spawn regions, and the window
//! options every popup shares.

use mlua::ExternalResult;
use serde::{Deserialize, Serialize};

use rand::seq::IndexedRandom;

// The Lua layer's own names for these: they describe a colour and an alignment, and a *theme*
// needs to as well, so they live with the themes and are re-exported here.
pub use shared::theme::Color;

use super::*;
use crate::{
    app::EventPoster,
    lua::request::RequestSender,
    monitor::Monitor,
    utils::{
        calculate_media_popup_size, calculate_text_popup_size, random_position, random_position_in,
    },
    window::{AppearanceChoice, ChromeDefaults, Theme, ThemeChoice},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Coord {
    Pixel(i32),
    Percent { percent: f64 },
}

impl Coord {
    /// Returns a signed pixel value. Callers are responsible for clamping to valid bounds.
    pub fn to_pixels(&self, total_size: u32) -> i32 {
        match self {
            Coord::Pixel(x) => *x,
            Coord::Percent { percent } => ((percent * total_size as f64) / 100.0).round() as i32,
        }
    }
}

/// A font size: either a literal point size, or a percentage of the monitor's (logical) height.
/// Kept separate from `Coord` so literal values keep `f32` precision rather than rounding to a
/// whole pixel.
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(untagged)]
pub enum FontSize {
    Value(f32),
    Percent { percent: f64 },
}

impl FontSize {
    /// Resolve to a concrete point size. `monitor_height` (logical) is the basis for `Percent`
    /// and is ignored for `Value`.
    pub fn to_pixels(self, monitor_height: u32) -> f32 {
        match self {
            FontSize::Value(size) => size,
            FontSize::Percent { percent } => (percent / 100.0) as f32 * monitor_height as f32,
        }
    }
}

/// The full 3x3 grid: horizontal (left/center/right) and vertical (top/center/bottom) are
/// independent, so e.g. `TopCenter` centers the window horizontally on the given x while leaving
/// y unadjusted -- see `resolve_x`/`resolve_y`, which each only look at the matching axis.
#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    #[serde(rename = "top-left")]
    #[default]
    TopLeft,
    #[serde(rename = "top-center")]
    TopCenter,
    #[serde(rename = "top-right")]
    TopRight,
    #[serde(rename = "center-left")]
    CenterLeft,
    #[serde(rename = "center")]
    Center,
    #[serde(rename = "center-right")]
    CenterRight,
    #[serde(rename = "bottom-left")]
    BottomLeft,
    #[serde(rename = "bottom-center")]
    BottomCenter,
    #[serde(rename = "bottom-right")]
    BottomRight,
}

/// A rectangle of the monitor a randomly placed window is confined to, as fractions of the usable
/// area (so `{ x = 0, y = 0, width = 0.5, height = 1 }` is its left half).
///
/// Only consulted for an axis the mode gave no coordinate for: `x` and `y` say exactly where the
/// window goes, and a region is a statement about where it goes *at random*. A window too big for
/// the region is centred on it rather than pinned to a corner (see [`random_position_in`]), which
/// is what lets a zero-size region name a single placement.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct SpawnRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl SpawnRegion {
    /// This region's horizontal span in pixels on a monitor of `width`, as `(start, span)`.
    pub fn horizontal(&self, width: u32) -> (f64, f64) {
        (self.x * width as f64, self.width * width as f64)
    }

    /// This region's vertical span in pixels on a monitor of `height`. See [`Self::horizontal`].
    pub fn vertical(&self, height: u32) -> (f64, f64) {
        (self.y * height as f64, self.height * height as f64)
    }
}

/// Where a coordinate falls relative to the window's edge along one axis.
#[derive(Clone, Copy)]
pub(super) enum AxisAnchor {
    Start,
    Center,
    End,
}

impl AxisAnchor {
    fn resolve(self, coord: i32, size: u32) -> i32 {
        match self {
            Self::Start => coord,
            Self::Center => coord - (size / 2) as i32,
            Self::End => coord - size as i32,
        }
    }
}

impl Anchor {
    fn horizontal(self) -> AxisAnchor {
        match self {
            Self::TopLeft | Self::CenterLeft | Self::BottomLeft => AxisAnchor::Start,
            Self::TopCenter | Self::Center | Self::BottomCenter => AxisAnchor::Center,
            Self::TopRight | Self::CenterRight | Self::BottomRight => AxisAnchor::End,
        }
    }

    fn vertical(self) -> AxisAnchor {
        match self {
            Self::TopLeft | Self::TopCenter | Self::TopRight => AxisAnchor::Start,
            Self::CenterLeft | Self::Center | Self::CenterRight => AxisAnchor::Center,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight => AxisAnchor::End,
        }
    }

    pub fn resolve_x(&self, coord: i32, width: u32) -> i32 {
        self.horizontal().resolve(coord, width)
    }

    pub fn resolve_y(&self, coord: i32, height: u32) -> i32 {
        self.vertical().resolve(coord, height)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SpawnWindowOpts {
    pub x: Option<Coord>,
    pub y: Option<Coord>,
    pub width: Option<Coord>,
    pub height: Option<Coord>,
    /// Multiplies the size the engine would otherwise pick for media, *before* the monitor caps
    /// apply -- so a scaled-up popup is still at most a third of the screen wide and half of it
    /// tall. That is the point of putting this here rather than leaving callers to compute a
    /// `width`: an explicit `width` is taken literally, and a caller scaling one from the media's
    /// own dimensions would be stepping around the caps rather than through them.
    ///
    /// Ignored when `width` or `height` is given (they already say the size exactly), and for
    /// windows that are not sized from media.
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub anchor: Anchor,
    /// Confines a *randomly placed* window to part of the monitor. Ignored on an axis `x` or `y`
    /// already pins exactly. See [`SpawnRegion`].
    #[serde(default)]
    pub region: Option<SpawnRegion>,
    pub monitor: Option<Monitor>,
    #[serde(default = "return_true")]
    pub decorations: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default = "return_true")]
    pub closeable: bool,
    /// Whether the window can be moved by dragging its custom header.
    #[serde(default)]
    pub draggable: bool,
    #[serde(default)]
    pub opacity: Option<f32>,
    #[serde(default)]
    pub transparent: Option<bool>,
    #[serde(default)]
    pub background_color: Option<Color>,
    #[serde(default)]
    pub click_through: bool,
    #[serde(default = "return_true")]
    pub clamp: bool,
    /// Which named look to draw the window's chrome and widgets with.
    ///
    /// `None` — the mode said nothing — means the user's own setting (`AppConfig::theme`), which
    /// is what almost every window should use. It is an `Option` rather than a defaulted enum
    /// precisely so that "said nothing" stays distinguishable from an explicit choice: naming a
    /// theme is also how a mode pins the metrics its layout arithmetic depends on, and that has
    /// to keep working even when the user's setting happens to be the same value.
    #[serde(default)]
    pub theme: Option<ThemeChoice>,
    /// Which palette that look is drawn in. `None` means the user's own setting
    /// (`AppConfig::appearance`), for the same reason as `theme`.
    #[serde(default)]
    pub appearance: Option<AppearanceChoice>,
}

impl Default for SpawnWindowOpts {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: None,
            height: None,
            scale: None,
            anchor: Anchor::default(),
            region: None,
            monitor: None,
            decorations: true,
            title: None,
            closeable: true,
            draggable: false,
            opacity: None,
            transparent: None,
            background_color: None,
            click_through: false,
            clamp: true,
            theme: None,
            appearance: None,
        }
    }
}

/// A window's size, before it's known whether media dimensions, an explicit default, or a text
/// measurement should drive it.
pub enum WindowSizeBehaviour {
    ResizeWithMedia {
        width: u32,
        height: u32,
    },
    UseDefaults {
        width: u32,
        height: u32,
    },
    MeasureText {
        text: String,
        font: TextFont,
        font_size: FontSize,
        outline_width: f32,
    },
}

/// A `SpawnWindowOpts` resolved against a monitor snapshot. Sizes, anchor math and clamping are
/// pure computation, so they happen here, on the Lua thread -- but a monitor's *current* absolute
/// position can only be read on the main thread (winit's `MonitorHandle` isn't `Send`, and can
/// change between this resolution and the window's actual, possibly much later, creation if
/// media is still decoding). So this carries `monitor_id` rather than a resolved position;
/// `LewdwareApp::finalize_window_opts` does that last step, fresh, right before the real window
/// is built.
#[derive(Debug, Clone)]
pub struct PopupSpawnOpts {
    pub monitor_id: u64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub outer_width: u32,
    pub outer_height: u32,
    pub gpu: bool,
    pub transparent: bool,
    pub force_opaque: bool,
    pub opacity: f32,
    pub click_through: bool,
    pub decorations: bool,
    pub title: Option<String>,
    pub closeable: bool,
    pub draggable: bool,
    pub background_color: Option<Color>,
    /// The palette the look is drawn in, still *unresolved*: `auto` depends on runtime state only
    /// the main thread can read, so — like `monitor_id` above — this carries the request and
    /// `WindowState::new` resolves it once the window exists. Safe to defer precisely because
    /// appearance never changes the metrics the sizing below is computed from.
    pub appearance: AppearanceChoice,
    /// The named look this window's chrome and widgets are drawn with, already concrete: the
    /// mode's own choice where it made one, and the user's setting otherwise. Its metrics are
    /// what the sizing above is computed from, which is why — unlike `appearance` — this one
    /// cannot be deferred past window creation.
    pub theme: Theme,
}

impl PopupSpawnOpts {
    /// Fails only on a window that could never be drawn: an explicitly requested width or height
    /// that resolves to zero or less. Sizes the engine derives itself (from media dimensions or a
    /// text measurement) are not checked here — those are the engine's own business, and
    /// `Header::new` clamps a degenerate one rather than failing a spawn the mode did not ask for.
    pub fn resolve(
        spawn_opts: SpawnWindowOpts,
        size_behaviour: WindowSizeBehaviour,
        monitor: &Monitor,
        chrome: ChromeDefaults,
        gpu_available: bool,
        mut gpu: bool,
        transparent: bool,
    ) -> Result<Self, InvalidWindowSize> {
        if !gpu_available {
            gpu = false;
        }

        // Before anything else: a zero-sized window has no content area, no drawable header, and
        // no way for the user to close it. Better a Lua error at the call site than a window that
        // exists but cannot be seen or dismissed.
        check_requested_size(&spawn_opts, monitor)?;
        let transparent = transparent && gpu;
        let force_opaque = spawn_opts.transparent == Some(false);

        let monitor_width = monitor.width;
        let monitor_height = monitor.height;

        let (width, height) = match size_behaviour {
            WindowSizeBehaviour::ResizeWithMedia { width, height } => calculate_media_popup_size(
                spawn_opts.width,
                spawn_opts.height,
                spawn_opts.scale,
                width,
                height,
                monitor_width,
                monitor_height,
            ),
            WindowSizeBehaviour::UseDefaults { width, height } => (
                spawn_opts
                    .width
                    .map(|w| w.to_pixels(monitor_width).max(0) as u32)
                    .unwrap_or(width),
                spawn_opts
                    .height
                    .map(|h| h.to_pixels(monitor_height).max(0) as u32)
                    .unwrap_or(height),
            ),
            WindowSizeBehaviour::MeasureText {
                text,
                font,
                font_size,
                outline_width,
            } => calculate_text_popup_size(
                spawn_opts.width.clone(),
                spawn_opts.height.clone(),
                &text,
                font,
                font_size.to_pixels(monitor_height),
                outline_width,
                monitor_width,
                monitor_height,
            ),
        };

        // Resolved here, once, so that a `native` alias becomes a concrete look before anything
        // downstream — window sizing included — ever asks what platform it is on. A mode that
        // named no theme gets the user's, which is the whole point of that setting.
        let theme = spawn_opts.theme.unwrap_or(chrome.theme).resolve();

        let (mut outer_width, mut outer_height) = (width, height);
        if spawn_opts.decorations {
            let (padding_x, padding_y) = theme.metrics().outer_padding();
            outer_width += padding_x;
            outer_height += padding_y;
        }

        let region = spawn_opts.region;
        let x: i32 = {
            let v = spawn_opts
                .x
                .map(|c| {
                    spawn_opts
                        .anchor
                        .resolve_x(c.to_pixels(monitor_width), outer_width)
                })
                .unwrap_or_else(|| match region {
                    Some(region) => {
                        let (start, span) = region.horizontal(monitor_width);
                        random_position_in(outer_width, start, span)
                    }
                    None => random_position(outer_width, monitor_width),
                });
            if spawn_opts.clamp {
                v.max(0)
                    .min(monitor_width.saturating_sub(outer_width) as i32)
            } else {
                v
            }
        };
        let y: i32 = {
            let v = spawn_opts
                .y
                .map(|c| {
                    spawn_opts
                        .anchor
                        .resolve_y(c.to_pixels(monitor_height), outer_height)
                })
                .unwrap_or_else(|| match region {
                    Some(region) => {
                        let (start, span) = region.vertical(monitor_height);
                        random_position_in(outer_height, start, span)
                    }
                    None => random_position(outer_height, monitor_height),
                });
            if spawn_opts.clamp {
                v.max(0)
                    .min(monitor_height.saturating_sub(outer_height) as i32)
            } else {
                v
            }
        };

        Ok(Self {
            monitor_id: monitor.id,
            x,
            y,
            width,
            height,
            outer_width,
            outer_height,
            gpu,
            transparent,
            force_opaque,
            opacity: spawn_opts.opacity.unwrap_or(1.0),
            click_through: spawn_opts.click_through,
            decorations: spawn_opts.decorations,
            title: spawn_opts.title,
            closeable: spawn_opts.closeable,
            draggable: spawn_opts.draggable,
            background_color: spawn_opts.background_color,
            appearance: spawn_opts.appearance.unwrap_or(chrome.appearance),
            theme,
        })
    }
}

/// A window size a mode asked for that cannot be drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidWindowSize {
    /// `"width"` or `"height"`.
    pub axis: &'static str,
    pub pixels: i32,
}

impl std::fmt::Display for InvalidWindowSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "window {} must be greater than zero, got {}",
            self.axis, self.pixels
        )
    }
}

impl std::error::Error for InvalidWindowSize {}

/// Reject an explicitly requested size that resolves to zero or less, including a percentage that
/// rounds down to nothing on a small monitor.
pub(super) fn check_requested_size(
    spawn_opts: &SpawnWindowOpts,
    monitor: &Monitor,
) -> Result<(), InvalidWindowSize> {
    for (axis, coord, total) in [
        ("width", &spawn_opts.width, monitor.width),
        ("height", &spawn_opts.height, monitor.height),
    ] {
        if let Some(coord) = coord {
            let pixels = coord.to_pixels(total);
            if pixels <= 0 {
                return Err(InvalidWindowSize { axis, pixels });
            }
        }
    }

    Ok(())
}

/// Pick the monitor a popup should spawn on: the one the mode explicitly asked for (from an
/// earlier `lewdware.monitors.list()`/`primary()` call), or a random one from a fresh
/// `list_monitors()` snapshot. Either way, only `.id` is trusted past this point -- see
/// `PopupSpawnOpts`'s doc comment.
pub(super) fn resolve_monitor<T: EventPoster>(
    spawn_opts: &SpawnWindowOpts,
    request_sender: &RequestSender<T>,
) -> mlua::Result<Monitor> {
    match &spawn_opts.monitor {
        Some(monitor) => Ok(monitor.clone()),
        None => {
            let monitors = request_sender.list_monitors().into_lua_err()?;
            let mut rng = rand::rng();
            monitors
                .choose(&mut rng)
                .cloned()
                .ok_or_else(|| mlua::Error::runtime("no monitors available"))
        }
    }
}
