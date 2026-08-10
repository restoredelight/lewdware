//! The engine's view of a [theme](shared::theme): how one reaches the two things that draw it.
//!
//! The themes themselves live in `shared::theme`, because `config/` reads them too — it renders a
//! preview of each look in its picker and cannot link this crate. What is left here is the
//! projection onto the renderers: [`to_egui_style`] for a dialog's widgets, [`to_color32`] and
//! [`to_egui_stroke`] for the chrome painter in `header.rs`, and [`physical_metrics`] for the
//! pixel geometry a window's decorations are laid out at.

use winit::dpi::PhysicalUnit;

pub use shared::theme::*;

use crate::text_font;

/// Extra font data egui needs loaded before a theme's widget font can be used, or `None` when the
/// built-in families are enough.
pub fn widget_font_definitions(theme: Theme) -> Option<egui::FontDefinitions> {
    text_font::build_font_definitions(theme.widget_font())
}

/// A theme's border width and header height in physical pixels at `scale_factor`, as
/// `(border, header_height)`.
///
/// A non-zero border never rounds down to nothing: at a small enough scale factor it otherwise
/// would, and the content would then be drawn over the window's own edge.
pub fn physical_metrics(metrics: Metrics, scale_factor: f64) -> (u32, u32) {
    let scale = |value: u32| -> u32 { PhysicalUnit::from_logical::<_, u32>(value, scale_factor).0 };

    let border = if metrics.border_width == 0 {
        0
    } else {
        scale(metrics.border_width).max(1)
    };

    (border, scale(metrics.header_height))
}

