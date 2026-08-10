//! Window themes: the catalogue of named looks, and the whole of what each one *is*.
//!
//! Every value a theme is made of lives here as plain data — the chrome half (border, title bar,
//! buttons) and the widget half (a dialog's controls) alike. Nothing here knows how to *draw*:
//! the engine projects [`Widgets`] onto an `egui::Style` and paints [`Chrome`] with tiny-skia
//! (see `lewdware/src/window/theme.rs`), while `config/` renders the same values as HTML to show
//! a user what they are choosing.
//!
//! Two very different renderers reading one description is the whole reason this lives in
//! `shared` rather than in the engine, where it began. See `design/window-themes.md`.

use serde::{Deserialize, Serialize};

mod color;

pub use color::{Color, Face, TextAlign};

/// One selectable window look.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemeInfo {
    /// The name stored in `AppConfig::theme` and passed to the Lua API. Stable: it is written to
    /// the user's config, so renaming one silently resets whoever had chosen it.
    pub name: &'static str,
    /// What a user sees. Says what the look *is* ("Windows 95"), not what it is called internally
    /// ("redmond"), because the internal names are deliberately not trademarks and mean nothing
    /// to anyone outside this repository.
    pub label: &'static str,
    /// Whether this look has a real dark palette. `false` means it draws light whatever the
    /// appearance setting says — Mac OS 9 never had a dark mode, and inventing one would be
    /// making up a look rather than reproducing one. The UI says so rather than leaving a user to
    /// wonder why nothing changed.
    ///
    /// Meaningless for an alias, whose answer depends on the machine it resolves on: ask
    /// [`Theme::supports_dark`] of whatever [`ThemeChoice::resolve`] returned instead.
    pub supports_dark: bool,
    /// Whether the look is an alias resolved per machine (`native`, `native-retro`) rather than a
    /// single concrete appearance. Worth surfacing: a preview of one can only ever show what it
    /// resolves to *here*.
    pub is_alias: bool,
}

/// Every theme, in the order they are worth offering: the two "match my system" aliases first,
/// since that is what most people want, then the concrete looks roughly newest-to-oldest.
pub const THEMES: &[ThemeInfo] = &[
    ThemeInfo {
        name: "native",
        label: "Match my system",
        supports_dark: true,
        is_alias: true,
    },
    ThemeInfo {
        name: "native-retro",
        label: "Match my system (retro)",
        supports_dark: true,
        is_alias: true,
    },
    ThemeInfo {
        name: "plain",
        label: "Plain",
        supports_dark: true,
        is_alias: false,
    },
    ThemeInfo {
        name: "fluent",
        label: "Windows 11",
        supports_dark: true,
        is_alias: false,
    },
    ThemeInfo {
        name: "redmond",
        label: "Windows 95",
        supports_dark: true,
        is_alias: false,
    },
    ThemeInfo {
        name: "aqua",
        label: "macOS",
        supports_dark: true,
        is_alias: false,
    },
    ThemeInfo {
        name: "adwaita",
        label: "GNOME",
        supports_dark: true,
        is_alias: false,
    },
    ThemeInfo {
        name: "breeze",
        label: "KDE Plasma",
        supports_dark: true,
        is_alias: false,
    },
    ThemeInfo {
        name: "platinum",
        label: "Mac OS 9",
        supports_dark: false,
        is_alias: false,
    },
    ThemeInfo {
        name: "cde",
        label: "CDE / Motif",
        supports_dark: true,
        is_alias: false,
    },
];

/// One selectable palette.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppearanceInfo {
    pub name: &'static str,
    pub label: &'static str,
}

pub const APPEARANCES: &[AppearanceInfo] = &[
    AppearanceInfo {
        name: "auto",
        label: "Match my system",
    },
    AppearanceInfo {
        name: "light",
        label: "Light",
    },
    AppearanceInfo {
        name: "dark",
        label: "Dark",
    },
];

/// Looks up a theme by the name stored in the user's config. `None` for a name from a newer
/// Lewdware, or a hand-edited config -- callers fall back rather than failing.
pub fn theme(name: &str) -> Option<&'static ThemeInfo> {
    THEMES.iter().find(|t| t.name == name)
}

pub fn appearance(name: &str) -> Option<&'static AppearanceInfo> {
    APPEARANCES.iter().find(|a| a.name == name)
}

/// One concrete, drawable look.
///
/// Every variant is a specific appearance, so nothing here branches on the platform — that is
/// [`ThemeChoice::resolve`]'s job. See `design/window-themes.md` for why the catalogue is a flat
/// list of looks rather than a list of operating systems.
#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    /// Minimal, monochrome, no OS reference. The API default, and the one theme whose metrics
    /// are a documented contract: modes may do arithmetic against `outer_width`/`outer_height`,
    /// so these numbers are stable across platforms and versions.
    #[serde(rename = "plain")]
    #[default]
    Plain,
    /// Windows 11: flat, light, hairline border.
    #[serde(rename = "fluent")]
    Fluent,
    /// Windows 95/98: grey bevels, solid navy title bar, W95FA.
    #[serde(rename = "redmond")]
    Redmond,
    /// macOS: traffic lights on the left, centred title, graded bar.
    #[serde(rename = "aqua")]
    Aqua,
    /// GNOME: tall flat headerbar, centred title, round close button.
    #[serde(rename = "adwaita")]
    Adwaita,
    /// KDE Plasma: Breeze's cool greys, blue accent and compact controls.
    #[serde(rename = "breeze")]
    Breeze,
    /// Mac OS 9: pinstriped bar, black frame, close box on the left.
    #[serde(rename = "platinum")]
    Platinum,
    /// CDE/Motif: teal title bar, sculpted grey controls and workstation-era typography.
    #[serde(rename = "cde")]
    Cde,
}

/// Which palette a theme draws in.
///
/// **Appearance never changes [`Metrics`].** Header height, border width and button sizes are
/// identical light and dark, which is what keeps this a palette swap rather than a second
/// catalogue — and what lets it be resolved *after* a window exists, since nothing about the
/// window's size depends on it.
#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    #[serde(rename = "light")]
    #[default]
    Light,
    #[serde(rename = "dark")]
    Dark,
}

/// What a mode *asks* for on the appearance axis, which may be "whatever the system says".
///
/// Defaults to `Light`, not `Auto`: the API default is the predictable one. A dialog mixes
/// theme-driven widget colours with author-specified ones (`background_color`, per-element
/// `TextStyle`), so silently flipping the palette can make an author's own text unreadable on a
/// machine they cannot see. The bundled modes default their option to `auto` instead.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceChoice {
    #[serde(rename = "light")]
    Light,
    #[serde(rename = "dark")]
    Dark,
    /// Follow the desktop's own light/dark setting, falling back to light where that cannot be
    /// determined. Resolved by [`crate::window::appearance::detect`].
    #[serde(rename = "auto")]
    Auto,
}

impl AppearanceChoice {
    /// Every choice a mode may name, in the order they should be offered.
    pub const ALL: &'static [Self] = &[Self::Auto, Self::Light, Self::Dark];

    /// The Lua-facing name, matching the `serde(rename)` above.
    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::Auto => "auto",
        }
    }

    /// The inverse of [`name`](Self::name), for reading `AppConfig::appearance` — a free-form
    /// string, so an unrecognised one is a `None` the caller falls back from, not an error.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.name() == name)
    }
}

/// The user's own answer to "what should windows look like", applied to every window whose mode
/// did not name a theme itself.
///
/// This is the *only* place platform-varying chrome enters a spawn the mode did not ask for, and
/// it is deliberate: see `AppConfig::theme`. A mode that names a theme per window overrides it
/// and gets that theme's fixed metrics; a mode that says nothing gets the user's look, which is
/// what the overwhelming majority of modes should want.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromeDefaults {
    pub theme: ThemeChoice,
    pub appearance: AppearanceChoice,
}

/// Mirrors `AppConfig`'s own defaults, *not* [`ThemeChoice`]/[`AppearanceChoice`]'s. Those are the
/// API defaults, chosen to be predictable for a mode doing layout arithmetic; this is the product
/// default, chosen to look like the machine it is running on. `design/window-themes.md`, Defaults.
impl Default for ChromeDefaults {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Native,
            appearance: AppearanceChoice::Auto,
        }
    }
}

impl ChromeDefaults {
    /// Reads the two free-form strings out of the user's config. An unrecognised name — a config
    /// written against a newer engine, or hand-edited — falls back to the product default rather
    /// than failing: the user asked for *some* real look, so `native` is a far better guess at
    /// what they meant than the deliberately characterless `plain`.
    pub fn from_config(theme: &str, appearance: &str) -> Self {
        let fallback = Self::default();
        Self {
            theme: ThemeChoice::from_name(theme).unwrap_or(fallback.theme),
            appearance: AppearanceChoice::from_name(appearance).unwrap_or(fallback.appearance),
        }
    }
}

/// What a mode *asks* for, which may be an alias rather than a look.
///
/// Kept separate from [`Theme`] so that every `Theme` stays one concrete drawable appearance: the
/// platform branching lives here and nowhere else, and is resolved once — early, in
/// `PopupSpawnOpts::resolve` — so nothing downstream ever asks what OS it is on.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    #[serde(rename = "plain")]
    Plain,
    #[serde(rename = "fluent")]
    Fluent,
    #[serde(rename = "redmond")]
    Redmond,
    #[serde(rename = "aqua")]
    Aqua,
    #[serde(rename = "adwaita")]
    Adwaita,
    #[serde(rename = "breeze")]
    Breeze,
    #[serde(rename = "platinum")]
    Platinum,
    #[serde(rename = "cde")]
    Cde,
    /// Whatever this platform's current windows look like. On Linux this distinguishes KDE Plasma
    /// from other desktops, using Breeze for KDE and Adwaita as the general fallback.
    #[serde(rename = "native")]
    Native,
    /// Whatever this platform's windows *used* to look like. Linux resolves to CDE/Motif.
    #[serde(rename = "native-retro")]
    NativeRetro,
}

