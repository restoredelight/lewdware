//! Named looks for a popup's chrome and widgets. See `design/window-themes.md`.
//!
//! Two of the three halves exist so far:
//!
//! - [`Metrics`] is the single source of the border and title-bar sizes, which three separate
//!   places used to derive independently from a bare `HEADER_HEIGHT` constant (window sizing in
//!   `lua::api`, the decoration layout, and egui's cursor translation).
//! - [`window_style`] is the single source of the [`egui::Style`] a dialog's buttons and text
//!   fields are drawn with, which both render backends used to build separately.
//!
//! Chrome *paints* — header fill, border colours, the close button — are still literals in
//! `header.rs`.

use serde::{Deserialize, Serialize};
use winit::dpi::PhysicalUnit;

use crate::lua::{Color, TextAlign};
use crate::text_font::{self, Face};

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
    /// Mac OS 9: pinstriped bar, black frame, close box on the left.
    #[serde(rename = "platinum")]
    Platinum,
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
#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceChoice {
    #[serde(rename = "light")]
    #[default]
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
}

/// What a mode *asks* for, which may be an alias rather than a look.
///
/// Kept separate from [`Theme`] so that every `Theme` stays one concrete drawable appearance: the
/// platform branching lives here and nowhere else, and is resolved once — early, in
/// `PopupSpawnOpts::resolve` — so nothing downstream ever asks what OS it is on.
#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    #[serde(rename = "plain")]
    #[default]
    Plain,
    #[serde(rename = "fluent")]
    Fluent,
    #[serde(rename = "redmond")]
    Redmond,
    #[serde(rename = "aqua")]
    Aqua,
    #[serde(rename = "adwaita")]
    Adwaita,
    #[serde(rename = "platinum")]
    Platinum,
    /// Whatever this platform's current windows look like.
    #[serde(rename = "native")]
    Native,
    /// Whatever this platform's windows *used* to look like. Linux has no retro native look users
    /// would recognise, so it falls back to `redmond` — documented rather than pretended.
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
        Self::Platinum,
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
            Self::Platinum => "platinum",
            Self::Native => "native",
            Self::NativeRetro => "native-retro",
        }
    }

    pub fn resolve(self) -> Theme {
        match self {
            Self::Plain => Theme::Plain,
            Self::Fluent => Theme::Fluent,
            Self::Redmond => Theme::Redmond,
            Self::Aqua => Theme::Aqua,
            Self::Adwaita => Theme::Adwaita,
            Self::Platinum => Theme::Platinum,
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
        Theme::Adwaita
    }

    #[cfg(target_os = "windows")]
    fn native_retro() -> Theme {
        Theme::Redmond
    }
    #[cfg(target_os = "macos")]
    fn native_retro() -> Theme {
        Theme::Platinum
    }
    /// No retro Linux look is recognisable enough to be worth inventing one; `redmond` is the
    /// documented fallback rather than a pretence that it is native here.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    fn native_retro() -> Theme {
        Theme::Redmond
    }
}

/// Every theme, so exhaustiveness can be checked at run time by the tests as well as at compile
/// time by the matches below. Step 5 also offers this to `config/`, which needs to list the
/// catalogue in a dropdown.
#[allow(dead_code)]
pub const ALL_THEMES: &[Theme] = &[
    Theme::Plain,
    Theme::Fluent,
    Theme::Redmond,
    Theme::Aqua,
    Theme::Adwaita,
    Theme::Platinum,
];

impl Theme {
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
            Self::Platinum => Metrics {
                header_height: 20,
                border_width: 1,
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
    fn effective(self, appearance: Appearance) -> Appearance {
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
            (Self::Platinum, _) => PLATINUM_CHROME,
        }
    }

    /// The face this theme lays widget text out in.
    ///
    /// The single biggest lever on whether a theme reads as the platform it imitates — a Win95
    /// dialog set in a modern UI font is neither. Each is the closest freely-licensed match:
    /// Selawik is Microsoft's own metric-compatible substitute for Segoe UI, Inter the usual
    /// stand-in for San Francisco, and Cantarell *is* GNOME's UI font.
    pub fn widget_font(self) -> Face {
        match self {
            // `plain` imitates nothing, so it stays on egui's own font.
            Self::Plain => Face::Default,
            Self::Fluent => Face::Selawik,
            Self::Redmond => Face::Pixel,
            Self::Aqua => Face::Inter,
            Self::Adwaita => Face::Cantarell,
            // Mac OS 9's Charcoal has no freely-licensed equivalent; Inter is at least a neutral
            // humanist sans rather than egui's Ubuntu.
            Self::Platinum => Face::Inter,
        }
    }

    /// The size widget text is laid out at, in logical points. Platforms differ: Win95's UI is
    /// noticeably smaller than a modern one, and GNOME's is larger.
    pub fn widget_font_size(self) -> f32 {
        match self {
            Self::Plain => 13.0,
            Self::Fluent => 14.0,
            Self::Redmond => 12.0,
            Self::Aqua => 13.0,
            Self::Adwaita => 14.0,
            Self::Platinum => 12.0,
        }
    }

    /// Extra font data egui needs loaded before [`Self::widget_font`] can be used, or `None` when
    /// the built-in families are enough.
    pub fn widget_font_definitions(self) -> Option<egui::FontDefinitions> {
        text_font::build_font_definitions(self.widget_font())
    }

    /// How this theme edges a dialog's buttons.
    ///
    /// Only the two themes whose controls are genuinely three-dimensional ask for a bevel. The flat
    /// platforms — Windows 11, macOS, GNOME — are flat in life, and `plain` is deliberately the
    /// plainest thing here, so all four keep egui's own painting.
    pub fn widget_edge(self, appearance: Appearance) -> WidgetEdge {
        match self {
            Self::Redmond => match self.effective(appearance) {
                Appearance::Light => WidgetEdge::Bevel {
                    raised: REDMOND_RAISED,
                    pressed: REDMOND_PRESSED,
                },
                Appearance::Dark => WidgetEdge::Bevel {
                    raised: REDMOND_RAISED_DARK,
                    pressed: REDMOND_PRESSED_DARK,
                },
            },
            Self::Platinum => WidgetEdge::Bevel {
                raised: PLATINUM_RAISED,
                pressed: PLATINUM_PRESSED,
            },
            _ => WidgetEdge::Flat,
        }
    }

