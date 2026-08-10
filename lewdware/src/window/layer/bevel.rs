//! Buttons painted by hand, for the themes whose controls are three-dimensional.
//!
//! `egui::Style` is the widget theme system and everything else goes through it — see
//! `design/window-themes.md`. The one thing it cannot express is a two-tone edge:
//! `WidgetVisuals::bg_stroke` is a single `Stroke`, uniform on all four sides, so a raised Win95
//! button — light on the top and left, dark on the bottom and right, inverted when held — is
//! unreachable however the style is tuned. And that edge *is* the theme: a flat grey rectangle with
//! one dark outline reads as an egui button in Win95 colours rather than as a Win95 button.
//!
//! So a theme asking for [`WidgetEdge::Bevel`] gets its buttons drawn here instead. Deliberately
//! **buttons only**: egui's `TextEdit` owns cursor, selection, IME and clipboard behaviour, none of
//! which is worth reimplementing for an edge, so text fields stay on egui's own painting even in
//! these themes.

use egui::{Response, Sense, Ui, Vec2};

use crate::window::theme::{
    BorderRing, DefaultButtonStyle, WidgetEdge, to_color32, to_egui_stroke,
};

/// Overlay a recessed themed edge on egui's real text edit. The widget keeps ownership of text,
/// cursor, selection, IME and clipboard behavior; only its otherwise-flat perimeter is replaced.
pub fn input_edge(ui: &Ui, response: &Response, edge: &WidgetEdge) {
    let WidgetEdge::Bevel { pressed, .. } = edge else {
        return;
    };

    // A field is permanently recessed, so it uses the same inverted rings as a held button.
    paint_rings(ui.painter(), response.rect, pressed);
}

/// Draw a button with a themed edge and return its response, mirroring `ui.button(label)`.
///
/// Sized exactly as egui sizes its own buttons — galley plus `button_padding`, at least
/// `interact_size` — so a bevelled row and a flat one lay out identically and the row-width
/// measurement in `paint_dialog` stays correct for both.
pub fn button(
    ui: &mut Ui,
    label: &str,
    edge: &WidgetEdge,
    default_style: Option<DefaultButtonStyle>,
) -> Response {
    let WidgetEdge::Bevel { raised, pressed } = edge else {
        return match default_style {
            Some(DefaultButtonStyle::Filled {
                idle,
                hover,
                active,
                text,
                border,
            }) => flat_default_button(
                ui,
                label,
                to_color32(idle),
                to_color32(hover),
                to_color32(active),
                to_color32(text),
                to_egui_stroke(border),
            ),
            Some(DefaultButtonStyle::Outline(stroke)) => {
                let response = ui.button(label);
                paint_outline(ui.painter(), response.rect, to_egui_stroke(stroke));
                response
            }
            None => ui.button(label),
        };
    };

    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.ctx().fonts_mut(|fonts| {
        fonts.layout_no_wrap(label.to_owned(), font_id, egui::Color32::PLACEHOLDER)
    });

    let padding = ui.spacing().button_padding;
    let desired = Vec2::new(
        galley.size().x + padding.x * 2.0,
        (galley.size().y + padding.y * 2.0).max(ui.spacing().interact_size.y),
    );

    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let held = response.is_pointer_button_down_on();
    let visuals = ui.style().interact(&response);
    let painter = ui.painter();

    // Square: a bevel and a corner radius are different idioms, and every theme that asks for one
    // of these has already set the other to zero.
    painter.rect_filled(rect, 0.0, visuals.weak_bg_fill);
    paint_rings(painter, rect, if held { pressed } else { raised });

    // A held Win95 button shifts its label down and right, as though the face itself had moved.
    let offset = if held { Vec2::splat(1.0) } else { Vec2::ZERO };
    let text_pos = rect.center() - galley.size() / 2.0 + offset;
    painter.galley(text_pos, galley, visuals.text_color());

    if let Some(DefaultButtonStyle::Outline(stroke)) = default_style {
        // Reinforce the outside edge. An inset outline boxes the label and obscures the face;
        // this leaves both bevel rings visible immediately inside it.
        paint_outline(painter, rect, to_egui_stroke(stroke));
    }

    // Keyboard focus, which egui's own button would otherwise have drawn.
    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(3.0),
            0.0,
            egui::Stroke::new(1.0_f32, visuals.text_color()),
            egui::StrokeKind::Inside,
        );
    }

    response
}