impl ThemeChoice {
    /// Every choice a mode may name, in the order they should be offered.
    pub const ALL: &'static [Self] = &[
        Self::Native,
        Self::NativeRetro,
        Self::Plain,
        Self::Fluent,
        Self::Redmond,
        Self::Aqua,
        Self::Adwaita,
        Self::Breeze,
        Self::Platinum,
        Self::Cde,
    ];

    /// The Lua-facing name, matching the `serde(rename)` above.
    ///
    /// Exposed to modes as `lewdware.themes` so a mode handed a theme name from *pack* data can
    /// check it before passing it on: an unknown name is a hard error at the spawn call, which is
    /// right for a mode author's typo but wrong for a pack built against a newer engine that knows
    /// a theme this one does not.
    pub fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Fluent => "fluent",
            Self::Redmond => "redmond",
            Self::Aqua => "aqua",
            Self::Adwaita => "adwaita",
            Self::Breeze => "breeze",
            Self::Platinum => "platinum",
            Self::Cde => "cde",
            Self::Native => "native",
            Self::NativeRetro => "native-retro",
        }
    }

    /// The inverse of [`name`](Self::name), for reading `AppConfig::theme`. `None` for a name
    /// this engine does not know — a config written against a newer engine — which the caller
    /// treats as "no preference" rather than as a failure.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.name() == name)
    }

    pub fn resolve(self) -> Theme {
        match self {
            Self::Plain => Theme::Plain,
            Self::Fluent => Theme::Fluent,
            Self::Redmond => Theme::Redmond,
            Self::Aqua => Theme::Aqua,
            Self::Adwaita => Theme::Adwaita,
            Self::Breeze => Theme::Breeze,
            Self::Platinum => Theme::Platinum,
            Self::Cde => Theme::Cde,
            Self::Native => Self::native(),
            Self::NativeRetro => Self::native_retro(),
        }
    }

    #[cfg(target_os = "windows")]
    fn native() -> Theme {
        Theme::Fluent
    }
    #[cfg(target_os = "macos")]
    fn native() -> Theme {
        Theme::Aqua
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    fn native() -> Theme {
        Self::native_unix(
            std::env::var("XDG_CURRENT_DESKTOP")
                .as_deref()
                .unwrap_or(""),
        )
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    fn native_unix(desktop: &str) -> Theme {
        if desktop
            .split(':')
            .any(|part| matches!(part.trim().to_ascii_lowercase().as_str(), "kde" | "plasma"))
        {
            Theme::Breeze
        } else {
            Theme::Adwaita
        }
    }

    #[cfg(target_os = "windows")]
    fn native_retro() -> Theme {
        Theme::Redmond
    }
    #[cfg(target_os = "macos")]
    fn native_retro() -> Theme {
        Theme::Platinum
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    fn native_retro() -> Theme {
        Theme::Cde
    }
}

/// Every theme, so exhaustiveness can be checked at run time by the tests as well as at compile
/// time by the matches below. Step 5 also offers this to `config/`, which needs to list the
/// catalogue in a dropdown.
pub const ALL_THEMES: &[Theme] = &[
    Theme::Plain,
    Theme::Fluent,
    Theme::Redmond,
    Theme::Aqua,
    Theme::Adwaita,
    Theme::Breeze,
    Theme::Platinum,
    Theme::Cde,
];

impl Theme {
    /// The catalogue name for this look, matching the `serde(rename)` on the variant.
    ///
    /// [`ThemeChoice`] has the same method, but they answer different questions: that one names
    /// what was *asked for* (`native` stays `native`), this one names what will be *drawn*. A
    /// caller that resolved an alias and wants to talk about the result needs this.
    pub fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Fluent => "fluent",
            Self::Redmond => "redmond",
            Self::Aqua => "aqua",
            Self::Adwaita => "adwaita",
            Self::Breeze => "breeze",
            Self::Platinum => "platinum",
            Self::Cde => "cde",
        }
    }

    pub fn metrics(self) -> Metrics {
        match self {
            Self::Plain => Metrics::PLAIN,
            Self::Fluent => Metrics {
                header_height: 32,
                border_width: 1,
            },
            Self::Redmond => Metrics {
                header_height: 18,
                border_width: 3,
            },
            Self::Aqua => Metrics {
                header_height: 28,
                border_width: 1,
            },
            Self::Adwaita => Metrics {
                header_height: 37,
                border_width: 1,
            },
            Self::Breeze => Metrics {
                header_height: 32,
                border_width: 1,
            },
            Self::Platinum => Metrics {
                header_height: 20,
                border_width: 1,
            },
            Self::Cde => Metrics {
                header_height: 22,
                border_width: 3,
            },
        }
    }

    /// Whether this theme has a dark palette at all.
    ///
    /// `platinum` does not: Mac OS 9 had no dark mode, and inventing one would be a look nobody
    /// recognises rather than a faithful one. It draws light whatever it is asked for.
    pub fn supports_dark(self) -> bool {
        !matches!(self, Self::Platinum)
    }

    /// The palette this theme will actually draw in.
    ///
    /// Both halves — chrome and widgets — go through this, so they can never end up disagreeing
    /// about which palette they are using.
    pub fn effective(self, appearance: Appearance) -> Appearance {
        if self.supports_dark() {
            appearance
        } else {
            Appearance::Light
        }
    }

    /// Everything this theme paints outside the content area.
    pub fn chrome(self, appearance: Appearance) -> Chrome {
        match (self, self.effective(appearance)) {
            (Self::Plain, Appearance::Light) => PLAIN_CHROME,
            (Self::Plain, Appearance::Dark) => PLAIN_CHROME_DARK,
            (Self::Fluent, Appearance::Light) => FLUENT_CHROME,
            (Self::Fluent, Appearance::Dark) => FLUENT_CHROME_DARK,
            (Self::Redmond, Appearance::Light) => REDMOND_CHROME,
            (Self::Redmond, Appearance::Dark) => REDMOND_CHROME_DARK,
            (Self::Aqua, Appearance::Light) => AQUA_CHROME,
            (Self::Aqua, Appearance::Dark) => AQUA_CHROME_DARK,
            (Self::Adwaita, Appearance::Light) => ADWAITA_CHROME,
            (Self::Adwaita, Appearance::Dark) => ADWAITA_CHROME_DARK,
            (Self::Breeze, Appearance::Light) => BREEZE_CHROME,
            (Self::Breeze, Appearance::Dark) => BREEZE_CHROME_DARK,
            (Self::Platinum, _) => PLATINUM_CHROME,
            (Self::Cde, Appearance::Light) => CDE_CHROME,
            (Self::Cde, Appearance::Dark) => CDE_CHROME_DARK,
        }
    }

    /// The face this theme lays widget text out in.
    ///
    /// The single biggest lever on whether a theme reads as the platform it imitates — a Win95
    /// dialog set in a modern UI font is neither. Each is the closest freely-licensed match:
    /// Selawik is Microsoft's own metric-compatible substitute for Segoe UI, Inter the usual
    /// stand-in for San Francisco, and Cantarell *is* GNOME's UI font.
    pub fn widget_font(self) -> Face {
        // Either palette would do -- a typeface is not a light/dark choice, which
        // `a_font_never_depends_on_the_palette` pins.
        self.widgets(Appearance::Light).font
    }

    /// The size widget text is laid out at, in logical points. Platforms differ: Win95's UI is
    /// noticeably smaller than a modern one, and GNOME's is larger.
    /// This theme's widget half in the requested palette.
    ///
    /// The whole of it, as data: `config/` renders a preview from exactly these values, and
    /// [`Widgets::to_egui_style`] is the only thing that turns them into something egui can draw.
    pub fn widgets(self, appearance: Appearance) -> &'static Widgets {
        match (self, self.effective(appearance)) {
            (Self::Plain, Appearance::Light) => &PLAIN_WIDGETS,
            (Self::Plain, Appearance::Dark) => &PLAIN_WIDGETS_DARK,
            (Self::Fluent, Appearance::Light) => &FLUENT_WIDGETS,
            (Self::Fluent, Appearance::Dark) => &FLUENT_WIDGETS_DARK,
            (Self::Redmond, Appearance::Light) => &REDMOND_WIDGETS,
            (Self::Redmond, Appearance::Dark) => &REDMOND_WIDGETS_DARK,
            (Self::Aqua, Appearance::Light) => &AQUA_WIDGETS,
            (Self::Aqua, Appearance::Dark) => &AQUA_WIDGETS_DARK,
            (Self::Adwaita, Appearance::Light) => &ADWAITA_WIDGETS,
            (Self::Adwaita, Appearance::Dark) => &ADWAITA_WIDGETS_DARK,
            (Self::Breeze, Appearance::Light) => &BREEZE_WIDGETS,
            (Self::Breeze, Appearance::Dark) => &BREEZE_WIDGETS_DARK,
            (Self::Platinum, _) => &PLATINUM_WIDGETS,
            (Self::Cde, Appearance::Light) => &CDE_WIDGETS,
            (Self::Cde, Appearance::Dark) => &CDE_WIDGETS_DARK,
        }
    }

    /// How this theme edges a dialog's buttons.
    ///
    /// Only the two themes whose controls are genuinely three-dimensional ask for a bevel. The flat
    /// platforms — Windows 11, macOS, GNOME — are flat in life, and `plain` is deliberately the
    /// plainest thing here, so all four keep egui's own painting.
    pub fn widget_edge(self, appearance: Appearance) -> WidgetEdge {
        self.widgets(appearance).edge
    }

    /// The persistent treatment used to identify a dialog's default action.
    ///
    /// This is separate from keyboard focus: the default remains the action Enter activates while
    /// the cursor is in a text field, so it must remain visible when focus is elsewhere. Modern
    /// themes use their platform's accent colour; `plain` uses a monochrome inversion; and the
    /// period themes reinforce the outer edge their controls already speak in.
    pub fn default_button_style(self, appearance: Appearance) -> DefaultButtonStyle {
        self.widgets(appearance).default_button
    }
}

// ── Chrome ───────────────────────────────────────────────────────────────────────
//
// A window's border, title bar and buttons, as the vocabulary `design/window-themes.md` settled
// on: gradients and pinstripes for the two Mac themes, multi-ring bevels for the period ones,
// circular and left-side button clusters for `aqua`/`platinum`. The engine paints these with
// tiny-skia in `header.rs`; `config/` draws the same values as CSS.

/// A colour from `#rrggbb` components, for building chrome palettes in `const`.
const fn rgb8(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

const BLACK: Color = rgb8(0, 0, 0);
const WHITE: Color = rgb8(255, 255, 255);
/// A glyph colour meaning "draw nothing" — used where a mark only appears on hover.
const TRANSPARENT: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

/// How an area of chrome is filled. Gradients are what `aqua` and `platinum` title bars need;
/// tiny-skia draws them natively, so they cost nothing to carry here.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub enum Fill {
    Solid(Color),
    VerticalGradient {
        from: Color,
        to: Color,
    },
    /// Alternating horizontal lines, `period` logical pixels apart — Mac OS 9's pinstriped title
    /// bar, which is most of what makes `platinum` recognisable.
    Pinstripe {
        base: Color,
        stripe: Color,
        period: u32,
    },
}

/// One one-logical-pixel ring of a window's border, outermost first.
///
/// `Bevel` is what makes a Win95-style 3D edge expressible: the top and left edges take one
/// colour and the bottom and right another, which a single flat colour per ring cannot express.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub enum BorderRing {
    Uniform(Color),
    Bevel {
        top_left: Color,
        bottom_right: Color,
    },
}

/// How a theme edges its *controls* — a dialog's buttons.
///
/// `Bevel` is the one thing `egui::Style` cannot express: `WidgetVisuals::bg_stroke` is a single
/// `Stroke`, uniform on all four sides, so a raised 3D edge — light on the top and left, dark on
/// the bottom and right — is inexpressible however the style is tuned. A theme asking for one is
/// painted by [`crate::window::layer::bevel`] instead of by egui.
///
/// Reuses [`BorderRing`] rather than a parallel type: a control's raised edge and a window's are
/// the same idea at different scales.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub enum WidgetEdge {
    /// egui paints it, from the stroke in the theme's `Style`.
    Flat,
    /// Painted here: rings from the outside in, one logical pixel each. `pressed` is drawn instead
    /// while the control is held, which is how a Win95 button appears to sink.
    Bevel {
        raised: &'static [BorderRing],
        pressed: &'static [BorderRing],
    },
}

/// How a dialog distinguishes the action activated by Enter from its other buttons.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub enum DefaultButtonStyle {
    /// Keep the normal themed face and reinforce its outer edge. Used by themes where a filled
    /// primary action would be anachronistic or needlessly loud.
    Outline(Stroke),
    /// A platform accent fill, with a separate response for hover and press.
    Filled {
        idle: Color,
        hover: Color,
        active: Color,
        text: Color,
        border: Stroke,
    },
}

/// A line: a colour and a width in logical pixels.
///
/// The theme-data stand-in for `egui::Stroke`, so that nothing in a theme's *definition* names an
/// egui type. [`Widgets::to_egui_style`] is the one place the two vocabularies meet.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    pub width: f32,
    pub color: Color,
}

impl Stroke {
    pub const NONE: Self = Self {
        width: 0.0,
        color: TRANSPARENT,
    };

    /// A hairline, which is what every theme that draws an edge at all asks for.
    pub const fn hairline(color: Color) -> Self {
        Self { width: 1.0, color }
    }

    const fn new(width: f32, color: Color) -> Self {
        Self { width, color }
    }
}

/// How a control is painted in one interaction state.
///
/// Buttons and text fields share these: egui draws a `TextEdit` with the same widget visuals as a
/// button, which is why a theme that leaves a field unbordered has to say so on both.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct ControlPaint {
    pub fill: Color,
    pub border: Stroke,
}

impl ControlPaint {
    const fn new(fill: Color, border: Stroke) -> Self {
        Self { fill, border }
    }
}

/// How a theme's controls are *shaped*, as distinct from coloured. Platforms differ far more in
/// padding and control height than in hue: this is what makes a Win95 button read as a Win95
/// button rather than an egui one in grey.
///
/// Every value is a whole number of logical points, deliberately. egui lays each widget out at the
/// running cursor, so a fractional gap puts every *subsequent* widget on a fractional coordinate —
/// which renders blurry, and which egui flags with an orange "Unaligned" marker in debug builds.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct WidgetMetrics {
    /// Horizontal and vertical padding inside a button.
    pub button_padding: (f32, f32),
    /// The minimum height of an interactive control.
    pub control_height: f32,
    /// Horizontal and vertical gap between adjacent controls.
    pub item_spacing: (f32, f32),
    /// One radius across every state. egui varies it slightly per state by default, which reads as
    /// wobble rather than intent once a theme has committed to a radius.
    pub corner_radius: u8,
}

/// The widget half of a theme: everything a dialog's buttons, text fields and labels are drawn
/// with, as plain data.
///
/// **Complete, not a diff.** Every value that can appear in a dialog is stated here, including the
/// ones egui would otherwise have supplied — text colour and caret being the two that used to be
/// inherited, differently, by half the catalogue. [`Self::to_egui_style`] still starts from
/// `egui::Visuals::light()`/`dark()`, but only as a substrate for the parts of egui we never draw
/// (window shadows, resize handles, hyperlinks); everything visible in one of our windows is set
/// from these fields. That is what makes the data, rather than an `egui::Style`, the theme.
///
/// Being egui-free, it also serialises — which is how `config/` can show a user what a theme looks
/// like without linking the engine.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Widgets {
    /// Which side of the light/dark divide this palette belongs to. Not redundant with the
    /// requested [`Appearance`]: `platinum` has no dark variant, so its dark request lands here as
    /// `Light`, and a reader of this data should be told what it actually got.
    pub base: Appearance,
    /// The dialog's own background. A Win95 dialog *is* button-face grey; leaving egui's near-white
    /// behind period chrome looked wrong and left some themes' buttons within a few levels of the
    /// surface they sat on. Overridden in turn by a window's `background_color`.
    pub panel: Color,
    /// Every piece of text in the window: labels, button captions, field contents. One colour
    /// rather than egui's per-state grey ramp — a dialog that darkens its own label text when the
    /// pointer crosses a button is egui's default, not any platform's behaviour.
    pub text: Color,
    /// The text-editing caret.
    pub caret: Stroke,
    /// A text field's interior, which is the one surface that is *not* the panel or a control.
    pub field: Color,
    /// The highlight behind selected text, and the colour of the text on top of it — which egui
    /// takes from the same `Selection::stroke` it uses for a focused field's outline. Both have to
    /// be set together: a saturated brand-colour highlight under egui's default text colour is
    /// unreadable, which is what every theme had. The fills are therefore tints rather than the
    /// platforms' full accent colours, since egui has one text colour to spend on both.
    pub selection: Color,
    pub selection_text: Color,
    /// At rest. Also used for the non-interactive and open states, which a dialog never shows in a
    /// way a user could tell apart.
    pub idle: ControlPaint,
    /// Under the pointer, and while held. Both are deliberately louder than the platforms they
    /// imitate: Windows 11 and Adwaita shift a control by a handful of levels on hover, and a
    /// macOS push button has no hover state at all — authentic, and useless here. These popups
    /// arrive in numbers and the user's usual goal is to dismiss them, so a control has to visibly
    /// answer the pointer. `every_theme_answers_the_pointer_visibly` holds the catalogue to a floor.
    pub hover: ControlPaint,
    pub pressed: ControlPaint,
    pub metrics: WidgetMetrics,
    /// The UI face and its size in logical points. Never varies with appearance — a palette swap
    /// is not a typeface change — which `a_font_never_depends_on_the_palette` pins.
    pub font: Face,
    pub font_size: f32,
    pub edge: WidgetEdge,
    pub default_button: DefaultButtonStyle,
}