    /// The dialog's own background.
    ///
    /// `None` keeps egui's, which is what `plain` wants. Everything else needs its own: a Win95
    /// dialog *is* button-face grey, and leaving egui's near-white behind the chrome both looked
    /// wrong and left some themes' buttons within a few levels of the surface they sit on.
    ///
    /// Overridden in turn by a window's `background_color`, which has always won here.
    fn panel_fill(self, appearance: Appearance) -> Option<Color> {
        let (light, dark) = match self {
            Self::Plain => (rgb8(242, 242, 242), rgb8(26, 26, 26)),
            Self::Fluent => (rgb8(243, 243, 243), rgb8(32, 32, 32)),
            // Win95 dialogs are the same grey as their buttons; the buttons are told apart by their
            // border, not their fill.
            Self::Redmond => (REDMOND_FACE, rgb8(79, 75, 92)),
            Self::Aqua => (rgb8(236, 236, 236), rgb8(50, 50, 52)),
            Self::Adwaita => (rgb8(250, 250, 250), rgb8(36, 36, 36)),
            Self::Platinum => (rgb8(238, 238, 238), rgb8(238, 238, 238)),
        };

        Some(match self.effective(appearance) {
            Appearance::Light => light,
            Appearance::Dark => dark,
        })
    }

    /// The highlight behind selected text, and the colour of the text on top of it — which egui
    /// takes from the same `Selection::stroke` it uses for a focused field's outline.
    ///
    /// Both have to be set together: a saturated brand-colour highlight with egui's default text
    /// colour on top is unreadable, which is what every theme had. The fills here are therefore
    /// tints rather than the platforms' full accent colours, since egui has one text colour to
    /// spend on both the selection and the focus ring.
    fn selection(self, appearance: Appearance) -> Option<(Color, Color)> {
        let (light, dark) = match self {
            Self::Plain => ((rgb8(200, 200, 200), BLACK), (rgb8(89, 89, 89), WHITE)),
            Self::Fluent => (
                (rgb8(204, 228, 247), rgb8(0, 62, 107)),
                (rgb8(31, 58, 92), rgb8(207, 230, 255)),
            ),
            Self::Redmond => (
                (rgb8(195, 207, 230), rgb8(0, 0, 128)),
                (rgb8(74, 66, 96), rgb8(230, 225, 245)),
            ),
            Self::Aqua => (
                (rgb8(180, 213, 254), rgb8(11, 61, 145)),
                (rgb8(44, 74, 110), rgb8(207, 226, 255)),
            ),
            Self::Adwaita => (
                (rgb8(197, 221, 246), rgb8(20, 84, 140)),
                (rgb8(38, 69, 107), rgb8(214, 230, 250)),
            ),
            Self::Platinum => (
                (rgb8(198, 208, 224), rgb8(0, 0, 128)),
                (rgb8(198, 208, 224), rgb8(0, 0, 128)),
            ),
        };

        Some(match self.effective(appearance) {
            Appearance::Light => light,
            Appearance::Dark => dark,
        })
    }

    /// How a theme's controls are *shaped*, as distinct from coloured.
    ///
    /// This is what makes a Win95 button read as a Win95 button rather than an egui one in grey:
    /// platforms differ far more in padding and control height than in hue. Only the fields a
    /// dialog actually uses are set; the rest keep egui's defaults.
    fn apply_widget_metrics(self, spacing: &mut egui::style::Spacing) {
        // Every value here is a whole number of logical points, deliberately. egui lays each
        // widget out at the running cursor, so a fractional gap puts every *subsequent* widget on
        // a fractional coordinate — which renders blurry, and which egui flags with an orange
        // "Unaligned" marker in debug builds.
        //
        // (button padding x/y, control height, spacing between controls x/y)
        let (padding, height, gap) = match self {
            // Tight and unobtrusive, and distinct from every platform's proportions.
            Self::Plain => ((9.0, 4.0), 24.0, (7.0, 5.0)),
            // Windows 11: generously padded, tall, roomy.
            Self::Fluent => ((11.0, 6.0), 32.0, (8.0, 6.0)),
            // Windows 95: tight and small, with the squat controls of the era.
            Self::Redmond => ((8.0, 3.0), 21.0, (6.0, 4.0)),
            // macOS: wide and short, the classic capsule proportions.
            Self::Aqua => ((13.0, 4.0), 24.0, (10.0, 6.0)),
            // GNOME: the most generous of the lot.
            Self::Adwaita => ((14.0, 7.0), 34.0, (8.0, 6.0)),
            // Mac OS 9: small, tight, square-ish.
            Self::Platinum => ((10.0, 3.0), 20.0, (6.0, 4.0)),
        };

        spacing.button_padding = egui::vec2(padding.0, padding.1);
        spacing.interact_size.y = height;
        spacing.item_spacing = egui::vec2(gap.0, gap.1);
        // A text field should be able to use the dialog's width rather than egui's fixed 280.
        // `INFINITY` is egui's own idiom for "take the available width".
        spacing.text_edit_width = f32::INFINITY;
    }

