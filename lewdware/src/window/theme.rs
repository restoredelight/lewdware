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
mod tests;