/// The outline a chrome button is filled within.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonShape {
    /// A full-height rectangle, as on Windows.
    Rect,
    /// A circle centred in the header, as on macOS.
    Circle,
}

/// The mark drawn on top of a chrome button's fill.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Cross,
    /// A more open cross spanning most of a circular decoration, as in KDE Breeze.
    WideCross,
    /// Nothing — an inert button, or one whose glyph only appears on hover.
    None,
}

/// What pressing a chrome button does.
///
/// `Inert` exists only so a platform's silhouette can be completed honestly: macOS keeps its
/// minimise and zoom lights in place, uncoloured, on a window that can do neither. Such a button
/// never reacts to the pointer and never activates — see `design/window-themes.md`, "decorations
/// never lie about function". A *coloured* button that did nothing would be the violation.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonAction {
    Close,
    Inert,
}

/// One chrome button's paint in one pointer state.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct ButtonPaint {
    pub fill: Fill,
    pub glyph: Color,
    /// Optional outer rim for circular chrome controls. Rectangular buttons fill their whole slot
    /// and leave their edge to the window theme's border vocabulary.
    pub rim: Option<Color>,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Button {
    pub action: ButtonAction,
    pub shape: ButtonShape,
    pub glyph: Glyph,
    /// Width as a multiple of the header height, so buttons scale with the title bar.
    pub width_ratio: f32,
    pub idle: ButtonPaint,
    /// Ignored for [`ButtonAction::Inert`], which never leaves `idle`.
    pub hover: ButtonPaint,
    pub active: ButtonPaint,
}

/// Which end of the title bar the buttons sit at, and how they are spaced.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Buttons {
    pub side: Side,
    /// Logical pixels between the outer edge and the first button.
    pub inset: f32,
    /// Logical pixels between adjacent buttons.
    pub gap: f32,
    /// In visual order from `side` inwards.
    pub buttons: &'static [Button],
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct TitleStyle {
    pub font: Face,
    /// Logical points.
    pub size: f32,
    pub color: Color,
    /// Logical pixels kept clear at the ends of the title bar.
    pub padding: f32,
    /// Where the title sits along the bar. A real per-platform difference: Windows and GNOME
    /// dialogs align left, macOS centres. `Center` falls back to the near edge when the text is
    /// too wide to centre without colliding with the buttons.
    pub align: TextAlign,
}

/// Everything a theme paints outside the content area.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Chrome {
    pub header: Fill,
    /// Outermost ring first. Its length is the border width in logical pixels, and must equal
    /// [`Metrics::border_width`] — a test holds every theme to that.
    pub border: &'static [BorderRing],
    pub title: TitleStyle,
    pub buttons: Buttons,
}

/// A close button that fills its corner of the bar, Windows-style.
const fn rect_close(
    width_ratio: f32,
    idle: ButtonPaint,
    hover: ButtonPaint,
    active: ButtonPaint,
) -> Button {
    Button {
        action: ButtonAction::Close,
        shape: ButtonShape::Rect,
        glyph: Glyph::Cross,
        width_ratio,
        idle,
        hover,
        active,
    }
}

const fn paint(fill: Color, glyph: Color) -> ButtonPaint {
    ButtonPaint {
        fill: Fill::Solid(fill),
        glyph,
        rim: None,
    }
}

const fn rimmed(fill: Fill, glyph: Color, rim: Color) -> ButtonPaint {
    ButtonPaint {
        fill,
        glyph,
        rim: Some(rim),
    }
}

// plain. Deliberately the plainest thing in the catalogue: greyscale, hairline, square, a short bar
// and a small square close button. It imitates no platform, which is the point — chrome that behaves
// as furniture and does not compete with whatever art a pack puts inside it, a job none of the
// platform themes can do. Its one flourish is the loud red close button kept from the engine's
// original look: when windows are stacked in numbers, a close target that shouts is a feature.
const PLAIN_HEADER: Color = rgb8(232, 232, 232);
const PLAIN_CLOSE_HOVER: Color = rgb8(255, 0, 0);
const PLAIN_CLOSE_ACTIVE: Color = rgb8(230, 0, 0);

/// `plain`'s close button: square rather than the wide slab a Windows caption uses, and monochrome
/// until the pointer reaches it.
const fn plain_close(bar: Color, glyph: Color) -> Button {
    Button {
        action: ButtonAction::Close,
        shape: ButtonShape::Rect,
        glyph: Glyph::Cross,
        width_ratio: 1.0,
        idle: ButtonPaint {
            fill: Fill::Solid(bar),
            glyph,
            rim: None,
        },
        hover: ButtonPaint {
            fill: Fill::Solid(PLAIN_CLOSE_HOVER),
            glyph: WHITE,
            rim: None,
        },
        active: ButtonPaint {
            fill: Fill::Solid(PLAIN_CLOSE_ACTIVE),
            glyph: WHITE,
            rim: None,
        },
    }
}

const PLAIN_CHROME: Chrome = Chrome {
    header: Fill::Solid(PLAIN_HEADER),
    border: &[BorderRing::Uniform(BLACK)],
    title: TitleStyle {
        font: Face::Default,
        size: 12.0,
        color: BLACK,
        padding: 8.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 0.0,
        gap: 0.0,
        buttons: &[plain_close(PLAIN_HEADER, BLACK)],
    },
};

// ── The catalogue ────────────────────────────────────────────────────────────────
//
// One concrete look each. Light only for now; appearance (light/dark) is a separate axis, and the
// invariance rule means it can be added without touching any of the metrics below.

// fluent — Windows 11. Today's chrome, corrected: a lighter bar and Windows' actual close red
// (#C42B1C) rather than pure red. Square corners; see `design/window-themes.md` on why rounding is
// not expressible yet.
const FLUENT_HEADER: Color = rgb8(243, 243, 243);
const FLUENT_TEXT: Color = rgb8(26, 26, 26);

const FLUENT_CHROME: Chrome = Chrome {
    header: Fill::Solid(FLUENT_HEADER),
    border: &[BorderRing::Uniform(rgb8(180, 180, 180))],
    title: TitleStyle {
        font: Face::Selawik,
        size: 12.0,
        color: FLUENT_TEXT,
        padding: 12.0,
        // Windows puts its title at the left, unlike the centred plain bar.
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 0.0,
        gap: 0.0,
        buttons: &[rect_close(
            1.5,
            paint(FLUENT_HEADER, FLUENT_TEXT),
            paint(rgb8(196, 43, 28), WHITE),
            paint(rgb8(168, 36, 25), WHITE),
        )],
    },
};

// redmond — Windows 95/98. A three-ring raised bevel, a solid navy bar, and the bundled W95FA.
const REDMOND_FACE: Color = rgb8(192, 192, 192);
const REDMOND_NAVY: Color = rgb8(0, 0, 128);

const REDMOND_CHROME: Chrome = Chrome {
    header: Fill::Solid(REDMOND_NAVY),
    // Light on the top-left, dark on the bottom-right: the 3D raised edge, outermost ring first.
    border: &[
        BorderRing::Bevel {
            top_left: rgb8(223, 223, 223),
            bottom_right: BLACK,
        },
        BorderRing::Bevel {
            top_left: WHITE,
            bottom_right: rgb8(128, 128, 128),
        },
        BorderRing::Uniform(REDMOND_FACE),
    ],
    title: TitleStyle {
        font: Face::Pixel,
        size: 12.0,
        color: WHITE,
        padding: 3.0,
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 2.0,
        gap: 0.0,
        // Roughly square, as Win95's caption buttons are. Hover only lightens: these buttons had
        // no hover state at all, but a close target with no feedback is worse than a mild one.
        buttons: &[rect_close(
            0.9,
            paint(REDMOND_FACE, BLACK),
            paint(rgb8(212, 208, 200), BLACK),
            paint(rgb8(160, 160, 160), BLACK),
        )],
    },
};

// The Win95 control edge, outermost ring first. Light on the top and left, dark on the bottom and
// right, which is what makes it read as raised; the pressed pair is the same rings inverted, so the
// control appears to sink under the pointer rather than merely change colour.
const REDMOND_RAISED: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: rgb8(223, 223, 223),
        bottom_right: BLACK,
    },
    BorderRing::Bevel {
        top_left: WHITE,
        bottom_right: rgb8(128, 128, 128),
    },
];
const REDMOND_PRESSED: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: BLACK,
        bottom_right: rgb8(223, 223, 223),
    },
    BorderRing::Bevel {
        top_left: rgb8(128, 128, 128),
        bottom_right: WHITE,
    },
];

const REDMOND_RAISED_DARK: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: REDMOND_HIGHLIGHT_DARK,
        bottom_right: BLACK,
    },
    BorderRing::Bevel {
        top_left: REDMOND_BRIGHT_DARK,
        bottom_right: REDMOND_SHADOW_DARK,
    },
];
const REDMOND_PRESSED_DARK: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: BLACK,
        bottom_right: REDMOND_HIGHLIGHT_DARK,
    },
    BorderRing::Bevel {
        top_left: REDMOND_SHADOW_DARK,
        bottom_right: REDMOND_BRIGHT_DARK,
    },
];

// Mac OS 9's controls are outlined in a hard grey with a single highlight inside it.
const PLATINUM_RAISED: &[BorderRing] = &[
    BorderRing::Uniform(rgb8(85, 85, 85)),
    BorderRing::Bevel {
        top_left: WHITE,
        bottom_right: rgb8(170, 170, 170),
    },
];
const PLATINUM_PRESSED: &[BorderRing] = &[
    BorderRing::Uniform(rgb8(85, 85, 85)),
    BorderRing::Bevel {
        top_left: rgb8(170, 170, 170),
        bottom_right: WHITE,
    },
];

// aqua — macOS. Left-side traffic lights, a centred title, a gently graded bar. Minimise and zoom
// are present but inert and uncoloured, which is what macOS itself does on a window that can do
// neither — see `design/window-themes.md`, "decorations never lie about function".
const AQUA_DISABLED: Color = rgb8(213, 213, 213);
const AQUA_LIGHT_RATIO: f32 = 0.43;

const fn aqua_disabled_light() -> ButtonPaint {
    rimmed(
        Fill::VerticalGradient {
            from: rgb8(226, 226, 226),
            to: AQUA_DISABLED,
        },
        TRANSPARENT,
        rgb8(190, 190, 190),
    )
}

const fn aqua_red(fill_top: Color, fill_bottom: Color, glyph: Color) -> ButtonPaint {
    rimmed(
        Fill::VerticalGradient {
            from: fill_top,
            to: fill_bottom,
        },
        glyph,
        rgb8(218, 72, 66),
    )
}

/// One of aqua's inert lights: flat grey, no glyph, no reaction to the pointer.
const fn aqua_inert() -> Button {
    Button {
        action: ButtonAction::Inert,
        shape: ButtonShape::Circle,
        glyph: Glyph::None,
        width_ratio: AQUA_LIGHT_RATIO,
        idle: aqua_disabled_light(),
        hover: aqua_disabled_light(),
        active: aqua_disabled_light(),
    }
}

const AQUA_CHROME: Chrome = Chrome {
    header: Fill::VerticalGradient {
        from: rgb8(246, 246, 246),
        to: rgb8(232, 232, 232),
    },
    border: &[BorderRing::Uniform(rgb8(176, 176, 176))],
    title: TitleStyle {
        font: Face::Inter,
        size: 13.0,
        color: rgb8(77, 77, 77),
        padding: 12.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Left,
        inset: 8.0,
        gap: 8.0,
        buttons: &[
            Button {
                action: ButtonAction::Close,
                shape: ButtonShape::Circle,
                glyph: Glyph::Cross,
                width_ratio: AQUA_LIGHT_RATIO,
                // The glyph is transparent until hovered, exactly as Aqua hides its marks until
                // the pointer enters the cluster.
                idle: aqua_red(rgb8(255, 128, 122), rgb8(255, 95, 87), TRANSPARENT),
                hover: aqua_red(rgb8(255, 128, 122), rgb8(255, 95, 87), rgb8(77, 0, 0)),
                active: aqua_red(rgb8(211, 87, 81), rgb8(191, 71, 66), rgb8(77, 0, 0)),
            },
            aqua_inert(),
            aqua_inert(),
        ],
    },
};