    /// This theme's own widget styling, before any per-window override.
    ///
    /// `egui::Style` *is* the widget theme system — corner radii, per-state fills and strokes,
    /// selection colour, the text-edit background — so there is no parallel vocabulary here to
    /// map onto it.
    fn widget_style(self, appearance: Appearance) -> egui::Style {
        let appearance = self.effective(appearance);

        // egui's own light/dark visuals are the starting point on each side — the same reason the
        // widget half of a dark variant is nearly free.
        let mut visuals = match appearance {
            Appearance::Light => egui::Visuals::light(),
            Appearance::Dark => egui::Visuals::dark(),
        };

        // Hover and press fills are deliberately louder than the platforms they imitate. Windows 11
        // and Adwaita shift a control by a handful of levels on hover, and a macOS push button has
        // no hover state at all — authentic, and useless here. These popups arrive in numbers and
        // the user's usual goal is to dismiss them, so a control has to visibly answer the pointer.
        // `every_theme_answers_the_pointer_visibly` holds the whole catalogue to a floor.
        if let Some(fill) = self.panel_fill(appearance) {
            visuals.panel_fill = to_color32(fill);
            visuals.window_fill = to_color32(fill);
        }
        if let Some((highlight, text)) = self.selection(appearance) {
            visuals.selection.bg_fill = to_color32(highlight);
            visuals.selection.stroke = egui::Stroke::new(1.0_f32, to_color32(text));
        }

        match self {
            // Square and greyscale, outlined rather than filled: the least assertive control that
            // is still unmistakably a control.
            Self::Plain => {
                round_widgets(&mut visuals, 0);

                let (face, edge, hover, press, field) = match appearance {
                    Appearance::Light => (
                        WHITE,
                        rgb8(77, 77, 77),
                        rgb8(220, 220, 220),
                        rgb8(180, 180, 180),
                        WHITE,
                    ),
                    Appearance::Dark => (
                        rgb8(51, 51, 51),
                        rgb8(128, 128, 128),
                        rgb8(77, 77, 77),
                        rgb8(102, 102, 102),
                        rgb8(13, 13, 13),
                    ),
                };

                for widget in widget_states(&mut visuals) {
                    widget.bg_fill = to_color32(face);
                    widget.weak_bg_fill = to_color32(face);
                    widget.bg_stroke = egui::Stroke::new(1.0_f32, to_color32(edge));
                }
                set_pointer_fills(&mut visuals, hover, press);
                visuals.extreme_bg_color = to_color32(field);
            }
            Self::Fluent => {
                round_widgets(&mut visuals, 4);

                // Windows 11 outlines every control, including at rest — that hairline is a lot of
                // why its buttons look like buttons rather than flat tinted rectangles.
                let (face, edge) = match appearance {
                    Appearance::Light => (rgb8(251, 251, 251), rgb8(209, 209, 209)),
                    Appearance::Dark => (rgb8(45, 45, 45), rgb8(66, 66, 66)),
                };
                for widget in widget_states(&mut visuals) {
                    widget.bg_fill = to_color32(face);
                    widget.weak_bg_fill = to_color32(face);
                    widget.bg_stroke = egui::Stroke::new(1.0_f32, to_color32(edge));
                }
                let (hover, press) = match appearance {
                    Appearance::Light => (rgb8(229, 229, 229), rgb8(213, 213, 213)),
                    Appearance::Dark => (rgb8(68, 68, 68), rgb8(82, 82, 82)),
                };
                set_pointer_fills(&mut visuals, hover, press);
                visuals.extreme_bg_color = to_color32(match appearance {
                    Appearance::Light => WHITE,
                    Appearance::Dark => rgb8(31, 31, 31),
                });
            }
            Self::Redmond => {
                // Square, flat, and a hard edge on every state: Win95 controls have no radius and
                // no hover fill, only a bevel.
                round_widgets(&mut visuals, 0);

                let (face, shadow) = match appearance {
                    Appearance::Light => (REDMOND_FACE, rgb8(128, 128, 128)),
                    Appearance::Dark => (REDMOND_FACE_DARK, rgb8(69, 65, 91)),
                };

                for widget in widget_states(&mut visuals) {
                    widget.bg_fill = to_color32(face);
                    widget.weak_bg_fill = to_color32(face);
                    widget.bg_stroke = egui::Stroke::new(1.0_f32, to_color32(shadow));
                }
                let (hover, press) = match appearance {
                    Appearance::Light => (rgb8(214, 210, 202), rgb8(160, 160, 160)),
                    Appearance::Dark => (rgb8(131, 126, 152), rgb8(85, 80, 106)),
                };
                set_pointer_fills(&mut visuals, hover, press);
                visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, to_color32(BLACK));
                visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, to_color32(BLACK));
                visuals.extreme_bg_color = to_color32(match appearance {
                    Appearance::Light => WHITE,
                    Appearance::Dark => rgb8(38, 34, 48),
                });
            }
            Self::Aqua => {
                // Fully rounded: macOS's push buttons are capsules, not rounded rectangles.
                round_widgets(&mut visuals, 12);

                let face = match appearance {
                    Appearance::Light => WHITE,
                    Appearance::Dark => rgb8(86, 86, 88),
                };
                for widget in widget_states(&mut visuals) {
                    widget.bg_fill = to_color32(face);
                    widget.weak_bg_fill = to_color32(face);
                    widget.bg_stroke = egui::Stroke::NONE;
                }
                let hover = match appearance {
                    Appearance::Light => rgb8(232, 232, 232),
                    Appearance::Dark => rgb8(110, 110, 112),
                };
                // Pressed is the accent blue, as on macOS — the one state it does make loud.
                set_pointer_fills(&mut visuals, hover, rgb8(0, 122, 255));
                // Aqua has no borders to find a control by, so the text field has to be told apart
                // from the dialog by its fill alone — clearly darker, as macOS does in dark mode.
                visuals.extreme_bg_color = to_color32(match appearance {
                    Appearance::Light => WHITE,
                    Appearance::Dark => rgb8(28, 28, 30),
                });
            }
            Self::Adwaita => {
                // Libadwaita rounds everything to the same generous radius.
                round_widgets(&mut visuals, 8);

                // Darker than the headerbar-grey panel behind it. Adwaita's own dialog buttons are
                // roughly 10% black over the window background rather than the near-white they were
                // here, which left them a couple of levels from the surface and invisible.
                let (face, edge) = match appearance {
                    Appearance::Light => (rgb8(225, 222, 219), rgb8(205, 199, 194)),
                    Appearance::Dark => (rgb8(56, 56, 56), rgb8(74, 74, 74)),
                };
                for widget in widget_states(&mut visuals) {
                    widget.bg_fill = to_color32(face);
                    widget.weak_bg_fill = to_color32(face);
                    // A hairline, not the flat look tried first: egui draws text fields with the
                    // same visuals as buttons, and an unbordered white field on a near-white panel
                    // cannot be found at all.
                    widget.bg_stroke = egui::Stroke::new(1.0_f32, to_color32(edge));
                }
                let (hover, press) = match appearance {
                    Appearance::Light => (rgb8(201, 197, 193), rgb8(180, 176, 171)),
                    Appearance::Dark => (rgb8(79, 79, 79), rgb8(96, 96, 96)),
                };
                set_pointer_fills(&mut visuals, hover, press);
                visuals.extreme_bg_color = to_color32(match appearance {
                    Appearance::Light => WHITE,
                    Appearance::Dark => rgb8(30, 30, 30),
                });
            }
            Self::Platinum => {
                // Square, not the 3pt tried first: `bevel::button` paints a bevel on square
                // corners, and a rounded fill under square edges reads as a mistake. Mac OS 9's
                // own controls are near-square anyway.
                round_widgets(&mut visuals, 0);
                for widget in widget_states(&mut visuals) {
                    widget.bg_fill = to_color32(PLATINUM_FACE);
                    widget.weak_bg_fill = to_color32(PLATINUM_FACE);
                    widget.bg_stroke = egui::Stroke::new(1.0_f32, to_color32(rgb8(85, 85, 85)));
                }
                // Darkens on hover rather than lightening, which leaves room for the press state.
                set_pointer_fills(&mut visuals, rgb8(198, 198, 198), rgb8(170, 170, 170));
                visuals.extreme_bg_color = to_color32(WHITE);
            }
        }

        let mut style = egui::Style {
            visuals,
            ..Default::default()
        };
        self.apply_widget_metrics(&mut style.spacing);
        style
    }
}

