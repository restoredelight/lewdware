//! Size and position, drawn against a frame standing in for the screen. Ported from
//! `pack-editor/src/lib/PopupPlacement.svelte`.
//!
//! A mode the viewer enters from the rail's "Size & position" button, not its resting state: most
//! visits to a popup file are about looking at it. The frame is the reference screen
//! (`REFERENCE_SCREEN`) scaled down, with the popup drawn at its real size and where the engine
//! would put it, and the spawn region over it. Drag on the screen to draw a region, drag the
//! region or its handles to change it, drag the popup's corner to resize it. A drag writes once,
//! on release; the presets, the nine pins and the numeric fields write as they're used.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::{
    canvas, div, px, rgba, AnyElement, Bounds, Context, CursorStyle, DispatchPhase, MouseButton,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, Window,
};
use pack_editor_core::pack::MediaFile;
use pack_editor_core::session::popup::{
    describe_region, from_edges, is_capped, is_full, place_in_region, point_region, popup_size,
    region_point, scale_for_width, snap, AREAS, POINTS, REFERENCE_SCREEN,
};
use shared::behaviour::SpawnRegion;

use super::{Field, Viewer};
use crate::icons::icon;
use crate::theme::{self, c};
use crate::ui::{self, Button, Size, Variant};

/// The popup's titlebar in the frame.
const MINI_HEADER_H: f32 = 18.;
/// How far a press must travel before it counts as a drag: a plain click must not pin a file to
/// a 3px box.
const DRAG_THRESHOLD: f32 = 3.;
/// The room the panel under the frame takes.
const PANEL_H: f32 = 250.;

/// Which edges a drag moves. `Move` slides the region; `New` draws a fresh one.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Grip {
    Move,
    New,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Grip {
    fn has(self, edge: char) -> bool {
        let name = match self {
            Grip::N => "n",
            Grip::S => "s",
            Grip::E => "e",
            Grip::W => "w",
            Grip::NE => "ne",
            Grip::NW => "nw",
            Grip::SE => "se",
            Grip::SW => "sw",
            Grip::Move | Grip::New => "",
        };
        name.contains(edge)
    }
}

struct Drag {
    grip: Grip,
    start: SpawnRegion,
    /// Where the press was, as fractions of the frame, and in window pixels.
    origin: (f64, f64),
    origin_px: Point<Pixels>,
    moved: bool,
}

/// The frame's in-progress gestures, reset when the viewer enters or leaves it.
#[derive(Default)]
pub(super) struct State {
    drag: Option<Drag>,
    /// The region mid-drag, written once on release.
    draft: Option<SpawnRegion>,
    resizing: bool,
    /// The scale mid-resize (`Some(None)` is "back to automatic"), written once on release.
    draft_scale: Option<Option<f64>>,
    /// The frame's bounds as last painted, and the popup's left edge and the frame's scale in
    /// it, for turning pointer positions into regions and sizes.
    frame: Rc<Cell<Bounds<Pixels>>>,
    popup_left: f32,
    /// The popup's top-left while its corner is dragged. Held still, so the corner follows the
    /// pointer instead of the popup re-centring under it; it re-centres on release.
    resize_anchor: Option<(f32, f32)>,
    ratio: f32,
    media_width: f64,
}

impl Viewer {
    fn region_now(&self) -> SpawnRegion {
        self.placement
            .draft
            .or(self.current_attributes().region)
            .unwrap_or(SpawnRegion::FULL)
    }

    fn commit_region(&mut self, region: SpawnRegion, what: &str, cx: &mut Context<Self>) {
        let (targets, label) = (self.targets(), self.label(what, cx));
        self.apply(cx, |s| s.set_popup_region(&targets, Some(region), &label));
    }