// adwaita — GNOME. A tall flat headerbar with a centred title and a round close button.
const ADWAITA_HEADER: Color = rgb8(235, 235, 233);
const ADWAITA_TEXT: Color = rgb8(46, 52, 54);

const ADWAITA_CHROME: Chrome = Chrome {
    header: Fill::Solid(ADWAITA_HEADER),
    border: &[BorderRing::Uniform(rgb8(192, 191, 188))],
    title: TitleStyle {
        font: Face::Cantarell,
        size: 14.0,
        color: ADWAITA_TEXT,
        padding: 12.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 6.0,
        gap: 0.0,
        buttons: &[Button {
            action: ButtonAction::Close,
            shape: ButtonShape::Circle,
            glyph: Glyph::Cross,
            width_ratio: 0.65,
            idle: paint(rgb8(214, 210, 205), ADWAITA_TEXT),
            hover: paint(rgb8(198, 194, 188), ADWAITA_TEXT),
            active: paint(rgb8(180, 176, 170), ADWAITA_TEXT),
        }],
    },
};

// breeze — KDE Plasma. Cool neutral surfaces, compact geometry and KDE's blue accent. KWin
// decorations vary by distribution, so this follows upstream Breeze rather than a distro skin.
const BREEZE_HEADER: Color = rgb8(239, 240, 241);
const BREEZE_TEXT: Color = rgb8(35, 38, 41);

const BREEZE_CHROME: Chrome = Chrome {
    header: Fill::Solid(BREEZE_HEADER),
    border: &[BorderRing::Uniform(rgb8(189, 195, 199))],
    title: TitleStyle {
        font: Face::NotoSans,
        size: 13.0,
        color: BREEZE_TEXT,
        padding: 10.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 6.0,
        gap: 0.0,
        buttons: &[Button {
            action: ButtonAction::Close,
            shape: ButtonShape::Circle,
            glyph: Glyph::WideCross,
            width_ratio: 0.55,
            idle: paint(BREEZE_HEADER, BREEZE_TEXT),
            hover: paint(rgb8(255, 130, 145), WHITE),
            active: paint(rgb8(225, 82, 105), WHITE),
        }],
    },
};

// cde — the Common Desktop Environment's Motif language: blue-green active title, warm grey
// faces and deeply modelled bevels. The palette is intentionally workstation-like rather than a
// clone of any one vendor's CDE defaults, which differed across Solaris, HP-UX and AIX.
const CDE_FACE: Color = rgb8(184, 184, 174);
const CDE_SHADOW: Color = rgb8(82, 82, 76);
const CDE_TITLE: Color = rgb8(45, 98, 96);
const CDE_FACE_DARK: Color = rgb8(91, 98, 94);
const CDE_PANEL_DARK: Color = rgb8(62, 67, 65);
const CDE_SHADOW_DARK: Color = rgb8(25, 28, 27);
const CDE_TITLE_DARK: Color = rgb8(26, 75, 73);

const CDE_RAISED: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: rgb8(238, 238, 226),
        bottom_right: rgb8(70, 70, 65),
    },
    BorderRing::Bevel {
        top_left: rgb8(211, 211, 200),
        bottom_right: rgb8(118, 118, 109),
    },
];
const CDE_PRESSED: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: rgb8(70, 70, 65),
        bottom_right: rgb8(238, 238, 226),
    },
    BorderRing::Bevel {
        top_left: rgb8(118, 118, 109),
        bottom_right: rgb8(211, 211, 200),
    },
];
const CDE_RAISED_DARK: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: rgb8(154, 164, 158),
        bottom_right: CDE_SHADOW_DARK,
    },
    BorderRing::Bevel {
        top_left: rgb8(124, 134, 129),
        bottom_right: rgb8(47, 51, 49),
    },
];
const CDE_PRESSED_DARK: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: CDE_SHADOW_DARK,
        bottom_right: rgb8(154, 164, 158),
    },
    BorderRing::Bevel {
        top_left: rgb8(47, 51, 49),
        bottom_right: rgb8(124, 134, 129),
    },
];

const CDE_CHROME: Chrome = Chrome {
    header: Fill::Solid(CDE_TITLE),
    border: &[
        BorderRing::Bevel {
            top_left: rgb8(238, 238, 226),
            bottom_right: rgb8(55, 55, 51),
        },
        BorderRing::Bevel {
            top_left: rgb8(211, 211, 200),
            bottom_right: CDE_SHADOW,
        },
        BorderRing::Uniform(CDE_FACE),
    ],
    title: TitleStyle {
        font: Face::LiberationSansBold,
        size: 12.0,
        color: WHITE,
        padding: 4.0,
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 2.0,
        gap: 0.0,
        buttons: &[rect_close(
            0.85,
            paint(CDE_FACE, BLACK),
            paint(rgb8(205, 205, 194), BLACK),
            paint(rgb8(145, 145, 137), BLACK),
        )],
    },
};

const CDE_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(CDE_TITLE_DARK),
    border: &[
        BorderRing::Bevel {
            top_left: rgb8(154, 164, 158),
            bottom_right: rgb8(18, 20, 19),
        },
        BorderRing::Bevel {
            top_left: rgb8(124, 134, 129),
            bottom_right: CDE_SHADOW_DARK,
        },
        BorderRing::Uniform(CDE_FACE_DARK),
    ],
    title: TitleStyle {
        font: Face::LiberationSansBold,
        size: 12.0,
        color: WHITE,
        padding: 4.0,
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 2.0,
        gap: 0.0,
        buttons: &[rect_close(
            0.85,
            paint(CDE_FACE_DARK, WHITE),
            paint(rgb8(111, 119, 115), WHITE),
            paint(rgb8(67, 72, 70), WHITE),
        )],
    },
};

// platinum — Mac OS 9. Pinstriped bar, black frame, close box on the *left*. No zoom or collapse
// box: Mac OS 9 omits them on windows that cannot do either, so nothing has to be faked.
const PLATINUM_FACE: Color = rgb8(221, 221, 221);
/// The pinstripe pair is its own, darker-contrasting pair rather than a tint of `PLATINUM_FACE`:
/// at only a few levels apart the stripes vanished into flat grey, especially above 1x where each
/// logical line covers a fractional number of physical pixels.
const PLATINUM_STRIPE_BASE: Color = rgb8(207, 207, 207);
const PLATINUM_STRIPE: Color = rgb8(232, 232, 232);

const PLATINUM_CHROME: Chrome = Chrome {
    header: Fill::Pinstripe {
        base: PLATINUM_STRIPE_BASE,
        stripe: PLATINUM_STRIPE,
        period: 2,
    },
    border: &[BorderRing::Uniform(BLACK)],
    title: TitleStyle {
        font: Face::SourceSansSemibold,
        size: 12.0,
        color: BLACK,
        padding: 4.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Left,
        inset: 4.0,
        gap: 0.0,
        buttons: &[rect_close(
            0.6,
            paint(PLATINUM_FACE, BLACK),
            paint(rgb8(238, 238, 238), BLACK),
            paint(rgb8(170, 170, 170), BLACK),
        )],
    },
};

// ── Dark palettes ────────────────────────────────────────────────────────────────
//
// Same metrics, same button geometry, different colours — see `Appearance`. Only `platinum` has
// none: Mac OS 9 had no dark mode, and an invented one would be a look nobody recognises.

// plain dark. The loud red close button is kept deliberately: `plain` is the one theme not
// imitating anything, and a close target that stands out is a feature when windows are stacked.
// Neutral greys, not the slightly blue-tinted pair this had: `plain` is greyscale by definition.
const PLAIN_HEADER_DARK: Color = rgb8(38, 38, 38);
const PLAIN_TEXT_DARK: Color = rgb8(240, 240, 240);

const PLAIN_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(PLAIN_HEADER_DARK),
    // Mid-grey rather than black: a dark border against a dark header leaves no visible edge.
    border: &[BorderRing::Uniform(rgb8(128, 128, 128))],
    title: TitleStyle {
        font: Face::Default,
        size: 12.0,
        color: PLAIN_TEXT_DARK,
        padding: 8.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 0.0,
        gap: 0.0,
        buttons: &[plain_close(PLAIN_HEADER_DARK, PLAIN_TEXT_DARK)],
    },
};

// fluent dark — Windows 11's own dark title bar, keeping the same close red.
const FLUENT_HEADER_DARK: Color = rgb8(32, 32, 32);

const FLUENT_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(FLUENT_HEADER_DARK),
    border: &[BorderRing::Uniform(rgb8(74, 74, 74))],
    title: TitleStyle {
        font: Face::Selawik,
        size: 12.0,
        color: WHITE,
        padding: 12.0,
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 0.0,
        gap: 0.0,
        buttons: &[rect_close(
            1.5,
            paint(FLUENT_HEADER_DARK, WHITE),
            paint(rgb8(196, 43, 28), WHITE),
            paint(rgb8(168, 36, 25), WHITE),
        )],
    },
};

// redmond dark. Not "Win95 with a dark mode" — Win95 shipped dark *appearance schemes* of its own
// (Eggplant, Plum), so this is a period-correct palette rather than an anachronism.
//
// Note the title bar is *lighter* than the light variant's, which is not a mistake: the light bar
// is `#000080` navy, already darker than most dark-mode chrome. What actually darkens is the face
// — the frame, the button fills and the dialog background around the content.
const REDMOND_FACE_DARK: Color = rgb8(112, 108, 124);
const REDMOND_PANEL_DARK: Color = rgb8(78, 77, 85);
const REDMOND_TITLE_DARK: Color = rgb8(46, 32, 60);
const REDMOND_HIGHLIGHT_DARK: Color = rgb8(174, 169, 190);
const REDMOND_BRIGHT_DARK: Color = rgb8(211, 206, 222);
const REDMOND_SHADOW_DARK: Color = rgb8(67, 63, 75);
const REDMOND_GLYPH_DARK: Color = rgb8(240, 238, 245);

const REDMOND_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(REDMOND_TITLE_DARK),
    border: &[
        BorderRing::Bevel {
            top_left: REDMOND_HIGHLIGHT_DARK,
            bottom_right: BLACK,
        },
        BorderRing::Bevel {
            top_left: REDMOND_BRIGHT_DARK,
            bottom_right: REDMOND_SHADOW_DARK,
        },
        BorderRing::Uniform(REDMOND_FACE_DARK),
    ],
    title: TitleStyle {
        font: Face::Pixel,
        size: 12.0,
        color: WHITE,
        padding: 3.0,
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 2.0,
        gap: 0.0,
        buttons: &[rect_close(
            0.9,
            paint(REDMOND_FACE_DARK, REDMOND_GLYPH_DARK),
            paint(rgb8(124, 119, 148), REDMOND_GLYPH_DARK),
            paint(rgb8(90, 86, 112), REDMOND_GLYPH_DARK),
        )],
    },
};

// aqua dark. The traffic lights keep their colours — macOS does not grey them in dark mode; only
// the bar and title change. The inert pair moves to dark mode's own disabled grey.
const AQUA_DISABLED_DARK: Color = rgb8(90, 90, 90);

const fn aqua_disabled_dark_paint() -> ButtonPaint {
    rimmed(
        Fill::VerticalGradient {
            from: rgb8(105, 105, 105),
            to: AQUA_DISABLED_DARK,
        },
        TRANSPARENT,
        rgb8(65, 65, 65),
    )
}

const fn aqua_inert_dark() -> Button {
    Button {
        action: ButtonAction::Inert,
        shape: ButtonShape::Circle,
        glyph: Glyph::None,
        width_ratio: AQUA_LIGHT_RATIO,
        idle: aqua_disabled_dark_paint(),
        hover: aqua_disabled_dark_paint(),
        active: aqua_disabled_dark_paint(),
    }
}

const AQUA_CHROME_DARK: Chrome = Chrome {
    header: Fill::VerticalGradient {
        from: rgb8(58, 58, 60),
        to: rgb8(44, 44, 46),
    },
    border: &[BorderRing::Uniform(rgb8(74, 74, 76))],
    title: TitleStyle {
        font: Face::Inter,
        size: 13.0,
        color: rgb8(176, 176, 180),
        padding: 12.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Left,
        inset: 8.0,
        gap: 8.0,
        buttons: &[
            Button {
                action: ButtonAction::Close,
                shape: ButtonShape::Circle,
                glyph: Glyph::Cross,
                width_ratio: AQUA_LIGHT_RATIO,
                idle: aqua_red(rgb8(255, 128, 122), rgb8(255, 95, 87), TRANSPARENT),
                hover: aqua_red(rgb8(255, 128, 122), rgb8(255, 95, 87), rgb8(77, 0, 0)),
                active: aqua_red(rgb8(211, 87, 81), rgb8(191, 71, 66), rgb8(77, 0, 0)),
            },
            aqua_inert_dark(),
            aqua_inert_dark(),
        ],
    },
};