/// The [`egui::Style`] one window's widgets are drawn with: the theme's own, with this window's
/// `background_color` applied over it.
///
/// The single place either render backend gets its style from. `background_color` keeps the
/// meaning it has always had — it overrides the panel and window fill and nothing else.
pub fn window_style(
    theme: Theme,
    appearance: Appearance,
    background_color: Option<Color>,
) -> egui::Style {
    let mut style = theme.widget_style(appearance);

    apply_widget_font(&mut style, theme.widget_font(), theme.widget_font_size());

    if let Some(color) = background_color {
        let color = to_color32(color);
        style.visuals.window_fill = color;
        style.visuals.panel_fill = color;
    }

    style
}

/// Point a style's proportional text at `font`'s family.
///
/// Loading the font (via [`Theme::widget_font_definitions`]) is not enough on its own: widget
/// text resolves its family through `text_styles`, and `build_font_definitions` registers a
/// *named* family without touching `Proportional`. Sizes are left alone, and `Monospace` entries
/// stay monospace. A no-op for `TextFont::Default`, whose family already *is* `Proportional`.
fn apply_widget_font(style: &mut egui::Style, font: Face, size: f32) {
    let family = text_font::font_family(font);

    for (text_style, font_id) in style.text_styles.iter_mut() {
        if font_id.family != egui::FontFamily::Proportional {
            // `Monospace` stays monospace: a themed proportional face is not a substitute for the
            // family a mode explicitly asked for.
            continue;
        }

        font_id.family = family.clone();

        // Only the sizes a dialog's own widgets use are retuned; `Heading`/`Small` keep egui's
        // proportions relative to the body size.
        let scale = size / 13.0;
        if matches!(text_style, egui::TextStyle::Body | egui::TextStyle::Button) {
            font_id.size = size;
        } else {
            font_id.size *= scale;
        }
    }
}

/// Every per-state widget appearance in a `Visuals`, so a theme can set a property across all of
/// them without naming each state.
fn widget_states(visuals: &mut egui::Visuals) -> [&mut egui::style::WidgetVisuals; 5] {
    let widgets = &mut visuals.widgets;
    [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ]
}

/// Set the fills a control takes under the pointer. Both `bg_fill` and `weak_bg_fill`, since egui
/// uses the latter for buttons and the former for most other controls — setting one and not the
/// other is why a hover can look like it applied everywhere except the buttons.
fn set_pointer_fills(visuals: &mut egui::Visuals, hover: Color, press: Color) {
    visuals.widgets.hovered.bg_fill = to_color32(hover);
    visuals.widgets.hovered.weak_bg_fill = to_color32(hover);
    visuals.widgets.active.bg_fill = to_color32(press);
    visuals.widgets.active.weak_bg_fill = to_color32(press);
}

/// Set one corner radius across every widget state. egui varies it slightly per state by default,
/// which reads as wobble rather than intent once a theme has committed to a radius.
fn round_widgets(visuals: &mut egui::Visuals, radius: u8) {
    for widget in widget_states(visuals) {
        widget.corner_radius = egui::CornerRadius::same(radius);
    }
}

pub fn to_color32(c: Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
        (c.a * 255.0).round() as u8,
    )
}

// ── Chrome ───────────────────────────────────────────────────────────────────────
//
// Several variants below are not constructed yet: they are the vocabulary the catalogue in
// `design/window-themes.md` needs, and `plain` — the only theme so far — is deliberately the
// plainest thing in it. They are exercised by tests here and in `header.rs`, and are what step 4
// fills in: gradients for `aqua`/`fluent`, bevels for `redmond`, circles and left-side clusters
// for `aqua`/`platinum`.

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
#[allow(dead_code)] // see the note above: step 4's vocabulary
#[derive(Debug, Clone, Copy, PartialEq)]
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
#[allow(dead_code)] // see the note above: step 4's vocabulary
#[derive(Debug, Clone, Copy, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// The outline a chrome button is filled within.
#[allow(dead_code)] // see the note above: step 4's vocabulary
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonShape {
    /// A full-height rectangle, as on Windows.
    Rect,
    /// A circle centred in the header, as on macOS.
    Circle,
}

/// The mark drawn on top of a chrome button's fill.
#[allow(dead_code)] // see the note above: step 4's vocabulary
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Cross,
    /// Nothing — an inert button, or one whose glyph only appears on hover.
    None,
}

/// What pressing a chrome button does.
///
/// `Inert` exists only so a platform's silhouette can be completed honestly: macOS keeps its
/// minimise and zoom lights in place, uncoloured, on a window that can do neither. Such a button
/// never reacts to the pointer and never activates — see `design/window-themes.md`, "decorations
/// never lie about function". A *coloured* button that did nothing would be the violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonAction {
    Close,
    Inert,
}

/// One chrome button's paint in one pointer state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonPaint {
    pub fill: Fill,
    pub glyph: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Buttons {
    pub side: Side,
    /// Logical pixels between the outer edge and the first button.
    pub inset: f32,
    /// Logical pixels between adjacent buttons.
    pub gap: f32,
    /// In visual order from `side` inwards.
    pub buttons: &'static [Button],
}