/// Project a theme's widget half onto an [`egui::Style`], which is what actually draws a dialog.
///
/// The substrate is egui's own light or dark visuals, but only to fill in the parts of egui we
/// never put on screen — window shadows, resize corners, hyperlinks. Everything a dialog shows is
/// set from [`Widgets`], including the two values that used to be inherited from egui: the text
/// colour and the caret.
pub fn to_egui_style(widgets: Widgets) -> egui::Style {
    let mut visuals = match widgets.base {
        Appearance::Light => egui::Visuals::light(),
        Appearance::Dark => egui::Visuals::dark(),
    };

    visuals.panel_fill = to_color32(widgets.panel);
    visuals.window_fill = to_color32(widgets.panel);
    visuals.extreme_bg_color = to_color32(widgets.field);
    visuals.override_text_color = Some(to_color32(widgets.text));
    visuals.text_cursor.stroke = to_egui_stroke(widgets.caret);
    visuals.selection.bg_fill = to_color32(widgets.selection);
    visuals.selection.stroke = to_egui_stroke(Stroke::hairline(widgets.selection_text));

    // `open` and `noninteractive` take the resting paint: a dialog has no control that could show
    // either state in a way a user could tell apart from a button at rest.
    let egui_widgets = &mut visuals.widgets;
    for (widget, paint) in [
        (&mut egui_widgets.noninteractive, widgets.idle),
        (&mut egui_widgets.inactive, widgets.idle),
        (&mut egui_widgets.open, widgets.idle),
        (&mut egui_widgets.hovered, widgets.hover),
        (&mut egui_widgets.active, widgets.pressed),
    ] {
        // Both fills: egui uses `weak_bg_fill` for buttons and `bg_fill` for most other controls,
        // so setting one and not the other themes half a dialog.
        widget.bg_fill = to_color32(paint.fill);
        widget.weak_bg_fill = to_color32(paint.fill);
        widget.bg_stroke = to_egui_stroke(paint.border);
        widget.fg_stroke = to_egui_stroke(Stroke::hairline(widgets.text));
        widget.corner_radius = egui::CornerRadius::same(widgets.metrics.corner_radius);
    }

    let mut style = egui::Style {
        visuals,
        ..Default::default()
    };

    let spacing = &mut style.spacing;
    spacing.button_padding = egui::vec2(
        widgets.metrics.button_padding.0,
        widgets.metrics.button_padding.1,
    );
    spacing.interact_size.y = widgets.metrics.control_height;
    spacing.item_spacing = egui::vec2(
        widgets.metrics.item_spacing.0,
        widgets.metrics.item_spacing.1,
    );
    // A text field should be able to use the dialog's width rather than egui's fixed 280.
    // `INFINITY` is egui's own idiom for "take the available width".
    spacing.text_edit_width = f32::INFINITY;

    style
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
    let widgets = theme.widgets(appearance);
    let mut style = to_egui_style(*widgets);

    apply_widget_font(&mut style, widgets.font, widgets.font_size);

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

pub fn to_egui_stroke(s: Stroke) -> egui::Stroke {
    egui::Stroke::new(s.width, to_color32(s.color))
}

pub fn to_color32(c: Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
        (c.a * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: [Appearance; 2] = [Appearance::Light, Appearance::Dark];

    /// Every real theme's metrics, plus two stand-ins for shapes no theme has yet, so the
    /// scaling below is exercised beyond the sizes that happen to ship.
    fn every_metric() -> impl Iterator<Item = Metrics> {
        const FUTURE: &[Metrics] = &[
            Metrics {
                header_height: 44,
                border_width: 3,
            },
            Metrics {
                header_height: 16,
                border_width: 0,
            },
        ];

        ALL_THEMES
            .iter()
            .map(|theme| theme.metrics())
            .chain(FUTURE.iter().copied())
    }

    #[test]
    fn integer_scaling_multiplies_both_metrics() {
        for metrics in every_metric() {
            for scale in [1u32, 2, 3] {
                let (border, header) = physical_metrics(metrics, scale as f64);
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
                let (border, _) = physical_metrics(metrics, scale);
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
            assert_eq!(physical_metrics(borderless, scale).0, 0, "at {scale}x");
        }
    }

    /// Every theme that imitates a platform uses that platform's face, and whenever a theme names
    /// a bundled face the data to load it must come with it — otherwise the style points at a font
    /// family egui was never given, and the text silently falls back.
    #[test]
    fn every_imitating_theme_brings_its_own_face() {
        for &theme in ALL_THEMES {
            let face = theme.widget_font();

            // `plain` imitates nothing, so it keeps the neutral face; everything else moves off it.
            if theme == Theme::Plain {
                assert_eq!(
                    face,
                    Face::Default,
                    "plain should not adopt a platform face"
                );
            } else {
                assert_ne!(face, Face::Default, "{theme:?} still uses the neutral face");
            }

            // Every face is bundled, `Default` included: a theme is never left to whatever font
            // egui would otherwise have reached for, which is what let `config/` draw the same
            // text in the same typeface.
            assert!(
                widget_font_definitions(theme).is_some(),
                "{theme:?} names {face:?} without the data to load it"
            );
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

    /// A dark palette declares itself as one, and the substrate `to_egui_style` builds on follows
    /// — which is what keeps the parts of egui a theme does not model (shadows, scrollbars) from
    /// being light-mode leftovers inside a dark dialog.
    #[test]
    fn dark_widgets_use_eguis_dark_visuals() {
        for &theme in ALL_THEMES.iter().filter(|t| t.supports_dark()) {
            let style = window_style(theme, Appearance::Dark, None);
            assert!(style.visuals.dark_mode, "{theme:?}");

            let light = window_style(theme, Appearance::Light, None);
            assert!(!light.visuals.dark_mode, "{theme:?}");
        }
    }

    /// The projection has one job. Checking a few representative fields is enough to catch a
    /// wiring mistake -- a value written to the wrong egui field, or dropped entirely -- without
    /// restating the whole conversion.
    #[test]
    fn the_egui_projection_carries_the_data_across() {
        for &theme in ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let w = theme.widgets(appearance);
                let style = to_egui_style(*w);
                let visuals = &style.visuals;

                assert_eq!(visuals.panel_fill, to_color32(w.panel), "{theme:?}");
                assert_eq!(visuals.extreme_bg_color, to_color32(w.field), "{theme:?}");
                assert_eq!(
                    visuals.override_text_color,
                    Some(to_color32(w.text)),
                    "{theme:?}"
                );
                assert_eq!(
                    visuals.text_cursor.stroke.color,
                    to_color32(w.caret.color),
                    "{theme:?}"
                );
                assert_eq!(
                    visuals.widgets.inactive.weak_bg_fill,
                    to_color32(w.idle.fill),
                    "{theme:?}"
                );
                assert_eq!(
                    visuals.widgets.hovered.weak_bg_fill,
                    to_color32(w.hover.fill),
                    "{theme:?}"
                );
                assert_eq!(
                    visuals.widgets.active.weak_bg_fill,
                    to_color32(w.pressed.fill),
                    "{theme:?}"
                );
                assert_eq!(
                    style.spacing.interact_size.y, w.metrics.control_height,
                    "{theme:?}"
                );
            }
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

    /// `Default` is a bundled face like any other now, not "leave it to egui". It still has to be
    /// *applied*, or `plain` would draw in whichever font egui happened to pick — which is exactly
    /// what stopped `config/` reproducing it.
    #[test]
    fn the_neutral_font_is_applied_like_any_other() {
        let mut style = egui::Style::default();
        let before = style.clone();
        apply_widget_font(&mut style, Face::Default, 13.0);

        assert_ne!(style, before, "the neutral face changed nothing");

        let family = text_font::font_family(Face::Default);
        assert_eq!(style.text_styles[&egui::TextStyle::Body].family, family);
        assert_eq!(style.text_styles[&egui::TextStyle::Button].family, family);
    }
}
