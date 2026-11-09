//! A live miniature of one window theme: border, title bar and close button, and a dialog with a
//! text field and buttons. Ported from `config/src/lib/ThemePreview.svelte`.
//!
//! Drawn from the same `shared::theme` values the engine paints with, so this is a second renderer
//! of one description, not a second description. Geometry follows the engine's:
//!
//! - A title-bar button is the full height of the bar and `width_ratio` x that wide. `inset` is
//!   one horizontal offset from the bar's end (`Header::button_span` in
//!   `lewdware/src/window/header.rs`).
//! - A glyph reaches `extent x glyph_ratio` from its button's centre (`Header::glyph_reach`).
//! - The dialog is a 10pt margin, centred content and a centred button row (`paint_dialog`).
//!
//! Corners stay square, as the engine's CPU path cannot draw transparent corner pixels, and inert
//! buttons (Aqua's traffic lights) never respond to the pointer.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    canvas, div, linear_color_stop, linear_gradient, point, px, AnyElement, App, Background,
    Bounds, ElementId, Hsla, PathBuilder, Pixels, Rgba, SharedString, Window,
};
use shared::theme::{
    BorderRing, Button, ButtonAction, ButtonPaint, ButtonShape, Color, DefaultButtonStyle, Fill,
    Glyph, Side, Stroke, TextAlign, WidgetEdge,
};

use crate::backend::themes::Look;
use crate::theme as ui_theme;

/// The dialog's own margin, and the space `paint_dialog` puts after every element.
const DIALOG_MARGIN: f32 = 10.;

fn color(c: Color) -> Hsla {
    Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
    .into()
}

fn background(fill: &Fill) -> Background {
    match *fill {
        Fill::Solid(c) => color(c).into(),
        // 180deg: top to bottom.
        Fill::VerticalGradient { from, to } => linear_gradient(
            180.,
            linear_color_stop(color(from), 0.),
            linear_color_stop(color(to), 1.),
        ),
        Fill::Pinstripe { base, .. } => color(base).into(),
    }
}

/// Mac OS 9's stripes, drawn over the base fill: one line every `period` pixels.
fn pinstripes(fill: &Fill, height: f32) -> Option<AnyElement> {
    let Fill::Pinstripe { stripe, period, .. } = *fill else {
        return None;
    };
    let period = period.max(1) as f32;
    let count = (height / period).ceil() as usize;
    Some(
        div()
            .absolute()
            .inset_0()
            .children((0..count).map(move |i| {
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(i as f32 * period + period - 1.))
                    .h(px(1.))
                    .bg(color(stripe))
            }))
            .into_any_element(),
    )
}

/// The engine interrupts Platinum's stripes with the midpoint of the stripe pair behind the title.
fn title_plaque(fill: &Fill) -> Option<Hsla> {
    let Fill::Pinstripe { base, stripe, .. } = *fill else {
        return None;
    };
    Some(color(Color {
        r: (base.r + stripe.r) / 2.,
        g: (base.g + stripe.g) / 2.,
        b: (base.b + stripe.b) / 2.,
        a: 1.,
    }))
}

/// One 1px ring around `inner`. A bevel needs a different colour per side, which a single gpui
/// border colour can't express, so its edges are drawn as four strips.
fn ring(ring: &BorderRing, inner: AnyElement) -> gpui_kit::Div {
    match *ring {
        BorderRing::Uniform(c) => div().border_1().border_color(color(c)).child(inner),
        BorderRing::Bevel {
            top_left,
            bottom_right,
        } => div()
            .relative()
            .p(px(1.))
            .child(inner)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(1.))
                    .bg(color(top_left)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .bottom_0()
                    .w(px(1.))
                    .bg(color(top_left)),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left(px(1.))
                    .right_0()
                    .h(px(1.))
                    .bg(color(bottom_right)),
            )
            .child(
                div()
                    .absolute()
                    .top(px(1.))
                    .right_0()
                    .bottom_0()
                    .w(px(1.))
                    .bg(color(bottom_right)),
            ),
    }
}

fn rings(rings: &[BorderRing], inner: AnyElement) -> AnyElement {
    rings
        .iter()
        .rev()
        .fold(inner, |inner, r| ring(r, inner).into_any_element())
}

fn stroke_border<E: Styled>(element: E, stroke: Stroke) -> E {
    if stroke.width <= 0. {
        element
    } else {
        element
            .border(px(stroke.width))
            .border_color(color(stroke.color))
    }
}

/// Hover and press on the title-bar buttons. Kept per card so the glyph, which is painted rather
/// than styled, can follow it too.
#[derive(Default, Clone, Copy)]
struct ChromeState {
    hovered: Option<usize>,
    pressed: Option<usize>,
}

#[derive(IntoElement)]
pub struct ThemePreview {
    id: ElementId,
    look: Look,
    title: SharedString,
    /// Content width in logical pixels, before the border is added around it.
    width: f32,
}