#[allow(dead_code)] // see the note above: step 4's vocabulary
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq)]
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
        },
        hover: ButtonPaint {
            fill: Fill::Solid(PLAIN_CLOSE_HOVER),
            glyph: WHITE,
        },
        active: ButtonPaint {
            fill: Fill::Solid(PLAIN_CLOSE_ACTIVE),
            glyph: WHITE,
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
        size: 14.0,
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
        top_left: rgb8(142, 138, 163),
        bottom_right: BLACK,
    },
    BorderRing::Bevel {
        top_left: rgb8(169, 165, 188),
        bottom_right: rgb8(69, 65, 91),
    },
];
const REDMOND_PRESSED_DARK: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: BLACK,
        bottom_right: rgb8(142, 138, 163),
    },
    BorderRing::Bevel {
        top_left: rgb8(69, 65, 91),
        bottom_right: rgb8(169, 165, 188),
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

/// One of aqua's inert lights: flat grey, no glyph, no reaction to the pointer.
const fn aqua_inert() -> Button {
    Button {
        action: ButtonAction::Inert,
        shape: ButtonShape::Circle,
        glyph: Glyph::None,
        width_ratio: AQUA_LIGHT_RATIO,
        idle: paint(AQUA_DISABLED, TRANSPARENT),
        hover: paint(AQUA_DISABLED, TRANSPARENT),
        active: paint(AQUA_DISABLED, TRANSPARENT),
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
                idle: paint(rgb8(255, 95, 87), TRANSPARENT),
                hover: paint(rgb8(255, 95, 87), rgb8(77, 0, 0)),
                active: paint(rgb8(191, 71, 66), rgb8(77, 0, 0)),
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
        font: Face::Inter,
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
        size: 14.0,
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
const REDMOND_FACE_DARK: Color = rgb8(107, 102, 128);
const REDMOND_TITLE_DARK: Color = rgb8(59, 51, 72);
const REDMOND_GLYPH_DARK: Color = rgb8(240, 238, 245);

const REDMOND_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(REDMOND_TITLE_DARK),
    border: &[
        BorderRing::Bevel {
            top_left: rgb8(142, 138, 163),
            bottom_right: BLACK,
        },
        BorderRing::Bevel {
            top_left: rgb8(169, 165, 188),
            bottom_right: rgb8(69, 65, 91),
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

const fn aqua_inert_dark() -> Button {
    Button {
        action: ButtonAction::Inert,
        shape: ButtonShape::Circle,
        glyph: Glyph::None,
        width_ratio: AQUA_LIGHT_RATIO,
        idle: paint(AQUA_DISABLED_DARK, TRANSPARENT),
        hover: paint(AQUA_DISABLED_DARK, TRANSPARENT),
        active: paint(AQUA_DISABLED_DARK, TRANSPARENT),
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
                idle: paint(rgb8(255, 95, 87), TRANSPARENT),
                hover: paint(rgb8(255, 95, 87), rgb8(77, 0, 0)),
                active: paint(rgb8(191, 71, 66), rgb8(77, 0, 0)),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// The border width and header height in physical pixels at `scale_factor`, as
    /// `(border, header_height)`.
    ///
    /// A non-zero border never rounds down to nothing: at a small enough scale factor it
    /// otherwise would, and the content would then be drawn over the window's own edge.
    pub fn physical(&self, scale_factor: f64) -> (u32, u32) {
        let scale =
            |value: u32| -> u32 { PhysicalUnit::from_logical::<_, u32>(value, scale_factor).0 };

        let border = if self.border_width == 0 {
            0
        } else {
            scale(self.border_width).max(1)
        };

        (border, scale(self.header_height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Modes may do arithmetic against `outer_*`, so these are a promise, not a default.
        assert_eq!(Theme::default(), Theme::Plain);
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

    #[test]
    fn integer_scaling_multiplies_both_metrics() {
        for metrics in every_metric() {
            for scale in [1u32, 2, 3] {
                let (border, header) = metrics.physical(scale as f64);
                assert_eq!(
                    border,
                    metrics.border_width * scale,
                    "{metrics:?} at {scale}x"
                );
                assert_eq!(
                    header,
                    metrics.header_height * scale,
                    "{metrics:?} at {scale}x"
                );
            }
        }
    }

    /// Fractional scale factors are where this is most likely to drift: a border that rounds to
    /// zero would let content be drawn over the window's own edge.
    #[test]
    fn a_non_zero_border_survives_every_scale_factor() {
        for metrics in every_metric().filter(|m| m.border_width > 0) {
            for scale in [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.5] {
                let (border, _) = metrics.physical(scale);
                assert!(border >= 1, "{metrics:?} border vanished at {scale}x");
            }
        }
    }

    /// A theme that asks for no border must not be given one — otherwise a borderless look
    /// would gain a stray pixel that the content-origin maths then has to account for.
    #[test]
    fn a_zero_border_stays_zero() {
        let borderless = Metrics {
            header_height: 32,
            border_width: 0,
        };
        for scale in [1.0, 1.5, 2.0] {
            assert_eq!(borderless.physical(scale).0, 0, "at {scale}x");
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
            ThemeChoice::Platinum,
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

    /// Named choices are pass-throughs, so a mode that names a look gets exactly that look
    /// regardless of platform. Only the two aliases may vary.
    #[test]
    fn naming_a_theme_is_never_platform_dependent() {
        assert_eq!(ThemeChoice::Plain.resolve(), Theme::Plain);
        assert_eq!(ThemeChoice::Fluent.resolve(), Theme::Fluent);
        assert_eq!(ThemeChoice::Redmond.resolve(), Theme::Redmond);
        assert_eq!(ThemeChoice::Aqua.resolve(), Theme::Aqua);
        assert_eq!(ThemeChoice::Adwaita.resolve(), Theme::Adwaita);
        assert_eq!(ThemeChoice::Platinum.resolve(), Theme::Platinum);
    }

    /// The API default is `plain` on every platform — the predictable value, not the
    /// system-matching one. `native` belongs in the bundled modes' options, not here.
    #[test]
    fn the_api_default_is_plain_everywhere() {
        assert_eq!(ThemeChoice::default(), ThemeChoice::Plain);
        assert_eq!(ThemeChoice::default().resolve(), Theme::Plain);
    }

    /// `native` and `native-retro` must not resolve to the same look, or the retro alias is
    /// pointless on that platform. (Linux is the exception: it has no recognisable retro native
    /// look, so both may legitimately differ from each other only by documentation.)
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
        assert_eq!(name(ThemeChoice::Platinum), "\"platinum\"");
        assert_eq!(name(ThemeChoice::Native), "\"native\"");
        assert_eq!(name(ThemeChoice::NativeRetro), "\"native-retro\"");
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
            // Grey, not a colour: identical channels.
            let Fill::Solid(fill) = button.idle.fill else {
                panic!("inert light is not a flat fill");
            };
            assert_eq!(fill.r, fill.g, "inert light is coloured");
            assert_eq!(fill.g, fill.b, "inert light is coloured");
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

    /// Every theme that imitates a platform uses that platform's face, and whenever a theme names
    /// a bundled face the data to load it must come with it — otherwise the style points at a font
    /// family egui was never given, and the text silently falls back.
    #[test]
    fn every_imitating_theme_brings_its_own_face() {
        for &theme in ALL_THEMES {
            let face = theme.widget_font();

            // `plain` imitates nothing and stays on egui's own font; everything else moves off it.
            if theme == Theme::Plain {
                assert_eq!(
                    face,
                    Face::Default,
                    "plain should not adopt a platform face"
                );
            } else {
                assert_ne!(face, Face::Default, "{theme:?} still uses egui's own font");
            }

            assert_eq!(
                theme.widget_font_definitions().is_some(),
                face != Face::Default,
                "{theme:?} names {face:?} without the data to load it"
            );
        }
    }

    /// A theme's title bar and its widgets must be set in the same face. Different faces above and
    /// below the header line is the clearest sign a theme is only skin-deep.
    #[test]
    fn a_themes_title_matches_its_widgets() {
        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                assert_eq!(
                    theme.chrome(appearance).title.font,
                    theme.widget_font(),
                    "{theme:?} {appearance:?}"
                );
            }
        }
    }

    /// Controls differ in *shape* between platforms, not just colour — which is what made every
    /// theme's buttons look like egui's in a different hue.
    #[test]
    fn each_theme_shapes_its_controls_differently() {
        let shape = |theme: Theme| {
            let style = window_style(theme, Appearance::Light, None);
            let spacing = style.spacing;
            (
                format!("{:?}", spacing.button_padding),
                format!("{:?}", spacing.interact_size.y),
                format!("{:?}", style.visuals.widgets.inactive.corner_radius),
            )
        };

        for (index, &a) in ALL_THEMES.iter().enumerate() {
            for &b in &ALL_THEMES[index + 1..] {
                assert_ne!(
                    shape(a),
                    shape(b),
                    "{a:?} and {b:?} shape controls identically"
                );
            }
        }
    }

    /// Each theme's widget styling is its own; two themes sharing a style would mean one is not
    /// actually themed.
    #[test]
    fn each_theme_styles_its_widgets_differently() {
        for (i, &a) in ALL_THEMES.iter().enumerate() {
            for &b in &ALL_THEMES[i + 1..] {
                assert_ne!(
                    styling(&window_style(a, Appearance::Light, None)),
                    styling(&window_style(b, Appearance::Light, None)),
                    "{a:?} and {b:?} style widgets identically"
                );
            }
        }
    }

    /// Perceived luminance, for judging whether two fills are actually distinguishable rather than
    /// merely unequal.
    fn luminance(color: egui::Color32) -> f32 {
        0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32
    }

    /// Every control must visibly answer the pointer, in both palettes.
    ///
    /// Calibrated, not arbitrary: before this floor existed every theme sat between 5 and 17 levels
    /// and *all* of them were reported as hard to distinguish — `fluent` light was 5. Note this is a
    /// deliberate departure from authenticity: real Windows 11 and Adwaita hovers are within a few
    /// levels, and a macOS push button has no hover state at all.
    #[test]
    fn every_theme_answers_the_pointer_visibly() {
        const FLOOR: f32 = 18.0;

        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let widgets = window_style(theme, appearance, None).visuals.widgets;

                // Both fills matter: egui draws buttons with `weak_bg_fill` and most other controls
                // with `bg_fill`, so a theme that moves only one has a hover that applies to only
                // half its widgets.
                for (label, idle, hovered, active) in [
                    (
                        "bg_fill",
                        widgets.inactive.bg_fill,
                        widgets.hovered.bg_fill,
                        widgets.active.bg_fill,
                    ),
                    (
                        "weak_bg_fill",
                        widgets.inactive.weak_bg_fill,
                        widgets.hovered.weak_bg_fill,
                        widgets.active.weak_bg_fill,
                    ),
                ] {
                    let hover_delta = (luminance(hovered) - luminance(idle)).abs();
                    assert!(
                        hover_delta >= FLOOR,
                        "{theme:?} {appearance:?} {label}: hover is only {hover_delta:.1} levels \
                         from idle"
                    );

                    let press_delta = (luminance(active) - luminance(hovered)).abs();
                    assert!(
                        press_delta >= FLOOR / 2.0,
                        "{theme:?} {appearance:?} {label}: pressed is only {press_delta:.1} levels \
                         from hover"
                    );
                }
            }
        }
    }

    /// A control has to be findable on the surface it sits on: either its fill differs from the
    /// dialog's background, or it has a border that does.
    ///
    /// Both halves are needed. `redmond`'s buttons are exactly the same grey as its dialog — as
    /// Win95's were — and are told apart purely by their border; `aqua`'s have no border at all and
    /// rely on the fill. What no theme may do is have neither, which is where `adwaita` was: a
    /// near-white face on a near-white panel with the border deliberately removed.
    #[test]
    fn every_control_is_findable_on_its_panel() {
        const BY_FILL: f32 = 12.0;
        const BY_BORDER: f32 = 15.0;

        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let visuals = window_style(theme, appearance, None).visuals;
                let panel = luminance(visuals.panel_fill);

                for (label, fill) in [
                    ("button", visuals.widgets.inactive.weak_bg_fill),
                    ("control", visuals.widgets.inactive.bg_fill),
                    ("text field", visuals.extreme_bg_color),
                ] {
                    let by_fill = (luminance(fill) - panel).abs();
                    let border = visuals.widgets.inactive.bg_stroke;
                    let by_border = if border.width > 0.0 {
                        (luminance(border.color) - panel).abs()
                    } else {
                        0.0
                    };

                    assert!(
                        by_fill >= BY_FILL || by_border >= BY_BORDER,
                        "{theme:?} {appearance:?}: the {label} is {by_fill:.1} levels from the \
                         dialog behind it, with a border {by_border:.1} levels from it — neither \
                         is enough to find it by"
                    );
                }
            }
        }
    }

    /// Selected text has to be readable against its own highlight.
    ///
    /// egui takes the selected-text colour from `Selection::stroke`, which was never themed — so
    /// every theme painted a saturated brand-colour highlight and then drew egui's default dark
    /// text on top of it. The same field is also a focused field's outline, which is why the
    /// highlights here are tints rather than full accent colours: one colour has to serve both.
    #[test]
    fn selected_text_is_readable_on_its_highlight() {
        const FLOOR: f32 = 60.0;

        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let visuals = window_style(theme, appearance, None).visuals;
                let contrast = (luminance(visuals.selection.bg_fill)
                    - luminance(visuals.selection.stroke.color))
                .abs();

                assert!(
                    contrast >= FLOOR,
                    "{theme:?} {appearance:?}: selected text is only {contrast:.1} levels from \
                     its highlight"
                );

                // And the highlight has to be visible against the field it appears in.
                let field = (luminance(visuals.selection.bg_fill)
                    - luminance(visuals.extreme_bg_color))
                .abs();
                assert!(
                    field >= 15.0,
                    "{theme:?} {appearance:?}: the highlight is only {field:.1} levels from the \
                     text field behind it"
                );
            }
        }
    }

    /// A theme's dialog background should belong to the theme, not to egui — a Win95 dialog is
    /// button-face grey, not near-white, and even `plain` is a designed greyscale rather than
    /// whatever egui happens to default to.
    #[test]
    fn every_theme_sets_its_own_dialog_background() {
        let egui_light = egui::Visuals::light().panel_fill;
        let egui_dark = egui::Visuals::dark().panel_fill;

        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let panel = window_style(theme, appearance, None).visuals.panel_fill;
                let egui_default = match theme.effective(appearance) {
                    Appearance::Light => egui_light,
                    Appearance::Dark => egui_dark,
                };

                assert_ne!(
                    panel, egui_default,
                    "{theme:?} {appearance:?} still uses egui's dialog background"
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
                let expected = matches!(theme, Theme::Redmond | Theme::Platinum);
                assert_eq!(bevelled, expected, "{theme:?} {appearance:?}");
            }
        }
    }

    /// A bevel and a corner radius are different idioms, and `bevel::button` paints square corners
    /// regardless — so a theme asking for one must have set the other to zero.
    #[test]
    fn a_bevelled_theme_has_square_controls() {
        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                if !matches!(theme.widget_edge(appearance), WidgetEdge::Bevel { .. }) {
                    continue;
                }

                let radius = window_style(theme, appearance, None)
                    .visuals
                    .widgets
                    .inactive
                    .corner_radius;
                assert_eq!(
                    radius,
                    egui::CornerRadius::same(0),
                    "{theme:?} {appearance:?} is bevelled but rounded"
                );
            }
        }
    }

    /// `redmond` is square-cornered, which is the most visible single property of a Win95 control.
    #[test]
    fn redmond_widgets_are_square() {
        let style = window_style(Theme::Redmond, Appearance::Light, None);
        for widget in [
            &style.visuals.widgets.inactive,
            &style.visuals.widgets.hovered,
            &style.visuals.widgets.active,
        ] {
            assert_eq!(widget.corner_radius, egui::CornerRadius::same(0));
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

    /// A theme claiming a dark variant must actually have one, in both halves — otherwise the
    /// option is offered and does nothing.
    #[test]
    fn a_theme_supporting_dark_actually_looks_different() {
        for &theme in ALL_THEMES.iter().filter(|t| t.supports_dark()) {
            assert_ne!(
                theme.chrome(Appearance::Light),
                theme.chrome(Appearance::Dark),
                "{theme:?} chrome is identical in both palettes"
            );
            assert_ne!(
                styling(&window_style(theme, Appearance::Light, None)),
                styling(&window_style(theme, Appearance::Dark, None)),
                "{theme:?} widgets are identical in both palettes"
            );
        }
    }

    /// `platinum` has no dark palette, and asking for one must leave *both* halves light — chrome
    /// and widgets disagreeing about the palette would be worse than either choice.
    #[test]
    fn a_theme_without_a_dark_variant_stays_light_in_both_halves() {
        for &theme in ALL_THEMES.iter().filter(|t| !t.supports_dark()) {
            assert_eq!(
                theme.chrome(Appearance::Dark),
                theme.chrome(Appearance::Light),
                "{theme:?} chrome"
            );
            assert_eq!(
                styling(&window_style(theme, Appearance::Dark, None)),
                styling(&window_style(theme, Appearance::Light, None)),
                "{theme:?} widgets"
            );
        }
    }

    #[test]
    fn platinum_is_the_only_theme_without_a_dark_variant() {
        for &theme in ALL_THEMES {
            assert_eq!(theme.supports_dark(), theme != Theme::Platinum, "{theme:?}");
        }
    }

    /// "Dark" has to mean darker. A palette that merely differs would satisfy the test above while
    /// being unreadable next to a dark desktop.
    ///
    /// Measured on the panel fill — the dialog background, and the largest coloured area a user
    /// sees — rather than the title bar. `redmond`'s *light* bar is `#000080` navy, already darker
    /// than most dark-mode chrome, so its Eggplant bar is legitimately lighter than its light one;
    /// the header is simply the wrong thing to measure this on.
    #[test]
    fn a_dark_palette_is_darker_overall() {
        for &theme in ALL_THEMES.iter().filter(|t| t.supports_dark()) {
            let fill = |appearance| {
                let c = window_style(theme, appearance, None).visuals.panel_fill;
                c.r() as u32 + c.g() as u32 + c.b() as u32
            };

            let (light, dark) = (fill(Appearance::Light), fill(Appearance::Dark));
            assert!(
                dark < light,
                "{theme:?}: dark panel {dark} vs light {light}"
            );
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
        const FLOOR: f32 = 90.0;

        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let chrome = theme.chrome(appearance);
                let bar = match chrome.header {
                    Fill::Solid(color) => color,
                    // The lighter end of a gradient is the harder case for dark text.
                    Fill::VerticalGradient { from, to } => {
                        if luminance(to_color32(from)) > luminance(to_color32(to)) {
                            from
                        } else {
                            to
                        }
                    }
                    Fill::Pinstripe { base, .. } => base,
                };

                let contrast =
                    (luminance(to_color32(chrome.title.color)) - luminance(to_color32(bar))).abs();
                assert!(
                    contrast >= FLOOR,
                    "{theme:?} {appearance:?}: the title is only {contrast:.1} levels from its bar"
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
        const FLOOR: f32 = 80.0;

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
                    let contrast =
                        (luminance(to_color32(paint.glyph)) - luminance(to_color32(fill))).abs();
                    assert!(
                        contrast >= FLOOR,
                        "{theme:?} {appearance:?} {state}: the close mark is only {contrast:.1} \
                         levels from its button"
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

    /// The widget half genuinely switches to egui's dark visuals, which is what makes a dialog's
    /// buttons and text fields match the chrome around them.
    #[test]
    fn dark_widgets_use_eguis_dark_visuals() {
        for &theme in ALL_THEMES.iter().filter(|t| t.supports_dark()) {
            let style = window_style(theme, Appearance::Dark, None);
            assert!(style.visuals.dark_mode, "{theme:?}");

            let light = window_style(theme, Appearance::Light, None);
            assert!(!light.visuals.dark_mode, "{theme:?}");
        }
    }

    /// `background_color` still overrides only the two fills, in either palette — a theme must not
    /// widen what that option means just because it changed palette.
    #[test]
    fn background_color_still_overrides_only_the_fills_in_dark() {
        let color = Color {
            r: 0.2,
            g: 0.4,
            b: 0.6,
            a: 1.0,
        };

        for &theme in ALL_THEMES {
            for appearance in BOTH {
                let unstyled = window_style(theme, appearance, None);
                let overridden = window_style(theme, appearance, Some(color));

                let mut expected = unstyled.clone();
                expected.visuals.window_fill = to_color32(color);
                expected.visuals.panel_fill = to_color32(color);

                assert_eq!(
                    styling(&overridden),
                    styling(&expected),
                    "{theme:?} {appearance:?}"
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
            let Fill::Solid(fill) = close.idle.fill else {
                panic!("aqua's close light is not a flat fill");
            };
            // Still red, not greyed: dark mode does not disable it.
            assert!(fill.r > fill.g && fill.r > fill.b, "{appearance:?}");

            for button in buttons.iter().filter(|b| b.action == ButtonAction::Inert) {
                assert_eq!(button.glyph, Glyph::None, "{appearance:?}");
                assert_eq!(button.idle, button.hover, "{appearance:?}");
                assert_eq!(button.idle, button.active, "{appearance:?}");

                let Fill::Solid(fill) = button.idle.fill else {
                    panic!("aqua's inert light is not a flat fill");
                };
                assert_eq!(fill.r, fill.g, "{appearance:?} inert light is coloured");
                assert_eq!(fill.g, fill.b, "{appearance:?} inert light is coloured");
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

    /// `plain`'s own character: greyscale, square, hairline, with a small square close button.
    ///
    /// It replaced the engine's original accidentally-modern-Windows look, which moved to `fluent`.
    /// What makes `plain` worth having is that it imitates nothing — chrome as furniture, for a pack
    /// whose own art should be what the eye goes to.
    #[test]
    fn plain_is_greyscale_square_and_understated() {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let chrome = Theme::Plain.chrome(appearance);

            // A square close button, not the wide slab a Windows caption uses.
            assert_eq!(chrome.buttons.buttons.len(), 1);
            let close = &chrome.buttons.buttons[0];
            assert_eq!(close.width_ratio, 1.0);
            assert_eq!(close.shape, ButtonShape::Rect);

            // Square controls, distinct from every platform theme's rounding.
            let style = window_style(Theme::Plain, appearance, None);
            for widget in [
                &style.visuals.widgets.inactive,
                &style.visuals.widgets.hovered,
                &style.visuals.widgets.active,
            ] {
                assert_eq!(widget.corner_radius, egui::CornerRadius::same(0));
            }

            // And egui's own font: `plain` names no platform's face.
            assert_eq!(Theme::Plain.widget_font(), Face::Default);
            assert_eq!(chrome.title.font, Face::Default);
        }
    }

    /// Everything `plain` draws is a shade of grey, bar the one deliberate exception: the close
    /// button turning red under the pointer. That is what "monochrome" has to mean to be worth
    /// stating — not merely "muted".
    #[test]
    fn plain_is_monochrome_apart_from_its_close_button() {
        let grey = |color: Color| color.r == color.g && color.g == color.b;

        for appearance in [Appearance::Light, Appearance::Dark] {
            let chrome = Theme::Plain.chrome(appearance);

            let Fill::Solid(bar) = chrome.header else {
                panic!("plain's bar is a flat fill");
            };
            assert!(grey(bar), "{appearance:?}: the bar is not grey: {bar:?}");
            assert!(
                grey(chrome.title.color),
                "{appearance:?}: the title is not grey"
            );
            for ring in chrome.border {
                assert!(
                    grey(ring.top_left()),
                    "{appearance:?}: the border is not grey"
                );
            }

            // The close button is grey at rest and red only once the pointer arrives.
            let close = &chrome.buttons.buttons[0];
            let Fill::Solid(idle) = close.idle.fill else {
                panic!("plain's close button is a flat fill");
            };
            assert!(
                grey(idle),
                "{appearance:?}: the idle close button is not grey"
            );

            let Fill::Solid(hover) = close.hover.fill else {
                panic!("plain's close button is a flat fill");
            };
            assert!(
                hover.r > 0.5 && hover.g < 0.2 && hover.b < 0.2,
                "{appearance:?}: the hovered close button should be red, got {hover:?}"
            );

            // Its widgets too, including the selection — no stray accent blue.
            let visuals = window_style(Theme::Plain, appearance, None).visuals;
            for (label, color) in [
                ("panel", visuals.panel_fill),
                ("control", visuals.widgets.inactive.weak_bg_fill),
                ("hover", visuals.widgets.hovered.weak_bg_fill),
                ("text field", visuals.extreme_bg_color),
                ("selection", visuals.selection.bg_fill),
            ] {
                assert!(
                    color.r() == color.g() && color.g() == color.b(),
                    "{appearance:?}: the {label} is not grey: {color:?}"
                );
            }
        }
    }

    // ── Widget styling ───────────────────────────────────────────────────────────

    /// Everything a theme actually sets on a style.
    ///
    /// Whole-`Style` equality is useless across two independently built styles: `Style`'s
    /// `PartialEq` compares `number_formatter` by `Arc::ptr_eq`, and each `Style::default()`
    /// allocates its own. (Comparing a style against a *clone* of itself is fine, since the clone
    /// shares that `Arc` — which is why the font tests below can use `==` directly.)
    fn styling(
        style: &egui::Style,
    ) -> (
        egui::Visuals,
        std::collections::BTreeMap<egui::TextStyle, egui::FontId>,
        egui::style::Spacing,
    ) {
        (
            style.visuals.clone(),
            style.text_styles.clone(),
            style.spacing.clone(),
        )
    }

    /// `background_color` has always meant the panel and window fill, and nothing else. A theme
    /// must not widen that: everything else in the style stays the theme's.
    #[test]
    fn background_color_overrides_only_the_two_fills() {
        let color = Color {
            r: 0.2,
            g: 0.4,
            b: 0.6,
            a: 1.0,
        };

        for &theme in ALL_THEMES {
            let unstyled = window_style(theme, Appearance::Light, None);
            let overridden = window_style(theme, Appearance::Light, Some(color));

            let mut expected = unstyled.clone();
            expected.visuals.window_fill = to_color32(color);
            expected.visuals.panel_fill = to_color32(color);

            assert_eq!(styling(&overridden), styling(&expected), "{theme:?}");
            // And the override actually did something, so the assert above isn't vacuous.
            assert_ne!(styling(&overridden), styling(&unstyled), "{theme:?}");
        }
    }

    #[test]
    fn an_alpha_channel_survives_into_the_fill() {
        let translucent = Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.5,
        };
        let style = window_style(Theme::Plain, Appearance::Light, Some(translucent));
        assert_eq!(style.visuals.panel_fill.a(), 128);
    }

    /// The seam that lets a theme choose its widget font. No theme picks a non-default font yet
    /// — `redmond` will want W95FA — so this exercises the mechanism directly: loading a font is
    /// not enough, the style's proportional entries have to be repointed at its family.
    #[test]
    fn a_non_default_font_is_remapped_onto_proportional_text_only() {
        for font in [Face::Pixel, Face::Display] {
            let mut style = egui::Style::default();
            let before = style.clone();
            apply_widget_font(&mut style, font, 13.0);

            let family = text_font::font_family(font);
            assert_ne!(style, before, "{font:?} changed nothing");

            for (text_style, font_id) in &style.text_styles {
                let original = &before.text_styles[text_style];
                // Sizes are never touched, whichever family an entry ends up with.
                assert_eq!(
                    font_id.size, original.size,
                    "{font:?} resized {text_style:?}"
                );

                match original.family {
                    egui::FontFamily::Proportional => {
                        assert_eq!(font_id.family, family, "{font:?} missed {text_style:?}");
                    }
                    // Monospace must stay monospace — a themed proportional font is not a
                    // substitute for the family a mode explicitly asked for.
                    ref other => assert_eq!(&font_id.family, other, "{font:?} took {text_style:?}"),
                }
            }
        }
    }

    #[test]
    fn the_default_font_leaves_the_style_untouched() {
        let mut style = egui::Style::default();
        let before = style.clone();
        apply_widget_font(&mut style, Face::Default, 13.0);
        assert_eq!(style, before);
    }
}