// adwaita dark — GNOME's own dark headerbar.
const ADWAITA_HEADER_DARK: Color = rgb8(48, 48, 48);

const ADWAITA_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(ADWAITA_HEADER_DARK),
    border: &[BorderRing::Uniform(rgb8(27, 27, 27))],
    title: TitleStyle {
        font: Face::Cantarell,
        size: 14.0,
        color: WHITE,
        padding: 12.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 6.0,
        gap: 0.0,
        buttons: &[Button {
            action: ButtonAction::Close,
            shape: ButtonShape::Circle,
            glyph: Glyph::Cross,
            width_ratio: 0.65,
            idle: paint(rgb8(69, 69, 69), WHITE),
            hover: paint(rgb8(85, 85, 85), WHITE),
            active: paint(rgb8(102, 102, 102), WHITE),
        }],
    },
};

const BREEZE_HEADER_DARK: Color = rgb8(49, 54, 59);

const BREEZE_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(BREEZE_HEADER_DARK),
    border: &[BorderRing::Uniform(rgb8(23, 26, 28))],
    title: TitleStyle {
        font: Face::NotoSans,
        size: 13.0,
        color: rgb8(239, 240, 241),
        padding: 10.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 6.0,
        gap: 0.0,
        buttons: &[Button {
            action: ButtonAction::Close,
            shape: ButtonShape::Circle,
            glyph: Glyph::WideCross,
            width_ratio: 0.55,
            idle: paint(BREEZE_HEADER_DARK, rgb8(239, 240, 241)),
            hover: paint(rgb8(255, 130, 145), WHITE),
            active: paint(rgb8(225, 82, 105), WHITE),
        }],
    },
};

/// The physical band `[start, end)` that ring `index` of `count` occupies within a border
/// `thickness` physical pixels thick.
///
/// Rings are one *logical* pixel each, so at a fractional scale factor they cannot each be a whole
/// number of physical pixels. Dividing the measured total keeps the rings adjacent and exactly
/// filling it, which is what matters: any gap would show the window's background between the
/// border and the content.
pub fn ring_band(index: usize, count: usize, thickness: u32) -> (u32, u32) {
    debug_assert!(index < count, "ring {index} out of range for {count} rings");

    let at = |i: usize| ((i as u64 * thickness as u64) / count as u64) as u32;
    (at(index), at(index + 1))
}

impl BorderRing {
    /// The colour of this ring's top and left edges.
    pub fn top_left(self) -> Color {
        match self {
            Self::Uniform(color) => color,
            Self::Bevel { top_left, .. } => top_left,
        }
    }

    /// The colour of this ring's bottom and right edges.
    pub fn bottom_right(self) -> Color {
        match self {
            Self::Uniform(color) => color,
            Self::Bevel { bottom_right, .. } => bottom_right,
        }
    }
}

/// A theme's geometry, in **logical** pixels.
///
/// Appearance (light/dark) must never change these — see `design/window-themes.md`'s invariance
/// rule. That is what lets appearance be resolved after a window already exists, while metrics
/// have to be known at creation time because they set the outer size.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    pub header_height: u32,
    pub border_width: u32,
}

impl Metrics {
    pub const PLAIN: Self = Self {
        header_height: 20,
        border_width: 1,
    };

    /// The extra logical pixels a decorated window needs beyond its content, as
    /// `(width, height)`. Borders on both sides; the header only above.
    pub fn outer_padding(&self) -> (u32, u32) {
        (
            self.border_width * 2,
            self.border_width * 2 + self.header_height,
        )
    }
}

// ─── Widget palettes ─────────────────────────────────────────────────────────
//
// One `Widgets` per theme per palette, stated in full. Values carried over from when this half
// was built by mutating an `egui::Style`, with two exceptions that were previously inherited from
// egui and are now each theme's own: `text` (egui's #505050/#8c8c8c body grey, plus a per-state
// ramp that darkened labels under the pointer) and `caret` (egui's #00537d/#c0deff, a blue that
// belonged to no theme here).

/// Every retro face wants a hairline caret; egui's default 2pt bar is a modern affectation.
const PERIOD_CARET: f32 = 1.0;
const MODERN_CARET: f32 = 2.0;

const PLAIN_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: rgb8(245, 245, 245),
    text: rgb8(26, 26, 26),
    caret: Stroke::new(MODERN_CARET, rgb8(26, 26, 26)),
    field: WHITE,
    selection: rgb8(200, 200, 200),
    selection_text: BLACK,
    idle: ControlPaint::new(WHITE, Stroke::hairline(rgb8(48, 48, 48))),
    hover: ControlPaint::new(rgb8(220, 220, 220), Stroke::hairline(rgb8(48, 48, 48))),
    pressed: ControlPaint::new(rgb8(180, 180, 180), Stroke::hairline(rgb8(48, 48, 48))),
    metrics: WidgetMetrics {
        button_padding: (8.0, 3.0),
        control_height: 22.0,
        item_spacing: (7.0, 5.0),
        corner_radius: 0,
    },
    font: Face::Default,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    // A monochrome inversion rather than an accent: `plain` imitates no platform, so it has no
    // accent colour to borrow.
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(45, 45, 45),
        hover: rgb8(70, 70, 70),
        active: rgb8(105, 105, 105),
        text: WHITE,
        border: Stroke::NONE,
    },
};

const PLAIN_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: rgb8(24, 24, 24),
    text: rgb8(240, 240, 240),
    caret: Stroke::new(MODERN_CARET, rgb8(240, 240, 240)),
    field: rgb8(13, 13, 13),
    selection: rgb8(89, 89, 89),
    selection_text: WHITE,
    idle: ControlPaint::new(rgb8(51, 51, 51), Stroke::hairline(rgb8(160, 160, 160))),
    hover: ControlPaint::new(rgb8(77, 77, 77), Stroke::hairline(rgb8(160, 160, 160))),
    pressed: ControlPaint::new(rgb8(102, 102, 102), Stroke::hairline(rgb8(160, 160, 160))),
    metrics: PLAIN_WIDGETS.metrics,
    font: Face::Default,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(235, 235, 235),
        hover: rgb8(210, 210, 210),
        active: rgb8(175, 175, 175),
        text: BLACK,
        border: Stroke::NONE,
    },
};

const FLUENT_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: rgb8(243, 243, 243),
    // Windows 11's own body text, which is near-black rather than the pure black of Win95.
    text: rgb8(27, 27, 27),
    caret: Stroke::new(MODERN_CARET, rgb8(27, 27, 27)),
    field: WHITE,
    selection: rgb8(204, 228, 247),
    selection_text: rgb8(0, 62, 107),
    // Windows 11 outlines every control, including at rest — that hairline is a lot of why its
    // buttons look like buttons rather than flat tinted rectangles.
    idle: ControlPaint::new(rgb8(251, 251, 251), Stroke::hairline(rgb8(209, 209, 209))),
    hover: ControlPaint::new(rgb8(229, 229, 229), Stroke::hairline(rgb8(209, 209, 209))),
    pressed: ControlPaint::new(rgb8(213, 213, 213), Stroke::hairline(rgb8(209, 209, 209))),
    metrics: WidgetMetrics {
        button_padding: (11.0, 6.0),
        control_height: 32.0,
        item_spacing: (8.0, 6.0),
        corner_radius: 4,
    },
    font: Face::Selawik,
    font_size: 14.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(0, 103, 192),
        hover: rgb8(25, 117, 197),
        active: rgb8(0, 90, 158),
        text: WHITE,
        border: Stroke::hairline(rgb8(0, 90, 158)),
    },
};

const FLUENT_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: rgb8(32, 32, 32),
    text: WHITE,
    caret: Stroke::new(MODERN_CARET, WHITE),
    field: rgb8(31, 31, 31),
    selection: rgb8(31, 58, 92),
    selection_text: rgb8(207, 230, 255),
    idle: ControlPaint::new(rgb8(45, 45, 45), Stroke::hairline(rgb8(66, 66, 66))),
    hover: ControlPaint::new(rgb8(68, 68, 68), Stroke::hairline(rgb8(66, 66, 66))),
    pressed: ControlPaint::new(rgb8(82, 82, 82), Stroke::hairline(rgb8(66, 66, 66))),
    metrics: FLUENT_WIDGETS.metrics,
    font: Face::Selawik,
    font_size: 14.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(0, 95, 158),
        hover: rgb8(18, 112, 178),
        active: rgb8(0, 78, 130),
        text: WHITE,
        border: Stroke::hairline(rgb8(0, 70, 117)),
    },
};

const REDMOND_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    // Win95 dialogs are the same grey as their buttons; the buttons are told apart by their
    // border, not their fill.
    panel: REDMOND_FACE,
    text: BLACK,
    caret: Stroke::new(PERIOD_CARET, BLACK),
    field: WHITE,
    selection: rgb8(195, 207, 230),
    selection_text: rgb8(0, 0, 128),
    // Square, flat, and a hard edge on every state: Win95 controls have no radius and no hover
    // fill of their own, only a bevel — which `edge` paints.
    idle: ControlPaint::new(REDMOND_FACE, Stroke::hairline(rgb8(128, 128, 128))),
    hover: ControlPaint::new(rgb8(214, 210, 202), Stroke::hairline(BLACK)),
    pressed: ControlPaint::new(rgb8(160, 160, 160), Stroke::hairline(BLACK)),
    metrics: WidgetMetrics {
        button_padding: (8.0, 3.0),
        control_height: 21.0,
        item_spacing: (6.0, 4.0),
        corner_radius: 0,
    },
    font: Face::Pixel,
    font_size: 12.0,
    edge: WidgetEdge::Bevel {
        raised: REDMOND_RAISED,
        pressed: REDMOND_PRESSED,
    },
    default_button: DefaultButtonStyle::Outline(Stroke::hairline(BLACK)),
};

const REDMOND_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: REDMOND_PANEL_DARK,
    text: rgb8(230, 225, 245),
    caret: Stroke::new(PERIOD_CARET, rgb8(230, 225, 245)),
    field: rgb8(34, 31, 39),
    selection: rgb8(74, 66, 96),
    selection_text: rgb8(230, 225, 245),
    idle: ControlPaint::new(REDMOND_FACE_DARK, Stroke::hairline(REDMOND_SHADOW_DARK)),
    hover: ControlPaint::new(rgb8(137, 132, 148), Stroke::hairline(BLACK)),
    pressed: ControlPaint::new(rgb8(82, 77, 92), Stroke::hairline(BLACK)),
    metrics: REDMOND_WIDGETS.metrics,
    font: Face::Pixel,
    font_size: 12.0,
    edge: WidgetEdge::Bevel {
        raised: REDMOND_RAISED_DARK,
        pressed: REDMOND_PRESSED_DARK,
    },
    default_button: DefaultButtonStyle::Outline(Stroke::hairline(REDMOND_GLYPH_DARK)),
};

const AQUA_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: rgb8(236, 236, 236),
    text: rgb8(29, 29, 31),
    caret: Stroke::new(MODERN_CARET, rgb8(29, 29, 31)),
    // Aqua has no borders to find a control by, so the text field has to be told apart from the
    // dialog by its fill alone — clearly lighter here, and clearly darker in dark mode, as macOS
    // does.
    field: WHITE,
    selection: rgb8(180, 213, 254),
    selection_text: rgb8(11, 61, 145),
    idle: ControlPaint::new(WHITE, Stroke::NONE),
    hover: ControlPaint::new(rgb8(232, 232, 232), Stroke::NONE),
    // Accent belongs to the primary action. An ordinary push button darkens neutrally while held
    // so it responds without briefly claiming primary status.
    pressed: ControlPaint::new(rgb8(205, 205, 207), Stroke::NONE),
    metrics: WidgetMetrics {
        button_padding: (13.0, 4.0),
        control_height: 24.0,
        item_spacing: (10.0, 6.0),
        // Modern macOS rounds controls generously without turning ordinary dialog fields and push
        // buttons into the full capsules used by iOS.
        corner_radius: 6,
    },
    font: Face::Inter,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(0, 122, 255),
        hover: rgb8(20, 132, 255),
        active: rgb8(0, 96, 205),
        text: WHITE,
        border: Stroke::NONE,
    },
};

const AQUA_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: rgb8(50, 50, 52),
    text: rgb8(245, 245, 247),
    caret: Stroke::new(MODERN_CARET, rgb8(245, 245, 247)),
    field: rgb8(28, 28, 30),
    selection: rgb8(44, 74, 110),
    selection_text: rgb8(207, 226, 255),
    idle: ControlPaint::new(rgb8(86, 86, 88), Stroke::NONE),
    hover: ControlPaint::new(rgb8(110, 110, 112), Stroke::NONE),
    pressed: ControlPaint::new(rgb8(75, 75, 78), Stroke::NONE),
    metrics: AQUA_WIDGETS.metrics,
    font: Face::Inter,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(10, 132, 255),
        hover: rgb8(40, 146, 255),
        active: rgb8(0, 105, 220),
        text: WHITE,
        border: Stroke::NONE,
    },
};

