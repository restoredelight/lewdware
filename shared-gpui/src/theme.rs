//! The design tokens from `shared-ui/DESIGN.md`, as gpui colours, plus the bundled UI fonts and
//! the gpui-component theme they are fed into.
//!
//! `shared-ui/styles/tokens.css` is still the source of truth while the Tauri app exists. Change a
//! value there and here together.

use std::borrow::Cow;

use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{px, rgb, rgba, App, Hsla, Pixels};

pub const BG: u32 = 0x0a0809;
pub const SURFACE: u32 = 0x141113;
pub const SURFACE_RAISED: u32 = 0x1f191c;
pub const BORDER: u32 = 0x2c2529;
pub const BORDER_STRONG: u32 = 0x463c41;
pub const ACCENT: u32 = 0xc70036;
pub const ACCENT_HOVER: u32 = 0xe8003f;
pub const TEXT: u32 = 0xf5f2f3;
pub const MUTED: u32 = 0x9e9398;
pub const FOCUS: u32 = 0xff4d7d;
pub const WARNING: u32 = 0xd6b271;
pub const WARNING_BG: u32 = 0x292112;
pub const WARNING_BORDER: u32 = 0x5c4b24;
pub const DANGER: u32 = 0xff6b6b;
pub const DANGER_BG: u32 = 0x321313;
pub const DANGER_BORDER: u32 = 0x7a2929;

pub const RADIUS_SM: Pixels = px(2.);
pub const RADIUS_MD: Pixels = px(3.);
pub const CONTROL_COMPACT: Pixels = px(32.);
pub const CONTROL_NORMAL: Pixels = px(36.);

pub const FONT_SANS: &str = "Inter";
pub const FONT_MONO: &str = "JetBrains Mono";

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// `--ui-shadow-pop`'s colour: black at 55%.
pub fn shadow_pop() -> Hsla {
    rgba(0x0000008c).into()
}

const FONTS: [&[u8]; 6] = [
    // Static instances cut from the bundled variable Inter (`shared-ui/fonts`), because gpui's
    // text system ignores a variable font's weight axis: every weight drew as 400. The 650 page
    // title resolves to the nearest of these.
    //
    // Latin only for now. The latin-ext subset shares the family name, so loading both would let
    // the text system pick the subset that has no basic Latin. Anything outside Latin falls back
    // to system fonts.
    include_bytes!("../assets/fonts/Inter-latin-400.ttf"),
    include_bytes!("../assets/fonts/Inter-latin-500.ttf"),
    include_bytes!("../assets/fonts/Inter-latin-600.ttf"),
    include_bytes!("../assets/fonts/Inter-latin-700.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
];

/// Registers extra font files with gpui's text system, logging rather than failing: a missing
/// face falls back to a system font.
pub fn add_fonts(cx: &mut App, fonts: &[&'static [u8]]) {
    if let Err(error) = cx
        .text_system()
        .add_fonts(fonts.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
    {
        tracing::error!("failed to load bundled fonts: {error}");
    }
}

/// The gpui-component palette mapped onto our tokens. Only the keys the components we use read
/// are worth setting; everything else inherits from the default dark theme.
fn theme_json() -> String {
    let hex = |v: u32| format!("#{v:06x}");
    let colors = serde_json::json!({
        "background": hex(BG),
        "foreground": hex(TEXT),
        "border": hex(BORDER),
        "input.border": hex(BORDER_STRONG),
        "ring": hex(FOCUS),
        "caret": hex(TEXT),
        "selection.background": "#c7003655",
        "muted.background": hex(SURFACE_RAISED),
        "muted.foreground": hex(MUTED),
        "accent.background": hex(SURFACE_RAISED),
        "accent.foreground": hex(TEXT),
        "primary.background": hex(ACCENT),
        "primary.hover.background": hex(ACCENT_HOVER),
        "primary.active.background": hex(ACCENT),
        "primary.foreground": hex(TEXT),
        "secondary.background": hex(SURFACE),
        "secondary.hover.background": hex(SURFACE_RAISED),
        "secondary.active.background": hex(SURFACE_RAISED),
        "secondary.foreground": hex(TEXT),
        "danger.background": hex(DANGER_BG),
        "danger.foreground": hex(DANGER),
        "warning.background": hex(WARNING_BG),
        "warning.foreground": hex(WARNING),
        // No success colour in this palette: successful outcomes are silent.
        "success.background": hex(SURFACE_RAISED),
        "success.foreground": hex(MUTED),
        "info.background": hex(SURFACE_RAISED),
        "info.foreground": hex(MUTED),
        "popover.background": hex(SURFACE),
        "popover.foreground": hex(TEXT),
        "list.background": hex(SURFACE),
        "list.hover.background": hex(SURFACE_RAISED),
        "list.active.background": hex(SURFACE_RAISED),
        "list.active.border": hex(ACCENT_HOVER),
        "switch.background": hex(BORDER_STRONG),
        "slider.bar.background": hex(ACCENT),
        "slider.thumb.background": hex(TEXT),
        "scrollbar.background": "#00000000",
        "scrollbar.thumb.background": hex(BORDER_STRONG),
        "scrollbar.thumb.hover.background": hex(MUTED),
        "tab.active.background": hex(SURFACE_RAISED),
        "tab.active.foreground": hex(TEXT),
        "tab.background": "#00000000",
        "tab.foreground": hex(MUTED),
        "tab_bar.background": hex(SURFACE),
        "sidebar.background": hex(SURFACE),
        "sidebar.border": hex(BORDER),
        "sidebar.foreground": hex(TEXT),
        "title_bar.background": hex(SURFACE),
        "title_bar.border": hex(BORDER),
        "window.border": hex(BORDER),
        "overlay": "#0a0809b3",
    });
    serde_json::json!({
        "name": "Lewdware",
        "themes": [{
            "name": "Lewdware",
            "mode": "dark",
            "font.family": FONT_SANS,
            // The rem, which gpui spacing is measured in. 16 matches Tailwind, so `p_8` is the
            // 32px it was in the webview. Text sizes are set explicitly (13px default, on the shell).
            "font.size": 16.0,
            "mono_font.family": FONT_MONO,
            "mono_font.size": 11.5,
            "radius": 3,
            "radius.lg": 4,
            "shadow": false,
            "colors": colors,
        }]
    })
    .to_string()
}

pub fn install(cx: &mut App) {
    add_fonts(cx, &FONTS);

    let registry = ThemeRegistry::global_mut(cx);
    registry
        .load_themes_from_str(&theme_json())
        .expect("the built-in theme is valid");
    let config = registry
        .themes()
        .get("Lewdware")
        .expect("the theme was just registered")
        .clone();

    // Both apps are dark-only (DESIGN.md), so the system appearance is never followed.
    Theme::update(cx, |theme| theme.dark_theme = config);
    Theme::change(ThemeMode::Dark, None, cx);
    // A scrollbar that only appears mid-scroll hides that there's more to scroll to.
    Theme::set_scrollbar_mode(gpui_kit::component::scroll::ScrollbarMode::Always, cx);
}