fn paint_outline(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    // All outline-only themes have square controls. Line segments avoid introducing a second
    // filled rectangle into egui's shape list and keep the mark on the actual outer edge.
    let rect = rect.shrink(stroke.width / 2.0);
    painter.line_segment([rect.left_top(), rect.right_top()], stroke);
    painter.line_segment([rect.right_top(), rect.right_bottom()], stroke);
    painter.line_segment([rect.right_bottom(), rect.left_bottom()], stroke);
    painter.line_segment([rect.left_bottom(), rect.left_top()], stroke);
}

/// A modern theme's filled primary action. Painted here instead of using `Button::fill`, whose
/// fixed override suppresses the theme-specific hover and pressed colours.
fn flat_default_button(
    ui: &mut Ui,
    label: &str,
    idle: egui::Color32,
    hover: egui::Color32,
    active: egui::Color32,
    text: egui::Color32,
    border: egui::Stroke,
) -> Response {
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .ctx()
        .fonts_mut(|fonts| fonts.layout_no_wrap(label.to_owned(), font_id, text));
    let padding = ui.spacing().button_padding;
    let desired = Vec2::new(
        galley.size().x + padding.x * 2.0,
        (galley.size().y + padding.y * 2.0).max(ui.spacing().interact_size.y),
    );
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let fill = if response.is_pointer_button_down_on() {
        active
    } else if response.hovered() {
        hover
    } else {
        idle
    };
    let radius = ui.style().visuals.widgets.inactive.corner_radius;
    ui.painter()
        .rect(rect, radius, fill, border, egui::StrokeKind::Inside);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, text);

    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(3.0),
            radius,
            egui::Stroke::new(1.0_f32, text),
            egui::StrokeKind::Inside,
        );
    }

    response
}

/// Paint `rings` inside `rect`, outermost first, one point per ring.
///
/// Filled strips rather than strokes: a stroke straddles the path it follows, so a one-point ring
/// drawn as a stroke lands half in and half out of the rect and blurs at any fractional scale
/// factor. This is the same reason `platinum`'s pinstripes are drawn unantialiased.
fn paint_rings(painter: &egui::Painter, rect: egui::Rect, rings: &[BorderRing]) {
    for (index, ring) in rings.iter().enumerate() {
        let inset = index as f32;
        let ring_rect = rect.shrink(inset);
        if ring_rect.width() <= 2.0 || ring_rect.height() <= 2.0 {
            break;
        }

        let top_left = crate::window::theme::to_color32(ring.top_left());
        let bottom_right = crate::window::theme::to_color32(ring.bottom_right());

        // Top and left in one tone; bottom and right in the other. The corners belong to whichever
        // is drawn second, which is what gives a bevel its mitred look.
        painter.rect_filled(
            egui::Rect::from_min_size(ring_rect.min, Vec2::new(ring_rect.width(), 1.0)),
            0.0,
            top_left,
        );
        painter.rect_filled(
            egui::Rect::from_min_size(ring_rect.min, Vec2::new(1.0, ring_rect.height())),
            0.0,
            top_left,
        );
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(ring_rect.min.x, ring_rect.max.y - 1.0),
                Vec2::new(ring_rect.width(), 1.0),
            ),
            0.0,
            bottom_right,
        );
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(ring_rect.max.x - 1.0, ring_rect.min.y),
                Vec2::new(1.0, ring_rect.height()),
            ),
            0.0,
            bottom_right,
        );
    }
}