const ADWAITA_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: rgb8(250, 250, 250),
    // Cantarell in egui's generic mid-grey made ordinary GNOME content look insensitive. Adwaita's
    // own foreground tones instead; weak text (including placeholders) stays muted independently.
    text: ADWAITA_TEXT,
    caret: Stroke::new(MODERN_CARET, ADWAITA_TEXT),
    field: WHITE,
    selection: rgb8(197, 221, 246),
    selection_text: rgb8(20, 84, 140),
    // Darker than the headerbar-grey panel behind it: Adwaita's own dialog buttons are roughly 10%
    // black over the window background rather than the near-white tried first, which left them a
    // couple of levels from the surface and invisible. The hairline matters for the same reason —
    // egui draws text fields with the same visuals as buttons, and an unbordered white field on a
    // near-white panel cannot be found at all.
    idle: ControlPaint::new(rgb8(225, 222, 219), Stroke::hairline(rgb8(205, 199, 194))),
    hover: ControlPaint::new(rgb8(201, 197, 193), Stroke::hairline(rgb8(205, 199, 194))),
    pressed: ControlPaint::new(rgb8(180, 176, 171), Stroke::hairline(rgb8(205, 199, 194))),
    metrics: WidgetMetrics {
        button_padding: (14.0, 7.0),
        control_height: 34.0,
        item_spacing: (8.0, 6.0),
        // Libadwaita rounds everything to the same generous radius.
        corner_radius: 8,
    },
    font: Face::Cantarell,
    font_size: 14.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(53, 132, 228),
        hover: rgb8(70, 145, 232),
        active: rgb8(38, 112, 204),
        text: WHITE,
        border: Stroke::NONE,
    },
};

const ADWAITA_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: rgb8(36, 36, 36),
    text: rgb8(246, 245, 244),
    caret: Stroke::new(MODERN_CARET, rgb8(246, 245, 244)),
    field: rgb8(30, 30, 30),
    selection: rgb8(38, 69, 107),
    selection_text: rgb8(214, 230, 250),
    idle: ControlPaint::new(rgb8(56, 56, 56), Stroke::hairline(rgb8(74, 74, 74))),
    hover: ControlPaint::new(rgb8(79, 79, 79), Stroke::hairline(rgb8(74, 74, 74))),
    pressed: ControlPaint::new(rgb8(96, 96, 96), Stroke::hairline(rgb8(74, 74, 74))),
    metrics: ADWAITA_WIDGETS.metrics,
    font: Face::Cantarell,
    font_size: 14.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(28, 113, 216),
        hover: rgb8(48, 128, 222),
        active: rgb8(20, 92, 180),
        text: WHITE,
        border: Stroke::NONE,
    },
};

const BREEZE_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: rgb8(239, 240, 241),
    text: rgb8(35, 38, 41),
    caret: Stroke::new(MODERN_CARET, rgb8(35, 38, 41)),
    field: WHITE,
    selection: rgb8(183, 225, 247),
    selection_text: rgb8(0, 82, 120),
    idle: ControlPaint::new(rgb8(247, 247, 247), Stroke::hairline(rgb8(189, 195, 199))),
    hover: ControlPaint::new(rgb8(225, 228, 230), Stroke::hairline(rgb8(189, 195, 199))),
    pressed: ControlPaint::new(rgb8(207, 212, 215), Stroke::hairline(rgb8(189, 195, 199))),
    metrics: WidgetMetrics {
        button_padding: (10.0, 5.0),
        control_height: 30.0,
        item_spacing: (8.0, 6.0),
        corner_radius: 3,
    },
    font: Face::NotoSans,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(61, 174, 233),
        hover: rgb8(83, 184, 236),
        active: rgb8(41, 143, 197),
        text: WHITE,
        border: Stroke::NONE,
    },
};

const BREEZE_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: rgb8(49, 54, 59),
    text: rgb8(239, 240, 241),
    caret: Stroke::new(MODERN_CARET, rgb8(239, 240, 241)),
    field: rgb8(35, 38, 41),
    selection: rgb8(38, 88, 112),
    selection_text: rgb8(224, 246, 255),
    idle: ControlPaint::new(rgb8(59, 64, 69), Stroke::hairline(rgb8(97, 102, 107))),
    hover: ControlPaint::new(rgb8(82, 90, 98), Stroke::hairline(rgb8(97, 102, 107))),
    pressed: ControlPaint::new(rgb8(98, 108, 116), Stroke::hairline(rgb8(97, 102, 107))),
    metrics: BREEZE_WIDGETS.metrics,
    font: Face::NotoSans,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    // Breeze's accent is the same in both palettes — the blue is the identity, not a light-mode
    // choice.
    default_button: BREEZE_WIDGETS.default_button,
};

/// Mac OS 9 has no dark counterpart, so this is what a dark request resolves to as well.
const PLATINUM_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: PLATINUM_FACE,
    text: BLACK,
    caret: Stroke::new(PERIOD_CARET, BLACK),
    field: WHITE,
    selection: rgb8(198, 208, 224),
    selection_text: rgb8(0, 0, 128),
    idle: ControlPaint::new(PLATINUM_FACE, Stroke::hairline(rgb8(85, 85, 85))),
    // Darkens on hover rather than lightening, which leaves room for the press state.
    hover: ControlPaint::new(rgb8(198, 198, 198), Stroke::hairline(rgb8(85, 85, 85))),
    pressed: ControlPaint::new(rgb8(170, 170, 170), Stroke::hairline(rgb8(85, 85, 85))),
    metrics: WidgetMetrics {
        button_padding: (10.0, 3.0),
        control_height: 20.0,
        item_spacing: (6.0, 4.0),
        // Square, not the 3pt tried first: `bevel::button` paints a bevel on square corners, and a
        // rounded fill under square edges reads as a mistake. Mac OS 9's own controls are
        // near-square anyway.
        corner_radius: 0,
    },
    font: Face::SourceSans,
    font_size: 12.0,
    edge: WidgetEdge::Bevel {
        raised: PLATINUM_RAISED,
        pressed: PLATINUM_PRESSED,
    },
    default_button: DefaultButtonStyle::Outline(Stroke::hairline(BLACK)),
};

const CDE_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: CDE_FACE,
    text: BLACK,
    caret: Stroke::new(PERIOD_CARET, BLACK),
    field: WHITE,
    selection: rgb8(175, 205, 202),
    selection_text: rgb8(20, 72, 70),
    idle: ControlPaint::new(CDE_FACE, Stroke::hairline(CDE_SHADOW)),
    hover: ControlPaint::new(rgb8(205, 205, 194), Stroke::hairline(CDE_SHADOW)),
    pressed: ControlPaint::new(rgb8(151, 151, 143), Stroke::hairline(CDE_SHADOW)),
    metrics: WidgetMetrics {
        button_padding: (9.0, 3.0),
        control_height: 22.0,
        item_spacing: (6.0, 4.0),
        corner_radius: 0,
    },
    font: Face::LiberationSans,
    font_size: 12.0,
    edge: WidgetEdge::Bevel {
        raised: CDE_RAISED,
        pressed: CDE_PRESSED,
    },
    default_button: DefaultButtonStyle::Outline(Stroke::hairline(BLACK)),
};