impl ThemePreview {
    pub fn new(id: impl Into<ElementId>, look: Look, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            look,
            title: title.into(),
            width: 236.,
        }
    }

    fn chrome_button(
        &self,
        index: usize,
        button: &Button,
        state: ChromeState,
        state_entity: &gpui_kit::Entity<ChromeState>,
    ) -> AnyElement {
        let header = self.look.metrics.header_height as f32;
        let width = button.width_ratio * header;
        let interactive = button.action != ButtonAction::Inert;
        let paint: ButtonPaint = match (interactive, state.pressed, state.hovered) {
            (true, Some(i), _) if i == index => button.active,
            (true, _, Some(i)) if i == index => button.hover,
            _ => button.idle,
        };
        let diameter = width.min(header * button.diameter_ratio);
        let extent = match button.shape {
            ButtonShape::Rect => width.min(header),
            _ => diameter,
        };
        let glyph_size = 2. * extent * button.glyph_ratio;
        let glyph = button.glyph;
        let glyph_color = color(paint.glyph);

        let face = div()
            .flex()
            .items_center()
            .justify_center()
            .bg(background(&paint.fill))
            .when_some(paint.rim, |this, rim| {
                this.border_1().border_color(color(rim))
            })
            .map(|this| match button.shape {
                ButtonShape::Rect => this.size_full(),
                ButtonShape::Square => this.size(px(diameter)),
                ButtonShape::Circle => this.size(px(diameter)).rounded_full(),
            })
            .when(glyph != Glyph::None, |this| {
                this.child(
                    canvas(
                        |_, _, _| {},
                        move |bounds: Bounds<Pixels>, _, window, _| {
                            let mut path = PathBuilder::stroke(px(1.));
                            let o = bounds.origin;
                            let s = bounds.size.width;
                            match glyph {
                                Glyph::Square => {
                                    path.move_to(o);
                                    path.line_to(point(o.x + s, o.y));
                                    path.line_to(point(o.x + s, o.y + s));
                                    path.line_to(point(o.x, o.y + s));
                                    path.close();
                                }
                                _ => {
                                    path.move_to(o);
                                    path.line_to(point(o.x + s, o.y + s));
                                    path.move_to(point(o.x + s, o.y));
                                    path.line_to(point(o.x, o.y + s));
                                }
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, glyph_color);
                            }
                        },
                    )
                    .size(px(glyph_size)),
                )
            });

        let hover_state = state_entity.clone();
        let press_state = state_entity.clone();
        let release_state = state_entity.clone();
        div()
            .id(("chrome-button", index))
            .flex_none()
            .w(px(width))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .child(face)
            .when(interactive, |this| {
                this.on_hover(move |hovered, _, cx| {
                    hover_state.update(cx, |state, cx| {
                        state.hovered = hovered.then_some(index);
                        if !hovered {
                            state.pressed = None;
                        }
                        cx.notify();
                    })
                })
                .on_mouse_down(gpui_kit::MouseButton::Left, move |_, _, cx| {
                    press_state.update(cx, |state, cx| {
                        state.pressed = Some(index);
                        cx.notify();
                    })
                })
                .on_mouse_up(gpui_kit::MouseButton::Left, move |_, _, cx| {
                    release_state.update(cx, |state, cx| {
                        state.pressed = None;
                        cx.notify();
                    })
                })
            })
            .into_any_element()
    }

    fn header(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let chrome = &self.look.chrome;
        let header = self.look.metrics.header_height as f32;
        let buttons = &chrome.buttons;
        let state_entity = window.use_keyed_state((self.id.clone(), "chrome"), cx, |_, _| {
            ChromeState::default()
        });
        let state = *state_entity.read(cx);

        // `Header::buttons_extent`: the whole cluster, including its single inset and any gaps.
        let cluster = if buttons.buttons.is_empty() {
            0.
        } else {
            buttons.inset
                + buttons
                    .buttons
                    .iter()
                    .map(|b| b.width_ratio * header)
                    .sum::<f32>()
                + buttons.gap * (buttons.buttons.len() - 1) as f32
        };
        let left = buttons.side == Side::Left;
        let (family, weight) = ui_theme::face(chrome.title.font);
        let plaque = title_plaque(&chrome.header);

        let title = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .px(px(chrome.title.padding))
            .flex()
            .map(|this| match chrome.title.align {
                TextAlign::Left => this.justify_start(),
                TextAlign::Center => this.justify_center(),
                TextAlign::Right => this.justify_end(),
            })
            // The engine centres a title across the whole bar when it fits, not in what the
            // buttons leave over, so the cluster is balanced on the far side.
            .when(chrome.title.align == TextAlign::Center, |this| {
                if left {
                    this.mr(px(cluster))
                } else {
                    this.ml(px(cluster))
                }
            })
            .child(
                div()
                    .truncate()
                    .font_family(family)
                    .font_weight(weight)
                    .text_size(px(chrome.title.size))
                    .line_height(px(chrome.title.size))
                    .text_color(color(chrome.title.color))
                    .when_some(plaque, |this, plaque| this.bg(plaque).px(px(4.)))
                    .child(self.title.clone()),
            );

        div()
            .relative()
            .h(px(header))
            .flex()
            .items_center()
            .overflow_hidden()
            .bg(background(&chrome.header))
            .children(pinstripes(&chrome.header, header))
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .items_center()
                    .gap(px(buttons.gap))
                    .map(|this| {
                        if left {
                            this.flex_row().pl(px(buttons.inset))
                        } else {
                            this.flex_row_reverse().pr(px(buttons.inset))
                        }
                    })
                    .children(
                        buttons
                            .buttons
                            .iter()
                            .enumerate()
                            .map(|(i, b)| self.chrome_button(i, b, state, &state_entity)),
                    )
                    .child(title),
            )
            // Over the buttons as well as the bar, as `Header::draw_separator` paints it.
            .when_some(chrome.separator, |this, separator| {
                this.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(1.))
                        .bg(color(separator)),
                )
            })
            .into_any_element()
    }

    fn dialog_button(&self, id: &'static str, label: &'static str, is_default: bool) -> AnyElement {
        let widgets = &self.look.widgets;
        let metrics = &widgets.metrics;
        let filled = match widgets.default_button {
            DefaultButtonStyle::Filled {
                idle,
                hover,
                active,
                text,
                border,
            } if is_default => Some((idle, hover, active, text, border)),
            _ => None,
        };
        let outline = match widgets.default_button {
            DefaultButtonStyle::Outline(stroke) if is_default => Some(stroke),
            _ => None,
        };
        let (idle, hover, active, text) = filled.map_or(
            (
                widgets.idle.fill,
                widgets.hover.fill,
                widgets.pressed.fill,
                widgets.text,
            ),
            |(idle, hover, active, text, _)| (idle, hover, active, text),
        );
        let bevel = match widgets.edge {
            WidgetEdge::Bevel { raised, .. } => raised.first().copied(),
            WidgetEdge::Flat => None,
        };

        let face = div()
            .id(id)
            .min_h(px(metrics.control_height))
            .px(px(metrics.button_padding.0))
            .py(px(metrics.button_padding.1))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(metrics.corner_radius as f32))
            .bg(color(idle))
            .text_color(color(text))
            .hover(move |style| style.bg(color(hover)))
            .active(move |style| style.bg(color(active)))
            .child(label);

        let face = match bevel {
            // Only the outer ring, as the webview preview did.
            Some(edge) => ring(&edge, face.into_any_element()).into_any_element(),
            None => stroke_border(
                face,
                filled.map_or(widgets.idle.border, |(.., border)| border),
            )
            .into_any_element(),
        };

        match outline {
            Some(stroke) if stroke.width > 0. => div()
                .border(px(stroke.width))
                .border_color(color(stroke.color))
                .rounded(px(metrics.corner_radius as f32))
                .child(face)
                .into_any_element(),
            _ => face,
        }
    }

    fn body(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let widgets = &self.look.widgets;
        let metrics = &widgets.metrics;
        let (family, weight) = ui_theme::face(widgets.font);
        let field = window.use_keyed_state((self.id.clone(), "field"), cx, |window, cx| {
            InputState::new(window, cx).default_value("Yes")
        });

        // What `paint_dialog` pads a field by to reach the theme's control height.
        let row = widgets.font_size * 1.25;
        let vertical = ((metrics.control_height - row) / 2.).round().max(2.);
        let field_box = div()
            .w_full()
            .px(px(metrics.button_padding.0))
            .py(px(vertical))
            .bg(color(widgets.field))
            .text_color(color(widgets.text))
            .child(
                // Sized to one text row: the box around it supplies the theme's padding.
                Input::new(&field)
                    .appearance(false)
                    .h(px(row))
                    .p_0()
                    .text_size(px(widgets.font_size)),
            );
        let field_box = match widgets.edge {
            // A field is permanently recessed under a bevelled theme: the inverted rings a held
            // button uses, as `bevel::input_edge` paints it.
            WidgetEdge::Bevel { pressed, .. } => match pressed.first() {
                Some(edge) => ring(edge, field_box.into_any_element())
                    .w_full()
                    .into_any_element(),
                None => field_box.into_any_element(),
            },
            WidgetEdge::Flat => stroke_border(
                field_box.rounded(px(metrics.corner_radius as f32)),
                widgets.idle.border,
            )
            .into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .items_center()
            .p(px(DIALOG_MARGIN))
            .gap(px(metrics.item_spacing.1 + DIALOG_MARGIN))
            .bg(color(widgets.panel))
            .text_color(color(widgets.text))
            .font_family(family)
            .font_weight(weight)
            .text_size(px(widgets.font_size))
            .child("Are you sure?")
            .child(field_box)
            .child(
                div()
                    .flex()
                    .gap(px(metrics.item_spacing.0))
                    .child(self.dialog_button("cancel", "Cancel", false))
                    .child(self.dialog_button("ok", "OK", true)),
            )
            .into_any_element()
    }
}

impl RenderOnce for ThemePreview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let inner = div()
            .w(px(self.width))
            .flex()
            .flex_col()
            .child(self.header(window, cx))
            .child(self.body(window, cx))
            .into_any_element();
        div()
            .id(self.id.clone())
            .flex()
            .child(rings(self.look.chrome.border, inner))
    }
}