    fn fraction(&self, position: Point<Pixels>) -> (f64, f64) {
        let bounds = self.placement.frame.get();
        let x = (position.x - bounds.origin.x) / bounds.size.width.max(px(1.));
        let y = (position.y - bounds.origin.y) / bounds.size.height.max(px(1.));
        ((x as f64).clamp(0., 1.), (y as f64).clamp(0., 1.))
    }

    fn start_drag(&mut self, grip: Grip, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.placement.resizing {
            return;
        }
        let origin = self.fraction(position);
        let current = self.region_now().sanitized();
        self.placement.drag = Some(Drag {
            grip,
            start: if grip == Grip::New {
                SpawnRegion {
                    x: origin.0,
                    y: origin.1,
                    width: 0.,
                    height: 0.,
                }
            } else {
                current
            },
            origin,
            origin_px: position,
            moved: false,
        });
        self.placement.draft = Some(current);
        cx.notify();
    }

    fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let at = self.fraction(position);
        let Some(drag) = &mut self.placement.drag else {
            return;
        };
        let travelled = (position.x - drag.origin_px.x)
            .abs()
            .max((position.y - drag.origin_px.y).abs());
        if travelled > px(DRAG_THRESHOLD) {
            drag.moved = true;
        }
        if !drag.moved {
            return;
        }
        let (start, origin) = (drag.start, drag.origin);
        let draft = match drag.grip {
            // A moved region keeps its size; sanitising slides it back on rather than shrinking.
            Grip::Move => {
                let x = snap(start.x + (at.0 - origin.0)).clamp(0., 1. - start.width);
                let y = snap(start.y + (at.1 - origin.1)).clamp(0., 1. - start.height);
                SpawnRegion { x, y, ..start }.sanitized()
            }
            Grip::New => from_edges(snap(origin.0), snap(origin.1), snap(at.0), snap(at.1)),
            grip => {
                let mut left = start.x;
                let mut top = start.y;
                let mut right = start.x + start.width;
                let mut bottom = start.y + start.height;
                if grip.has('w') {
                    left = snap(at.0);
                }
                if grip.has('e') {
                    right = snap(at.0);
                }
                if grip.has('n') {
                    top = snap(at.1);
                }
                if grip.has('s') {
                    bottom = snap(at.1);
                }
                from_edges(left, top, right, bottom)
            }
        };
        self.placement.draft = Some(draft);
        cx.notify();
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(drag) = self.placement.drag.take() {
            let draft = self.placement.draft.take();
            // A press that never became a drag was a click, and a click isn't an edit.
            if let (true, Some(region)) = (drag.moved, draft) {
                self.commit_region(region, "Set popup area", cx);
            }
        }
        if self.placement.resizing {
            self.placement.resizing = false;
            self.placement.resize_anchor = None;
            if let Some(scale) = self.placement.draft_scale.take() {
                let (targets, label) = (self.targets(), self.label("Set popup size", cx));
                self.apply(cx, |s| s.set_popup_scale(&targets, scale, &label));
            }
        }
        cx.notify();
    }

    fn resize_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let bounds = self.placement.frame.get();
        let anchor = self
            .placement
            .resize_anchor
            .map_or(self.placement.popup_left, |(left, _)| left);
        let left = f32::from(bounds.origin.x) + anchor;
        let target = (f32::from(position.x) - left).max(8.) / self.placement.ratio.max(0.0001);
        self.placement.draft_scale = Some(scale_for_width(
            self.placement.media_width,
            target.round() as f64,
        ));
        cx.notify();
    }

    pub(super) fn placement(
        &mut self,
        file: &MediaFile,
        room: (f32, f32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let screen = REFERENCE_SCREEN;
        let frame_w = room
            .0
            .min(1100.)
            .min((room.1 - PANEL_H) * 16. / 9.)
            .max(260.);
        let frame_h = frame_w * (screen.1 / screen.0) as f32;
        let ratio = frame_w / screen.0 as f32;

        let attributes = self.current_attributes();
        let scale = self.placement.draft_scale.unwrap_or(attributes.scale);
        let dims = Self::media_size(file)
            .map(|(w, h)| (w as f64, h as f64))
            .unwrap_or((0., 0.));
        let (width, height) = popup_size(dims, scale, screen);
        let capped = is_capped(dims, scale, screen);
        let shown = ((width as f32 * ratio).max(44.), height as f32 * ratio);
        let outer_h = (shown.1 + MINI_HEADER_H).max(30.);

        let region = self.region_now();
        let area = region.sanitized();
        let full = is_full(Some(region));
        let point = region_point(Some(region));
        let popup_left = place_in_region(
            area.x * frame_w as f64,
            area.width * frame_w as f64,
            shown.0 as f64,
            frame_w as f64,
        ) as f32;
        let popup_top = place_in_region(
            area.y * frame_h as f64,
            area.height * frame_h as f64,
            outer_h as f64,
            frame_h as f64,
        ) as f32;
        let (popup_left, popup_top) = self
            .placement
            .resize_anchor
            .unwrap_or((popup_left, popup_top));
        self.placement.popup_left = popup_left;
        self.placement.ratio = ratio;
        self.placement.media_width = dims.0;

        let entity = cx.entity();
        let tracking = self.placement.drag.is_some() || self.placement.resizing;
        let frame_cell = self.placement.frame.clone();
        let listener = entity.downgrade();
        // Tracks the gesture across the whole window, so a drag that leaves the frame keeps going.
        let tracker = canvas(
            move |bounds, _, _| frame_cell.set(bounds),
            move |_, _, window, _| {
                if !tracking {
                    return;
                }
                let moved = listener.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        let _ = moved.update(cx, |this, cx| {
                            if this.placement.resizing {
                                this.resize_to(event.position, cx)
                            } else {
                                this.drag_to(event.position, cx)
                            }
                        });
                    }
                });
                let released = listener.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        let _ = released.update(cx, |this, cx| this.end_drag(cx));
                    }
                });
            },
        )
        .absolute()
        .size_full();

        let caption: SharedString = attributes
            .caption
            .clone()
            .unwrap_or_else(|| "From the caption pool".into())
            .into();
        let resize = entity.clone();
        let popup = div()
            .id("placement-popup")
            .absolute()
            .left(px(popup_left))
            .top(px(popup_top))
            .w(px(shown.0))
            .h(px(outer_h))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .bg(rgba(0xf8f8f8ff))
            .shadow(vec![gpui_kit::BoxShadow {
                color: gpui_kit::hsla(0., 0., 0., 0.55),
                offset: gpui_kit::point(px(6.), px(6.)),
                blur_radius: px(0.),
                spread_radius: px(0.),
                inset: false,
            }])
            .cursor(CursorStyle::Arrow)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_none()
                    .h(px(MINI_HEADER_H))
                    .px_1()
                    .flex()
                    .items_center()
                    .gap_1()
                    .border_b_1()
                    .border_color(rgba(0x00000026))
                    .bg(rgba(0xe4e4e4ff))
                    .child(
                        div()
                            .size(px(5.))
                            .flex_none()
                            .rounded_full()
                            .bg(c(theme::ACCENT)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(9.))
                            .text_color(if attributes.caption.is_some() {
                                rgba(0x222222ff)
                            } else {
                                rgba(0x00000059)
                            })
                            .child(caption),
                    )
                    .child(
                        div()
                            .text_color(rgba(0x00000073))
                            .child(icon(gpui_kit::assets::IconName::Close).size(px(10.))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .bg(gpui_kit::black())
                    .child(self.preview.clone()),
            )
            .child(
                div()
                    .id("placement-resize")
                    .absolute()
                    .right_0()
                    .bottom_0()
                    .size(px(14.))
                    .bg(c(theme::ACCENT))
                    .cursor(CursorStyle::ResizeUpLeftDownRight)
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        resize.update(cx, |this, cx| {
                            this.placement.resizing = true;
                            this.placement.resize_anchor = Some((popup_left, popup_top));
                            cx.notify();
                        });
                    }),
            );

        // The region, always drawn: at full screen it outlines the frame (dashed, and transparent
        // to the pointer so a new one can be drawn), at a point it's a ring.
        let region_el = if point.is_some() {
            div()
                .absolute()
                .left(px(area.x as f32 * frame_w - 7.))
                .top(px(area.y as f32 * frame_h - 7.))
                .size(px(14.))
                .rounded_full()
                .border_2()
                .border_color(c(theme::ACCENT_HOVER))
                .into_any_element()
        } else if full {
            div()
                .absolute()
                .size_full()
                .border_2()
                .border_dashed()
                .border_color(c(theme::ACCENT_HOVER))
                .into_any_element()
        } else {
            let mover = entity.clone();
            let handles = [
                (Grip::NW, 0., 0., CursorStyle::ResizeUpLeftDownRight),
                (Grip::N, 0.5, 0., CursorStyle::ResizeUpDown),
                (Grip::NE, 1., 0., CursorStyle::ResizeUpRightDownLeft),
                (Grip::E, 1., 0.5, CursorStyle::ResizeLeftRight),
                (Grip::SE, 1., 1., CursorStyle::ResizeUpLeftDownRight),
                (Grip::S, 0.5, 1., CursorStyle::ResizeUpDown),
                (Grip::SW, 0., 1., CursorStyle::ResizeUpRightDownLeft),
                (Grip::W, 0., 0.5, CursorStyle::ResizeLeftRight),
            ];
            let (w, h) = (area.width as f32 * frame_w, area.height as f32 * frame_h);
            div()
                .id("placement-region")
                .absolute()
                .left(px(area.x as f32 * frame_w))
                .top(px(area.y as f32 * frame_h))
                .w(px(w))
                .h(px(h))
                .border_2()
                .border_color(c(theme::ACCENT_HOVER))
                .cursor(CursorStyle::OpenHand)
                .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                    cx.stop_propagation();
                    let position = event.position;
                    mover.update(cx, |this, cx| this.start_drag(Grip::Move, position, cx));
                })
                .children(handles.into_iter().map(|(grip, fx, fy, cursor)| {
                    let grab = entity.clone();
                    div()
                        .id(SharedString::from(format!("handle-{grip:?}")))
                        .absolute()
                        .left(px(fx * w - 5.))
                        .top(px(fy * h - 5.))
                        .size(px(9.))
                        .border_1()
                        .border_color(c(theme::ACCENT_HOVER))
                        .bg(c(theme::SURFACE_RAISED))
                        .cursor(cursor)
                        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                            cx.stop_propagation();
                            let position = event.position;
                            grab.update(cx, |this, cx| this.start_drag(grip, position, cx));
                        })
                }))
                .into_any_element()
        };

        let draw = entity.clone();
        let frame = div()
            .id("placement-frame")
            .relative()
            .w(px(frame_w))
            .h(px(frame_h))
            .overflow_hidden()
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .bg(c(theme::SURFACE))
            .cursor(CursorStyle::Crosshair)
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                let position = event.position;
                draw.update(cx, |this, cx| this.start_drag(Grip::New, position, cx));
            })
            .child(tracker)
            .child(popup)
            .child(region_el);

        // ── The panel ──
        let pins = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .children((0..3).map(|row| {
                div().flex().gap(px(2.)).children((0..3).map(|column| {
                    let on = point == Some((row, column));
                    let pin = entity.clone();
                    div()
                        .id(SharedString::from(format!("pin-{row}-{column}")))
                        .size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(theme::RADIUS_SM)
                        .border_1()
                        .border_color(c(theme::BORDER))
                        .bg(c(if on { theme::SURFACE_RAISED } else { theme::BG }))
                        .cursor_pointer()
                        .hover(|style| style.border_color(c(theme::ACCENT)))
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(format!(
                                "Pin {}",
                                POINTS[row][column]
                            ))
                            .build(window, cx)
                        })
                        .child(div().size(px(4.)).rounded_full().bg(c(if on {
                            theme::ACCENT_HOVER
                        } else {
                            theme::MUTED
                        })))
                        .on_click(move |_, _, cx| {
                            pin.update(cx, |this, cx| {
                                this.commit_region(point_region(row, column), "Pin popup", cx)
                            })
                        })
                }))
            }));
        let preset = |id: &'static str, label: &'static str, target: SpawnRegion, current: bool| {
            let set = entity.clone();
            Button::new(id, label)
                .variant(if current {
                    Variant::Secondary
                } else {
                    Variant::Quiet
                })
                .size(Size::Compact)
                .on_click(move |_, _, cx| {
                    set.update(cx, |this, cx| {
                        this.commit_region(target, "Set popup area", cx)
                    })
                })
        };
        let presets = div()
            .flex()
            .flex_wrap()
            .gap(px(5.))
            .child(preset("area-full", "Anywhere", SpawnRegion::FULL, full))
            .children(AREAS.iter().enumerate().map(|(index, (label, target))| {
                let ids = ["area-0", "area-1", "area-2", "area-3", "area-4"];
                preset(ids[index], label, *target, !full && *target == area)
            }));

        let labelled = |label: &'static str, field: Div| {
            div()
                .w(px(112.))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(c(theme::MUTED))
                        .child(label),
                )
                .child(field)
        };
        let mut fields = div().flex().gap_2().child(labelled(
            "Size (x)",
            self.number_field(Field::Scale, None, window, cx),
        ));
        if !full {
            for (label, field) in [
                ("Left", Field::Left),
                ("Top", Field::Top),
                ("Width", Field::Width),
                ("Height", Field::Height),
            ] {
                fields = fields.child(labelled(
                    label,
                    self.number_field(field, Some("%"), window, cx),
                ));
            }
        }

        let count = self.targets().len();
        let (reset, done) = (entity.clone(), entity.clone());
        let bar = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(6.))
                    .child(ui::readout(format!("{width} x {height}")).text_color(c(theme::TEXT)))
                    .child(ui::readout(format!(
                        "on a {} x {} screen{}",
                        screen.0,
                        screen.1,
                        if capped { ", at the size limit" } else { "" }
                    ))),
            )
            .child(div().flex_1())
            .child(ui::readout(format!(
                "Spawns {}",
                describe_region(Some(region))
            )))
            .when(count > 1, |this| {
                this.child(ui::readout(format!("Applies to all {count}")))
            })
            .when(attributes.scale.is_some(), |this| {
                this.child(
                    Button::new("reset-size", "Reset size")
                        .variant(Variant::Quiet)
                        .size(Size::Compact)
                        .on_click(move |_, _, cx| {
                            reset.update(cx, |this, cx| {
                                let (targets, label) =
                                    (this.targets(), this.label("Set popup size", cx));
                                this.apply(cx, |s| s.set_popup_scale(&targets, None, &label));
                            })
                        }),
                )
            })
            .child(
                Button::new("placement-done", "Done")
                    .size(Size::Compact)
                    .on_click(move |_, _, cx| {
                        done.update(cx, |this, cx| this.set_placing(false, cx))
                    }),
            );

        let panel = div()
            .w(px(frame_w))
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::BG))
            .text_color(c(theme::TEXT))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(c(theme::MUTED))
                                    .child("Pin to"),
                            )
                            .child(pins),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(c(theme::MUTED))
                                    .child("Or an area"),
                            )
                            .child(presets)
                            .child(
                                ui::hint("Or drag out a rectangle on the screen above.")
                                    .text_size(px(11.)),
                            ),
                    ),
            )
            .child(fields)
            .child(bar);

        div()
            .id("placement")
            .occlude()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(frame)
            .child(panel)
            .into_any_element()
    }
}

type Div = gpui_kit::Div;