const CDE_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: CDE_PANEL_DARK,
    text: rgb8(235, 240, 232),
    caret: Stroke::new(PERIOD_CARET, rgb8(235, 240, 232)),
    field: rgb8(30, 33, 32),
    selection: rgb8(51, 93, 90),
    selection_text: rgb8(232, 246, 241),
    idle: ControlPaint::new(CDE_FACE_DARK, Stroke::hairline(CDE_SHADOW_DARK)),
    hover: ControlPaint::new(rgb8(111, 119, 115), Stroke::hairline(CDE_SHADOW_DARK)),
    pressed: ControlPaint::new(rgb8(67, 72, 70), Stroke::hairline(CDE_SHADOW_DARK)),
    metrics: CDE_WIDGETS.metrics,
    font: Face::LiberationSans,
    font_size: 12.0,
    edge: WidgetEdge::Bevel {
        raised: CDE_RAISED_DARK,
        pressed: CDE_PRESSED_DARK,
    },
    default_button: DefaultButtonStyle::Outline(Stroke::hairline(rgb8(235, 240, 232))),
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Perceived brightness, for the contrast floors below. The engine has its own copy over
    /// `egui::Color32`, for the tests that check a *projected* style rather than the data.
    fn luminance(color: Color) -> f32 {
        0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b
    }

    /// Every theme's metrics, so the properties below hold for the whole catalogue rather than
    /// just whichever one happens to be default.
    const ALL: &[Metrics] = &[Metrics::PLAIN];

    /// Stand-ins for themes yet to be written, so the geometry seam is exercised against sizes
    /// other than `plain`'s before any of them exist: a thick multi-pixel bevel (Win95) and a
    /// tall header (Adwaita), plus an undecorated-looking borderless case.
    const FUTURE: &[Metrics] = &[
        Metrics {
            header_height: 18,
            border_width: 4,
        },
        Metrics {
            header_height: 37,
            border_width: 1,
        },
        Metrics {
            header_height: 32,
            border_width: 0,
        },
    ];

    fn every_metric() -> impl Iterator<Item = &'static Metrics> {
        ALL.iter().chain(FUTURE)
    }

    #[test]
    fn plain_metrics_are_the_documented_contract() {
        // Modes may do arithmetic against `outer_*`, so these are a promise: any mode naming
        // `plain` gets these exact numbers, on every platform and every version.
        assert_eq!(Theme::Plain.metrics().header_height, 20);
        assert_eq!(Theme::Plain.metrics().border_width, 1);
    }

    #[test]
    fn padding_covers_both_borders_but_only_one_header() {
        for metrics in every_metric() {
            let (x, y) = metrics.outer_padding();
            assert_eq!(x, metrics.border_width * 2, "{metrics:?}");
            assert_eq!(
                y,
                metrics.border_width * 2 + metrics.header_height,
                "{metrics:?}"
            );
        }
    }

    // ── The catalogue ────────────────────────────────────────────────────────────

    /// Every alias resolves to a theme that is actually in the catalogue, on whatever platform
    /// this is compiled for — the `cfg` arms are easy to get wrong and impossible to notice.
    #[test]
    fn every_choice_resolves_into_the_catalogue() {
        for choice in [
            ThemeChoice::Plain,
            ThemeChoice::Fluent,
            ThemeChoice::Redmond,
            ThemeChoice::Aqua,
            ThemeChoice::Adwaita,
            ThemeChoice::Breeze,
            ThemeChoice::Platinum,
            ThemeChoice::Cde,
            ThemeChoice::Native,
            ThemeChoice::NativeRetro,
        ] {
            let theme = choice.resolve();
            assert!(
                ALL_THEMES.contains(&theme),
                "{choice:?} resolved to {theme:?}, which is not in the catalogue"
            );
        }
    }

    /// The catalogue is what `config/` builds its picker from; [`ThemeChoice`] is what can
    /// actually be asked for. A look in one and not the other is either a theme no user can pick
    /// or a setting that silently does nothing, so the two lists have to agree exactly — including
    /// on order, which is the order the picker offers them in (aliases first, then newest to
    /// oldest).
    #[test]
    fn the_catalogue_lists_exactly_what_can_be_asked_for() {
        let engine: Vec<&str> = ThemeChoice::ALL.iter().map(|c| c.name()).collect();
        let catalogue: Vec<&str> = THEMES.iter().map(|t| t.name).collect();
        assert_eq!(engine, catalogue);

        let engine: Vec<&str> = AppearanceChoice::ALL.iter().map(|c| c.name()).collect();
        let catalogue: Vec<&str> = APPEARANCES.iter().map(|a| a.name).collect();
        assert_eq!(engine, catalogue);
    }

    /// The catalogue tells the user which looks have no dark palette. That claim has to come from
    /// the palettes themselves rather than being a note someone kept up to date by hand.
    #[test]
    fn the_catalogue_agrees_about_dark_support() {
        for info in THEMES {
            let choice = ThemeChoice::from_name(info.name).expect("checked by the test above");
            assert_eq!(
                info.is_alias,
                matches!(choice, ThemeChoice::Native | ThemeChoice::NativeRetro),
                "{}",
                info.name
            );

            // Concrete looks only. An alias resolves per machine, so no single answer here could
            // be right everywhere -- `native-retro` is Windows 95 on one platform and Mac OS 9,
            // which has no dark variant at all, on another. Whoever resolves the alias works out
            // the real answer from what it resolved to.
            if !info.is_alias {
                assert_eq!(
                    info.supports_dark,
                    choice.resolve().supports_dark(),
                    "{}",
                    info.name
                );
            }
        }
    }

    /// A drawn look can always be named, and the name is one the catalogue offers — which is what
    /// lets `config/` say "match my system" *is* KDE Plasma here rather than showing both.
    #[test]
    fn every_drawable_look_names_itself_into_the_catalogue() {
        for &drawn in ALL_THEMES {
            let info = theme(drawn.name())
                .unwrap_or_else(|| panic!("{drawn:?} names {:?}, absent here", drawn.name()));
            assert!(!info.is_alias, "{drawn:?} names an alias");
            assert_eq!(
                ThemeChoice::from_name(drawn.name()).map(ThemeChoice::resolve),
                Some(drawn)
            );
        }
    }

    /// Named choices are pass-throughs, so a mode that names a look gets exactly that look
    /// regardless of platform. Only the two aliases may vary.
    #[test]
    fn naming_a_theme_is_never_platform_dependent() {
        assert_eq!(ThemeChoice::Plain.resolve(), Theme::Plain);
        assert_eq!(ThemeChoice::Fluent.resolve(), Theme::Fluent);
        assert_eq!(ThemeChoice::Redmond.resolve(), Theme::Redmond);
        assert_eq!(ThemeChoice::Aqua.resolve(), Theme::Aqua);
        assert_eq!(ThemeChoice::Adwaita.resolve(), Theme::Adwaita);
        assert_eq!(ThemeChoice::Breeze.resolve(), Theme::Breeze);
        assert_eq!(ThemeChoice::Platinum.resolve(), Theme::Platinum);
        assert_eq!(ThemeChoice::Cde.resolve(), Theme::Cde);
    }

    /// The default a window falls back to is the *user's*, and on the theme axis that is a real
    /// look rather than the characterless `plain`. `plain` is a value the user can pick, not the
    /// one they get by saying nothing.
    #[test]
    fn the_fallback_is_the_users_own_look() {
        assert_eq!(ChromeDefaults::default().theme, ThemeChoice::Native);
        assert_ne!(ChromeDefaults::default().theme, ThemeChoice::Plain);
    }

    /// `native` and `native-retro` must not resolve to the same look, or the retro alias is
    /// pointless on that platform.
    #[test]
    fn native_and_native_retro_differ() {
        assert_ne!(
            ThemeChoice::Native.resolve(),
            ThemeChoice::NativeRetro.resolve()
        );
    }

    /// Every theme is distinguishable from every other by its chrome — two identical entries
    /// would mean one of them is not pulling its weight in the catalogue.
    #[test]
    fn no_two_themes_look_the_same() {
        for (i, &a) in ALL_THEMES.iter().enumerate() {
            for &b in &ALL_THEMES[i + 1..] {
                assert_ne!(
                    (a.metrics(), a.chrome(Appearance::Light)),
                    (b.metrics(), b.chrome(Appearance::Light)),
                    "{a:?} and {b:?} are identical"
                );
            }
        }
    }

    /// Serde names are the Lua-facing spelling, so a typo silently changes the API.
    #[test]
    fn choices_serialise_to_their_lua_names() {
        let name = |choice: ThemeChoice| serde_json::to_string(&choice).unwrap();

        assert_eq!(name(ThemeChoice::Plain), "\"plain\"");
        assert_eq!(name(ThemeChoice::Fluent), "\"fluent\"");
        assert_eq!(name(ThemeChoice::Redmond), "\"redmond\"");
        assert_eq!(name(ThemeChoice::Aqua), "\"aqua\"");
        assert_eq!(name(ThemeChoice::Adwaita), "\"adwaita\"");
        assert_eq!(name(ThemeChoice::Breeze), "\"breeze\"");
        assert_eq!(name(ThemeChoice::Platinum), "\"platinum\"");
        assert_eq!(name(ThemeChoice::Cde), "\"cde\"");
        assert_eq!(name(ThemeChoice::Native), "\"native\"");
        assert_eq!(name(ThemeChoice::NativeRetro), "\"native-retro\"");
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn linux_native_distinguishes_plasma_from_other_desktops() {
        assert_eq!(ThemeChoice::native_unix("KDE"), Theme::Breeze);
        assert_eq!(ThemeChoice::native_unix("ubuntu:KDE"), Theme::Breeze);
        assert_eq!(ThemeChoice::native_unix("Plasma"), Theme::Breeze);
        assert_eq!(ThemeChoice::native_unix("GNOME"), Theme::Adwaita);
        assert_eq!(ThemeChoice::native_unix(""), Theme::Adwaita);
        assert_eq!(ThemeChoice::native_retro(), Theme::Cde);
    }

    /// A header has to be tall enough for the title and buttons to fit, and a border thin enough
    /// to leave a usable content area on a small popup.
    #[test]
    fn every_theme_has_workable_metrics() {
        for &theme in ALL_THEMES {
            let metrics = theme.metrics();
            assert!(
                (12..=48).contains(&metrics.header_height),
                "{theme:?} header height {} is out of range",
                metrics.header_height
            );
            assert!(
                metrics.border_width <= 4,
                "{theme:?} border {} is too thick",
                metrics.border_width
            );

            // A button must fit inside the bar it sits in.
            for button in theme.chrome(Appearance::Light).buttons.buttons {
                assert!(
                    button.width_ratio > 0.0 && button.width_ratio <= 2.0,
                    "{theme:?} button ratio {} is implausible",
                    button.width_ratio
                );
            }
        }
    }

    /// Any button a theme draws must be reachable: the cluster cannot be wider than the bar it
    /// sits in, or the outermost buttons would be clipped off a narrow popup.
    #[test]
    fn button_clusters_fit_a_narrow_popup() {
        const NARROW: f32 = 120.0;

        for &theme in ALL_THEMES {
            let layout = theme.chrome(Appearance::Light).buttons;
            let height = theme.metrics().header_height as f32;
            let widths: f32 = layout.buttons.iter().map(|b| b.width_ratio * height).sum();
            let gaps = layout.gap * layout.buttons.len().saturating_sub(1) as f32;
            let extent = layout.inset + widths + gaps;

            assert!(
                extent < NARROW,
                "{theme:?} needs {extent} logical px of buttons, more than a {NARROW}px popup"
            );
        }
    }

    /// `redmond` is the one theme with a multi-ring border, and its rings must read as a raised
    /// 3D edge: light on the top-left, dark on the bottom-right.
    #[test]
    fn redmond_bevels_are_raised_not_flat() {
        let border = Theme::Redmond.chrome(Appearance::Light).border;
        assert_eq!(border.len(), 3);

        for (index, ring) in border.iter().take(2).enumerate() {
            let (light, dark) = (ring.top_left(), ring.bottom_right());
            let brightness = |c: Color| c.r + c.g + c.b;
            assert!(
                brightness(light) > brightness(dark),
                "ring {index} is not raised: {light:?} vs {dark:?}"
            );
        }
    }

    /// Aqua's inert lights are uncoloured and glyphless in every state — the honesty rule. A
    /// coloured light that did nothing would be the violation, not the grey one.
    #[test]
    fn aquas_inert_lights_never_look_active() {
        let buttons = Theme::Aqua.chrome(Appearance::Light).buttons.buttons;
        assert_eq!(buttons.len(), 3, "aqua keeps its three-light silhouette");

        let inert: Vec<_> = buttons
            .iter()
            .filter(|b| b.action == ButtonAction::Inert)
            .collect();
        assert_eq!(inert.len(), 2, "minimise and zoom are both inert");

        for button in inert {
            assert_eq!(button.glyph, Glyph::None);
            assert_eq!(button.idle, button.hover, "inert light reacts to hover");
            assert_eq!(button.idle, button.active, "inert light reacts to a press");
            // Both ends of the dimensional fill stay grey, not canonical yellow/green.
            let Fill::VerticalGradient { from, to } = button.idle.fill else {
                panic!("inert light is not deliberately shaded");
            };
            for fill in [from, to] {
                assert_eq!(fill.r, fill.g, "inert light is coloured");
                assert_eq!(fill.g, fill.b, "inert light is coloured");
            }
            assert!(
                button.idle.rim.is_some(),
                "inert light has no intentional rim"
            );
        }
    }

    /// Aqua hides its close glyph until hovered, which is the authentic behaviour — and means the
    /// idle glyph colour is deliberately invisible rather than accidentally so.
    #[test]
    fn aquas_close_glyph_appears_on_hover() {
        let close = &Theme::Aqua.chrome(Appearance::Light).buttons.buttons[0];
        assert_eq!(close.action, ButtonAction::Close);
        assert_eq!(close.idle.glyph.a, 0.0, "idle glyph should be invisible");
        assert!(close.hover.glyph.a > 0.0, "hover glyph should be visible");
    }

    #[test]
    fn aquas_controls_are_mac_rounded_and_secondary_press_stays_neutral() {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let widgets = Theme::Aqua.widgets(appearance);
            assert_eq!(widgets.metrics.corner_radius, 6);

            let fill = widgets.pressed.fill;
            assert_eq!(fill.r, fill.g, "{appearance:?}");
            assert!((fill.r - fill.b).abs() <= 3.0 / 255.0, "{appearance:?}");
        }
    }

    #[test]
    fn adwaita_uses_its_own_foreground_in_both_appearances() {
        assert_eq!(Theme::Adwaita.widgets(Appearance::Light).text, ADWAITA_TEXT);
        assert_eq!(
            Theme::Adwaita.widgets(Appearance::Dark).text,
            rgb8(246, 245, 244)
        );
    }

    /// The two Mac themes put their close control on the left, which is the placement that made
    /// button side theme data in the first place.
    #[test]
    fn the_mac_themes_close_on_the_left() {
        assert_eq!(
            Theme::Aqua.chrome(Appearance::Light).buttons.side,
            Side::Left
        );
        assert_eq!(
            Theme::Platinum.chrome(Appearance::Light).buttons.side,
            Side::Left
        );
    }

    /// A theme's title bar and its widgets must be set in the same family. A heavier companion
    /// face is fine; an unrelated typeface above and below the header line is the clearest sign a
    /// theme is only skin-deep.
    #[test]
    fn a_themes_title_matches_its_widgets() {
        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let title = theme.chrome(appearance).title.font;
                let widgets = theme.widget_font();
                assert!(
                    title == widgets
                        || matches!(
                            (title, widgets),
                            (Face::SourceSansSemibold, Face::SourceSans)
                                | (Face::LiberationSansBold, Face::LiberationSans)
                        ),
                    "{theme:?} {appearance:?}: {title:?} does not match {widgets:?}"
                );
            }
        }
    }

    /// A pressed bevel is the raised one turned inside out — ring for ring, the two tones swapped.
    ///
    /// That inversion is what makes a control look like it sinks rather than merely changes colour,
    /// and it is easy to get subtly wrong by editing one palette and not the other.
    #[test]
    fn a_pressed_bevel_inverts_the_raised_one() {
        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let WidgetEdge::Bevel { raised, pressed } = theme.widget_edge(appearance) else {
                    continue;
                };

                assert_eq!(
                    raised.len(),
                    pressed.len(),
                    "{theme:?} {appearance:?}: the two states have different ring counts"
                );

                for (index, (up, down)) in raised.iter().zip(pressed).enumerate() {
                    assert_eq!(
                        up.top_left(),
                        down.bottom_right(),
                        "{theme:?} {appearance:?} ring {index} is not inverted when pressed"
                    );
                    assert_eq!(
                        up.bottom_right(),
                        down.top_left(),
                        "{theme:?} {appearance:?} ring {index} is not inverted when pressed"
                    );
                }
            }
        }
    }

    /// Only the themes whose real controls are three-dimensional take the hand-painted path; the
    /// flat platforms and `plain` stay on egui's own painting, which is the point of keeping the
    /// custom drawing narrow.
    #[test]
    fn only_the_retro_themes_are_bevelled() {
        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let bevelled = matches!(theme.widget_edge(appearance), WidgetEdge::Bevel { .. });
                let expected = matches!(theme, Theme::Redmond | Theme::Platinum | Theme::Cde);
                assert_eq!(bevelled, expected, "{theme:?} {appearance:?}");
            }
        }
    }

    // ── Appearance ───────────────────────────────────────────────────────────────

    const BOTH: [Appearance; 2] = [Appearance::Light, Appearance::Dark];

    /// The rule the whole appearance axis rests on: **it never changes geometry.** Metrics take no
    /// appearance at all, so the risk is in `Chrome` — a dark variant with an extra border ring or
    /// a differently-sized button would move the content area, and the outer size was already
    /// fixed from the light metrics before the palette was even known.
    #[test]
    fn appearance_never_changes_geometry() {
        for &theme in ALL_THEMES {
            let light = theme.chrome(Appearance::Light);
            let dark = theme.chrome(Appearance::Dark);

            assert_eq!(
                light.border.len(),
                dark.border.len(),
                "{theme:?} border ring count"
            );
            assert_eq!(
                light.buttons.side, dark.buttons.side,
                "{theme:?} button side"
            );
            assert_eq!(light.buttons.inset, dark.buttons.inset, "{theme:?} inset");
            assert_eq!(light.buttons.gap, dark.buttons.gap, "{theme:?} gap");
            assert_eq!(
                light.buttons.buttons.len(),
                dark.buttons.buttons.len(),
                "{theme:?} button count"
            );

            for (index, (l, d)) in light
                .buttons
                .buttons
                .iter()
                .zip(dark.buttons.buttons)
                .enumerate()
            {
                assert_eq!(
                    l.width_ratio, d.width_ratio,
                    "{theme:?} button {index} width"
                );
                assert_eq!(l.shape, d.shape, "{theme:?} button {index} shape");
                assert_eq!(l.action, d.action, "{theme:?} button {index} action");
                assert_eq!(l.glyph, d.glyph, "{theme:?} button {index} glyph");
            }

            // The title's size and placement are layout too, even though its colour is not.
            assert_eq!(light.title.size, dark.title.size, "{theme:?} title size");
            assert_eq!(
                light.title.padding, dark.title.padding,
                "{theme:?} title padding"
            );
            assert_eq!(light.title.align, dark.title.align, "{theme:?} title align");
            assert_eq!(light.title.font, dark.title.font, "{theme:?} title font");
        }
    }

    /// Border rings still match the reserved width in *both* palettes — the layout-versus-paint
    /// invariant, now with twice as many ways to break it.
    #[test]
    fn border_rings_match_the_border_width_in_both_palettes() {
        for &theme in ALL_THEMES {
            for appearance in BOTH {
                assert_eq!(
                    theme.chrome(appearance).border.len() as u32,
                    theme.metrics().border_width,
                    "{theme:?} {appearance:?}"
                );
            }
        }
    }

    #[test]
    fn platinum_is_the_only_theme_without_a_dark_variant() {
        for &theme in ALL_THEMES {
            assert_eq!(theme.supports_dark(), theme != Theme::Platinum, "{theme:?}");
        }
    }

    /// A title has to be readable on its own bar, in either palette.
    ///
    /// Never covered before: there was a check that a *dark* bar carries light text, but nothing
    /// stopping a new light palette from putting pale text on a pale bar. The floor sits below the
    /// catalogue's current minimum (`aqua` dark, at 118 — macOS's own muted secondary grey) and well
    /// above anything unreadable.
    #[test]
    fn every_title_is_readable_on_its_bar() {
        const FLOOR: f32 = 90.0 / 255.0;

        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let chrome = theme.chrome(appearance);
                let bar = match chrome.header {
                    Fill::Solid(color) => color,
                    // The lighter end of a gradient is the harder case for dark text.
                    Fill::VerticalGradient { from, to } => {
                        if luminance(from) > luminance(to) {
                            from
                        } else {
                            to
                        }
                    }
                    Fill::Pinstripe { base, .. } => base,
                };

                let contrast = (luminance(chrome.title.color) - luminance(bar)).abs();
                assert!(
                    contrast >= FLOOR,
                    "{theme:?} {appearance:?}: the title is only {contrast:.2} from its bar"
                );
            }
        }
    }

    /// A close button's mark has to be readable on the button itself, in every state it is drawn in.
    ///
    /// `aqua` is the exception by design: its glyph is transparent until hovered, which is what
    /// macOS does, so only the states where a mark is actually painted are checked.
    #[test]
    fn every_close_glyph_is_readable_on_its_button() {
        const FLOOR: f32 = 80.0 / 255.0;

        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let chrome = theme.chrome(appearance);
                let Some(close) = chrome
                    .buttons
                    .buttons
                    .iter()
                    .find(|button| button.action == ButtonAction::Close)
                else {
                    continue;
                };
                if close.glyph == Glyph::None {
                    continue;
                }

                for (state, paint) in [
                    ("idle", close.idle),
                    ("hover", close.hover),
                    ("pressed", close.active),
                ] {
                    // Nothing is drawn when the mark is transparent.
                    if paint.glyph.a == 0.0 {
                        continue;
                    }

                    let Fill::Solid(fill) = paint.fill else {
                        continue;
                    };
                    let contrast = (luminance(paint.glyph) - luminance(fill)).abs();
                    assert!(
                        contrast >= FLOOR,
                        "{theme:?} {appearance:?} {state}: the close mark is only \
                         {contrast:.2} from its button"
                    );
                }
            }
        }
    }

    /// And the title has to stay readable against it: light text on the dark bar.
    #[test]
    fn a_dark_header_carries_lighter_title_text() {
        for &theme in ALL_THEMES.iter().filter(|t| t.supports_dark()) {
            let title = theme.chrome(Appearance::Dark).title.color;
            let brightness = title.r + title.g + title.b;
            assert!(
                brightness > 1.5,
                "{theme:?} dark title text is too dark: {brightness}"
            );
        }
    }

    // ── Widgets as data ──────────────────────────────────────────────────────────

    /// A typeface is not a light/dark choice. The two accessors that ignore appearance
    /// (`widget_font`, `widget_font_size`) read the light palette and would quietly be wrong for
    /// dark if a theme ever disagreed with itself here.
    #[test]
    fn a_font_never_depends_on_the_palette() {
        for &theme in ALL_THEMES {
            let light = theme.widgets(Appearance::Light);
            let dark = theme.widgets(Appearance::Dark);
            assert_eq!(light.font, dark.font, "{theme:?}");
            assert_eq!(light.font_size, dark.font_size, "{theme:?}");
        }
    }

    /// The whole point of the data-first shape: a theme is plain data, so it can be read by
    /// something that cannot link this crate -- `config/`, to show a user what they are choosing.
    /// An egui type anywhere in `Widgets` would break this at compile time, and this test is what
    /// notices if one is reintroduced behind a `#[serde(skip)]`.
    #[test]
    fn every_widget_palette_survives_the_trip_out_of_this_crate() {
        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let widgets = theme.widgets(appearance);
                let json = serde_json::to_string(widgets)
                    .unwrap_or_else(|e| panic!("{theme:?}/{appearance:?}: {e}"));

                // Spot-check that it is the theme's own values coming out, not a defaulted shell.
                assert!(json.contains("\"panel\""), "{theme:?}");
                assert!(json.contains("\"font\""), "{theme:?}");
                assert!(json.contains("\"default_button\""), "{theme:?}");
            }
        }
    }

    /// Text used to come from egui: a mid-grey body colour plus a per-state ramp that darkened a
    /// button's own label as the pointer crossed it. Now every theme states one colour, and it has
    /// to be legible on all three surfaces it can land on. Nothing checked this before, because
    /// there was nothing to check -- half the catalogue had no text colour of its own.
    #[test]
    fn every_theme_states_readable_text() {
        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let w = theme.widgets(appearance);
                let text = luminance(w.text);

                for (what, surface) in [
                    ("panel", w.panel),
                    ("control", w.idle.fill),
                    ("hovered control", w.hover.fill),
                    ("held control", w.pressed.fill),
                    ("text field", w.field),
                ] {
                    let contrast = (text - luminance(surface)).abs();
                    assert!(
                        contrast > 0.35,
                        "{theme:?}/{appearance:?}: text on {what} is {contrast:.2}"
                    );
                }
            }
        }
    }

    /// The caret is the other value that used to be inherited -- egui's #00537d/#c0deff blue,
    /// which belonged to no theme here. It has to be visible in the field it blinks in.
    #[test]
    fn every_caret_is_visible_in_its_field() {
        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let w = theme.widgets(appearance);
                assert!(w.caret.width > 0.0, "{theme:?}/{appearance:?}");

                let contrast = (luminance(w.caret.color) - luminance(w.field)).abs();
                assert!(
                    contrast > 0.35,
                    "{theme:?}/{appearance:?}: caret contrast is {contrast:.2}"
                );
            }
        }
    }

    /// `aqua` keeps its lights coloured in dark mode — macOS does not grey them — but the inert
    /// pair must still be uncoloured and unreactive in both palettes.
    #[test]
    fn aquas_inert_lights_stay_honest_in_dark_too() {
        for appearance in BOTH {
            let buttons = Theme::Aqua.chrome(appearance).buttons.buttons;

            let close = &buttons[0];
            assert_eq!(close.action, ButtonAction::Close);
            let Fill::VerticalGradient { from, to } = close.idle.fill else {
                panic!("aqua's close light is not shaded");
            };
            // Still red, not greyed: dark mode does not disable it.
            for fill in [from, to] {
                assert!(fill.r > fill.g && fill.r > fill.b, "{appearance:?}");
            }

            for button in buttons.iter().filter(|b| b.action == ButtonAction::Inert) {
                assert_eq!(button.glyph, Glyph::None, "{appearance:?}");
                assert_eq!(button.idle, button.hover, "{appearance:?}");
                assert_eq!(button.idle, button.active, "{appearance:?}");

                let Fill::VerticalGradient { from, to } = button.idle.fill else {
                    panic!("aqua's inert light is not shaded");
                };
                for fill in [from, to] {
                    assert_eq!(fill.r, fill.g, "{appearance:?} inert light is coloured");
                    assert_eq!(fill.g, fill.b, "{appearance:?} inert light is coloured");
                }
                assert!(
                    button.idle.rim.is_some(),
                    "{appearance:?} inert light has no rim"
                );
            }
        }
    }

    /// `redmond` dark is a Win95 *appearance scheme* (Eggplant/Plum), not a modern dark mode, so it
    /// keeps the three-ring raised bevel that makes it Win95 at all.
    #[test]
    fn redmond_keeps_its_raised_bevel_in_dark() {
        let border = Theme::Redmond.chrome(Appearance::Dark).border;
        assert_eq!(border.len(), 3);

        for (index, ring) in border.iter().take(2).enumerate() {
            let brightness = |c: Color| c.r + c.g + c.b;
            assert!(
                brightness(ring.top_left()) > brightness(ring.bottom_right()),
                "dark ring {index} is not raised"
            );
        }
    }

    #[test]
    fn redmond_dark_keeps_a_clear_period_palette_hierarchy() {
        let brightness = |color: Color| color.r + color.g + color.b;
        let chrome = Theme::Redmond.chrome(Appearance::Dark);
        let Fill::Solid(title) = chrome.header else {
            panic!("redmond title bar should be solid");
        };
        let panel_brightness = brightness(Theme::Redmond.widgets(Appearance::Dark).panel);

        assert!(
            brightness(title) < panel_brightness,
            "title bar is not the strongest dark plane"
        );
        assert!(
            panel_brightness < brightness(REDMOND_FACE_DARK),
            "panel and raised controls do not separate"
        );
        assert!(
            brightness(REDMOND_BRIGHT_DARK) > brightness(REDMOND_HIGHLIGHT_DARK),
            "inner bevel highlight should be brightest"
        );
    }

    // ── Chrome ───────────────────────────────────────────────────────────────────

    /// The invariant that keeps a border's layout and its paint in step: the reserved width comes
    /// from [`Metrics`], the painted rings from [`Chrome`], and one ring is one logical pixel. A
    /// theme with three rings and a two-pixel border would paint over its own content.
    #[test]
    fn border_rings_match_the_border_width() {
        for &theme in ALL_THEMES {
            assert_eq!(
                theme.chrome(Appearance::Light).border.len() as u32,
                theme.metrics().border_width,
                "{theme:?}"
            );
        }
    }

    /// A theme's buttons must contain at most one closable control — the close button is what
    /// `handle_mouse_up` reports, and two would make "was the window closed" ambiguous.
    #[test]
    fn a_theme_has_at_most_one_close_button() {
        for &theme in ALL_THEMES {
            let closers = theme
                .chrome(Appearance::Light)
                .buttons
                .buttons
                .iter()
                .filter(|b| b.action == ButtonAction::Close)
                .count();
            assert!(closers <= 1, "{theme:?} has {closers} close buttons");
        }
    }

    #[test]
    fn rings_partition_the_thickness_without_gaps_or_overlap() {
        for count in 1..=4usize {
            for thickness in 0..=12u32 {
                let mut previous_end = 0;
                for index in 0..count {
                    let (start, end) = ring_band(index, count, thickness);
                    assert_eq!(start, previous_end, "{count} rings, {thickness}px: gap");
                    assert!(end >= start, "{count} rings, {thickness}px: inverted");
                    previous_end = end;
                }
                assert_eq!(
                    previous_end, thickness,
                    "{count} rings did not fill {thickness}px"
                );
            }
        }
    }

    /// A single ring takes the whole thickness, which is what makes the HiDPI fix work: `plain`
    /// has one ring, so at 2x it paints both reserved pixels rather than only the outer one.
    #[test]
    fn a_single_ring_covers_everything() {
        for thickness in 0..=8 {
            assert_eq!(ring_band(0, 1, thickness), (0, thickness));
        }
    }

    #[test]
    fn a_uniform_ring_paints_both_edge_pairs_the_same() {
        let ring = BorderRing::Uniform(BLACK);
        assert_eq!(ring.top_left(), BLACK);
        assert_eq!(ring.bottom_right(), BLACK);
    }

    #[test]
    fn a_bevel_ring_keeps_its_two_colours_apart() {
        let ring = BorderRing::Bevel {
            top_left: WHITE,
            bottom_right: BLACK,
        };
        assert_eq!(ring.top_left(), WHITE);
        assert_eq!(ring.bottom_right(), BLACK);
    }

    // ── Widget styling ───────────────────────────────────────────────────────────

    /// Names reach the user's config file, so a duplicate or a stray rename is a settings-losing
    /// bug rather than a cosmetic one.
    #[test]
    fn every_name_is_unique() {
        for (i, theme) in THEMES.iter().enumerate() {
            assert!(
                !THEMES[i + 1..].iter().any(|t| t.name == theme.name),
                "duplicate theme name {}",
                theme.name
            );
        }
    }

    /// The defaults `AppConfig` hands out must be values the picker can actually show, or a fresh
    /// install opens on a setting that renders as blank.
    #[test]
    fn the_config_defaults_are_offerable_values() {
        let config = crate::user_config::AppConfig::default();
        assert!(theme(&config.theme).is_some(), "{}", config.theme);
        assert!(
            appearance(&config.appearance).is_some(),
            "{}",
            config.appearance
        );
    }

    #[test]
    fn an_unknown_name_is_none_rather_than_a_panic() {
        assert!(theme("some-future-theme").is_none());
        assert!(appearance("sepia").is_none());
    }
}
