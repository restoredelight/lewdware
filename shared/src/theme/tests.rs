use super::chrome::*;
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
    assert_eq!(Theme::Adwaita.widgets(Appearance::Dark).text, WHITE);
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

#[test]
fn platinum_uses_the_classic_square_close_box() {
    let close = Theme::Platinum.chrome(Appearance::Light).buttons.buttons[0];
    assert_eq!(close.shape, ButtonShape::Square);
    assert_eq!(close.glyph, Glyph::Square);
}

#[test]
fn platform_header_measurements_stay_pinned() {
    assert_eq!(Theme::Fluent.metrics().header_height, 32);
    assert_eq!(Theme::Adwaita.metrics().header_height, 47);

    let windows = Theme::Fluent.chrome(Appearance::Light).buttons.buttons[0];
    assert!((windows.width_ratio * 32.0 - 46.0).abs() < f32::EPSILON);

    let gnome = Theme::Adwaita.chrome(Appearance::Light).buttons.buttons[0];
    assert!((gnome.width_ratio * 47.0 - 34.0).abs() < f32::EPSILON);
    assert!((gnome.diameter_ratio * 47.0 - 24.0).abs() < f32::EPSILON);
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
    assert!(
        theme(config.theme.name()).is_some(),
        "{}",
        config.theme.name()
    );
    assert!(
        appearance(config.appearance.name()).is_some(),
        "{}",
        config.appearance.name()
    );
}

#[test]
fn an_unknown_name_is_none_rather_than_a_panic() {
    assert!(theme("some-future-theme").is_none());
    assert!(appearance("sepia").is_none());
}
