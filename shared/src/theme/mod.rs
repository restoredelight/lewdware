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

mod appearance;
mod chrome;
mod color;
mod paint;
mod widgets;

pub use appearance::system_appearance;
pub use color::{Color, Face, TextAlign};
pub use paint::*;

use chrome::*;
use widgets::*;

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
        label: "macOS (Aqua)",
        supports_dark: true,
        is_alias: false,
    },
    ThemeInfo {
        name: "adwaita",
        label: "GNOME (Adwaita)",
        supports_dark: true,
        is_alias: false,
    },
    ThemeInfo {
        name: "breeze",
        label: "KDE Plasma (Breeze)",
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
        label: "CDE / Motif-inspired",
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
                // AdwHeaderBar's current minimum height. The old 37px value is specifically its
                // compact `.default-decoration`, not the centred application headerbar copied by
                // this theme.
                header_height: 47,
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

#[cfg(test)]
mod tests;
