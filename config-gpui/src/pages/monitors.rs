//! Which screens popups may use, and how much of each. Ported from `config/src/lib/Monitors.svelte`
//! and `MonitorAreas.svelte`.
//!
//! The diagram shows the desk to scale. Clicking a screen zooms in on it, where an area can be
//! dragged out, moved, and resized by its handles, with presets and percentage fields beside it.
//! Dragging works on a draft and saves once, on release.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    canvas, div, px, AnyElement, Bounds, Context, CursorStyle, DispatchPhase, Entity, MouseButton,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Subscription, Window,
};
use shared::monitor::{MonitorInfo, MonitorRegion, MIN_REGION_SIZE};

use crate::store::Store;
use crate::theme::{self, c};
use crate::ui::{self, Button, Size, Variant};

/// Which edges a drag moves. `Move` slides the whole rectangle; `New` draws a fresh one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    const HANDLES: [(Grip, f32, f32); 8] = [
        (Grip::NW, 0., 0.),
        (Grip::N, 0.5, 0.),
        (Grip::NE, 1., 0.),
        (Grip::E, 1., 0.5),
        (Grip::SE, 1., 1.),
        (Grip::S, 0.5, 1.),
        (Grip::SW, 0., 1.),
        (Grip::W, 0., 0.5),
    ];

    fn is_corner(self) -> bool {
        matches!(self, Grip::NE | Grip::NW | Grip::SE | Grip::SW)
    }

    fn moves_west(self) -> bool {
        matches!(self, Grip::W | Grip::NW | Grip::SW)
    }
    fn moves_east(self) -> bool {
        matches!(self, Grip::E | Grip::NE | Grip::SE)
    }
    fn moves_north(self) -> bool {
        matches!(self, Grip::N | Grip::NE | Grip::NW)
    }
    fn moves_south(self) -> bool {
        matches!(self, Grip::S | Grip::SE | Grip::SW)
    }

    fn cursor(self) -> CursorStyle {
        match self {
            Grip::N | Grip::S => CursorStyle::ResizeUpDown,
            Grip::E | Grip::W => CursorStyle::ResizeLeftRight,
            Grip::NW | Grip::SE => CursorStyle::ResizeUpLeftDownRight,
            Grip::NE | Grip::SW => CursorStyle::ResizeUpRightDownLeft,
            Grip::Move => CursorStyle::OpenHand,
            Grip::New => CursorStyle::Crosshair,
        }
    }
}

const PRESETS: [(&str, MonitorRegion); 6] = [
    ("Whole screen", MonitorRegion::FULL),
    (
        "Left half",
        MonitorRegion {
            x: 0.,
            y: 0.,
            width: 0.5,
            height: 1.,
        },
    ),
    (
        "Right half",
        MonitorRegion {
            x: 0.5,
            y: 0.,
            width: 0.5,
            height: 1.,
        },
    ),
    (
        "Top half",
        MonitorRegion {
            x: 0.,
            y: 0.,
            width: 1.,
            height: 0.5,
        },
    ),
    (
        "Bottom half",
        MonitorRegion {
            x: 0.,
            y: 0.5,
            width: 1.,
            height: 0.5,
        },
    ),
    (
        "Centre",
        MonitorRegion {
            x: 0.25,
            y: 0.25,
            width: 0.5,
            height: 0.5,
        },
    ),
];

/// Edges a dragged edge sticks to within `SNAP` of. Halves, thirds and quarters are what people
/// want, and hitting them exactly by hand is fiddly.
const SNAP_LINES: [f64; 7] = [0., 0.25, 1. / 3., 0.5, 2. / 3., 0.75, 1.];
const SNAP: f64 = 0.015;
/// How far the pointer must travel, in pixels, before a press counts as a drag. A plain click on
/// a screen only selects it; otherwise every mis-click would restrict a monitor to a tiny box.
const DRAG_THRESHOLD: f32 = 3.;

/// The zoomed-in screen is edited at up to this height.
const EDIT_HEIGHT: f32 = 300.;
const EDIT_MAX_WIDTH: f32 = 600.;
/// The overview's height, and the aspect ratio past which it scrolls sideways rather than
/// squashing every screen into a smear (six monitors in a row, say).
const OVERVIEW_HEIGHT: f32 = 180.;
const OVERVIEW_MAX_WIDTH: f32 = 760.;
const WIDE_ARRANGEMENT: f64 = 6.;

fn snap(value: f64) -> f64 {
    SNAP_LINES
        .iter()
        .copied()
        .find(|line| (line - value).abs() <= SNAP)
        .unwrap_or(value)
}

fn round(value: f64) -> f64 {
    (value * 1000.).round() / 1000.
}

/// The smallest region a monitor can express, as a fraction of it. Mirrors the engine's floor
/// (`MIN_REGION_SIZE`), which is in logical pixels.
fn min_fraction(monitor: &MonitorInfo) -> (f64, f64) {
    let scale = if monitor.scale_factor > 0. {
        monitor.scale_factor
    } else {
        1.
    };
    let min = MIN_REGION_SIZE as f64;
    (
        (min / (monitor.width as f64 / scale).max(1.)).min(1.),
        (min / (monitor.height as f64 / scale).max(1.)).min(1.),
    )
}

/// Forces a region inside its monitor and up to the minimum size, keeping the user's intent where
/// it can: an over-large rectangle shrinks, an off-edge one slides back in.
fn clamp(region: MonitorRegion, monitor: &MonitorInfo) -> MonitorRegion {
    let (min_x, min_y) = min_fraction(monitor);
    let width = region.width.max(min_x).min(1.);
    let height = region.height.max(min_y).min(1.);
    MonitorRegion {
        x: round(region.x.max(0.).min(1. - width)),
        y: round(region.y.max(0.).min(1. - height)),
        width: round(width),
        height: round(height),
    }
}

/// A region from its four edges, tolerating a drag that crossed over itself.
fn from_edges(left: f64, top: f64, right: f64, bottom: f64) -> MonitorRegion {
    MonitorRegion {
        x: left.min(right),
        y: top.min(bottom),
        width: (right - left).abs(),
        height: (bottom - top).abs(),
    }
}

fn percent(value: f64) -> i64 {
    (value * 100.).round() as i64
}

/// A region's size in the logical pixels a mode would see, which is what governs how much room a
/// popup has.
fn logical_size(monitor: &MonitorInfo, region: &MonitorRegion) -> String {
    let scale = if monitor.scale_factor > 0. {
        monitor.scale_factor
    } else {
        1.
    };
    format!(
        "{}×{}",
        ((monitor.width as f64 / scale) * region.width).round(),
        ((monitor.height as f64 / scale) * region.height).round()
    )
}

struct Drag {
    id: String,
    grip: Grip,
    start: MonitorRegion,
    /// Where in the monitor the pointer went down, as a fraction.
    origin: (f64, f64),
    origin_px: Point<Pixels>,
    moved: bool,
}

#[derive(Clone, Copy)]
enum Edge {
    X,
    Y,
    Width,
    Height,
}

pub struct MonitorsPage {
    store: Entity<Store>,
    /// The monitor being edited. Nothing is the resting state: the diagram alone answers "which
    /// screens do I have, and where may popups go".
    selected: Option<String>,
    /// The region being dragged, held here rather than saved on every move.
    draft: Option<(String, MonitorRegion)>,
    drag: Option<Drag>,
    /// The zoomed-in screen's on-screen box, captured at paint for the drag maths.
    screen_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    fields: [(Edge, &'static str, Entity<InputState>); 4],
    _subscriptions: Vec<Subscription>,
}

impl MonitorsPage {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let make =
            |window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx));
        let fields = [
            (Edge::X, "Left", make(window, cx)),
            (Edge::Y, "Top", make(window, cx)),
            (Edge::Width, "Width", make(window, cx)),
            (Edge::Height, "Height", make(window, cx)),
        ];
        let mut subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        for (edge, _, state) in &fields {
            let edge = *edge;
            subscriptions.push(cx.subscribe_in(
                state,
                window,
                move |this, state, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let value = state
                            .read(cx)
                            .value()
                            .trim()
                            .trim_end_matches('%')
                            .to_string();
                        this.set_edge(edge, &value, window, cx);
                    }
                },
            ));
        }
        Self {
            store,
            selected: None,
            draft: None,
            drag: None,
            screen_bounds: Rc::new(Cell::new(None)),
            fields,
            _subscriptions: subscriptions,
        }
    }

    fn region_of(&self, monitor: &MonitorInfo, cx: &Context<Self>) -> MonitorRegion {
        match &self.draft {
            Some((id, region)) if id == &monitor.id => *region,
            _ => self.store.read(cx).monitor_region(&monitor.id),
        }
    }

    fn selected_monitor(&self, cx: &Context<Self>) -> Option<MonitorInfo> {
        let store = self.store.read(cx);
        let id = self.selected.as_ref()?;
        // Derived rather than stored, so a monitor switched off mid-edit can't leave the controls
        // pointing at something that is gone.
        store
            .monitors
            .iter()
            .find(|m| &m.id == id && store.monitor_enabled(&m.id))
            .cloned()
    }

    fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = id;
        self.sync_fields(window, cx);
        cx.notify();
    }

    /// Writes the current region into the percentage fields. Called whenever it changes other
    /// than by typing, and after typing, so a value that clamped shows what was actually kept.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(monitor) = self.selected_monitor(cx) else {
            return;
        };
        let region = self.region_of(&monitor, cx);
        for (edge, _, state) in &self.fields {
            let value = match edge {
                Edge::X => region.x,
                Edge::Y => region.y,
                Edge::Width => region.width,
                Edge::Height => region.height,
            };
            state.update(cx, |state, cx| {
                state.set_value(percent(value).to_string(), window, cx)
            });
        }
    }

    /// The single write path. A whole-screen region is stored as no entry at all, and an unchanged
    /// one isn't written.
    fn commit(
        &mut self,
        monitor: &MonitorInfo,
        region: MonitorRegion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = clamp(region, monitor);
        let current = self.store.read(cx).monitor_region(&monitor.id);
        if next != current {
            let id = monitor.id.clone();
            let stored = (!next.is_full()).then_some(next);
            self.store
                .update(cx, |store, cx| store.set_monitor_region(id, stored, cx));
        }
        self.sync_fields(window, cx);
    }

    fn set_edge(&mut self, edge: Edge, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(monitor) = self.selected_monitor(cx) else {
            return;
        };
        let mut region = self.region_of(&monitor, cx);
        if let Ok(parsed) = value.parse::<f64>() {
            let fraction = parsed / 100.;
            match edge {
                Edge::X => region.x = fraction,
                Edge::Y => region.y = fraction,
                Edge::Width => region.width = fraction,
                Edge::Height => region.height = fraction,
            }
        }
        self.commit(&monitor, region, window, cx);
    }

    fn fraction_at(&self, position: Point<Pixels>) -> Option<(f64, f64)> {
        let bounds = self.screen_bounds.get()?;
        let x = (position.x - bounds.origin.x) / bounds.size.width.max(px(1.));
        let y = (position.y - bounds.origin.y) / bounds.size.height.max(px(1.));
        Some((x.clamp(0., 1.) as f64, y.clamp(0., 1.) as f64))
    }

    fn start_drag(
        &mut self,
        monitor: &MonitorInfo,
        grip: Grip,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(origin) = self.fraction_at(position) else {
            return;
        };
        let current = self.region_of(monitor, cx);
        let start = if grip == Grip::New {
            MonitorRegion {
                x: origin.0,
                y: origin.1,
                width: 0.,
                height: 0.,
            }
        } else {
            current
        };
        self.drag = Some(Drag {
            id: monitor.id.clone(),
            grip,
            start,
            origin,
            origin_px: position,
            moved: false,
        });
        self.draft = Some((monitor.id.clone(), current));
        cx.notify();
    }

    fn move_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(at) = self.fraction_at(position) else {
            return;
        };
        let Some(monitor) = self.selected_monitor(cx) else {
            return;
        };
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        if !drag.moved {
            let dx = (position.x - drag.origin_px.x).abs();
            let dy = (position.y - drag.origin_px.y).abs();
            if dx <= px(DRAG_THRESHOLD) && dy <= px(DRAG_THRESHOLD) {
                return;
            }
            drag.moved = true;
        }

        let (start, origin) = (drag.start, drag.origin);
        let next = match drag.grip {
            // A moved rectangle keeps its size; clamping slides it back rather than shrinking it.
            Grip::Move => MonitorRegion {
                x: snap(start.x + (at.0 - origin.0)),
                y: snap(start.y + (at.1 - origin.1)),
                ..start
            },
            Grip::New => from_edges(snap(origin.0), snap(origin.1), snap(at.0), snap(at.1)),
            grip => {
                let mut left = start.x;
                let mut top = start.y;
                let mut right = start.x + start.width;
                let mut bottom = start.y + start.height;
                if grip.moves_west() {
                    left = snap(at.0);
                }
                if grip.moves_east() {
                    right = snap(at.0);
                }
                if grip.moves_north() {
                    top = snap(at.1);
                }
                if grip.moves_south() {
                    bottom = snap(at.1);
                }
                from_edges(left, top, right, bottom)
            }
        };
        self.draft = Some((drag.id.clone(), clamp(next, &monitor)));
        cx.notify();
    }

    fn end_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let draft = self.draft.take();
        // A press that never became a drag was a click: it selected the monitor, and that's all.
        if drag.moved {
            if let (Some(monitor), Some((_, region))) = (self.selected_monitor(cx), draft) {
                self.commit(&monitor, region, window, cx);
            }
        }
        cx.notify();
    }

    /// One screen in the overview: a target, not a canvas. A click zooms in on it.
    fn overview_screen(&self, monitor: &MonitorInfo, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.store.read(cx).monitor_enabled(&monitor.id);
        let region = self.region_of(monitor, cx);
        let id = monitor.id.clone();
        div()
            .id(gpui_kit::SharedString::from(format!(
                "overview-{}",
                monitor.id
            )))
            .relative()
            .size_full()
            .rounded(theme::RADIUS_SM)
            .border_1()
            .map(|this| {
                if enabled {
                    this.border_color(c(theme::BORDER))
                        .bg(c(theme::SURFACE))
                        .cursor_pointer()
                        .hover(|style| style.border_color(c(theme::BORDER_STRONG)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                // Stop it reaching the ground, which clears the selection.
                                cx.stop_propagation();
                                this.select(Some(id.clone()), window, cx);
                            }),
                        )
                        .child(
                            region_box(&region)
                                .bg(c(theme::SURFACE_RAISED))
                                .border_1()
                                .border_color(c(theme::BORDER_STRONG)),
                        )
                } else {
                    this.border_dashed()
                        .border_color(c(theme::BORDER))
                        .bg(c(theme::BG))
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(ui::readout("Off").text_size(px(10.))),
                        )
                }
            })
            .child(screen_label(&monitor.name))
            .into_any_element()
    }

    fn overview(&self, cx: &mut Context<Self>) -> AnyElement {
        let monitors = self.store.read(cx).monitors.clone();
        // An engine older than this picker reports no geometry, so every monitor claims the
        // origin. Lay them out in a row rather than stacking them on top of each other.
        let no_geometry = monitors.len() > 1 && monitors.iter().all(|m| m.x == 0 && m.y == 0);
        let mut cursor = 0i64;
        let placed: Vec<(MonitorInfo, i64, i64)> = monitors
            .into_iter()
            .map(|m| {
                if no_geometry {
                    let x = cursor;
                    cursor += m.width as i64;
                    (m, x, 0)
                } else {
                    let (x, y) = (m.x as i64, m.y as i64);
                    (m, x, y)
                }
            })
            .collect();

        let left = placed.iter().map(|p| p.1).min().unwrap_or(0);
        let top = placed.iter().map(|p| p.2).min().unwrap_or(0);
        let right = placed
            .iter()
            .map(|p| p.1 + p.0.width as i64)
            .max()
            .unwrap_or(1);
        let bottom = placed
            .iter()
            .map(|p| p.2 + p.0.height as i64)
            .max()
            .unwrap_or(1);
        let (bw, bh) = (
            ((right - left).max(1)) as f64,
            ((bottom - top).max(1)) as f64,
        );
        let ratio = bw / bh;
        let wide = ratio > WIDE_ARRANGEMENT;

        // To scale: one size for the whole desk, so the arrangement isn't stretched.
        let (width, height) = if wide {
            (OVERVIEW_HEIGHT as f64 * 150. / 180. * ratio, 150.)
        } else {
            let height = OVERVIEW_HEIGHT as f64;
            let width = (height * ratio).min(OVERVIEW_MAX_WIDTH as f64);
            (width, width / ratio)
        };

        let desk = div()
            .relative()
            .mx_auto()
            .flex_none()
            .w(px(width as f32))
            .h(px(height as f32))
            .children(placed.iter().map(|(monitor, x, y)| {
                div()
                    .absolute()
                    .left(px((((x - left) as f64 / bw) * width) as f32))
                    .top(px((((y - top) as f64 / bh) * height) as f32))
                    .w(px(((monitor.width as f64 / bw) * width) as f32))
                    .h(px(((monitor.height as f64 / bh) * height) as f32))
                    .p(px(3.))
                    .child(self.overview_screen(monitor, cx))
            }));

        if wide {
            div()
                .id("overview-scroll")
                .w_full()
                .overflow_x_scroll()
                .child(desk)
                .into_any_element()
        } else {
            div().w_full().flex().child(desk).into_any_element()
        }
    }

    /// The selected screen, zoomed in for editing. However small a screen is in the overview, it
    /// is edited at full size, which keeps this usable on a six-monitor desk.
    fn editor_screen(&self, monitor: &MonitorInfo, cx: &mut Context<Self>) -> AnyElement {
        let ratio = monitor.width as f32 / monitor.height.max(1) as f32;
        let (width, height) = if EDIT_HEIGHT * ratio > EDIT_MAX_WIDTH {
            (EDIT_MAX_WIDTH, EDIT_MAX_WIDTH / ratio)
        } else {
            (EDIT_HEIGHT * ratio, EDIT_HEIGHT)
        };
        let region = self.region_of(monitor, cx);
        let tiny = region.width < 0.15 || region.height < 0.15;
        let dragging = self.drag.is_some();

        let start = |grip: Grip, monitor: &MonitorInfo| {
            let monitor = monitor.clone();
            cx.listener(
                move |this: &mut Self, event: &gpui_kit::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.start_drag(&monitor, grip, event.position, cx);
                },
            )
        };

        // Eight handles on a tiny rectangle overlap into a clump none of which can be hit; keep
        // the corners only below that size.
        let handles = Grip::HANDLES
            .iter()
            .filter(|(grip, _, _)| !tiny || grip.is_corner())
            .map(|&(grip, fx, fy)| {
                div()
                    .absolute()
                    .left(gpui_kit::relative(fx))
                    .top(gpui_kit::relative(fy))
                    .ml(px(-5.))
                    .mt(px(-5.))
                    .size(px(10.))
                    .bg(c(theme::SURFACE))
                    .border_1()
                    .border_color(c(theme::ACCENT_HOVER))
                    .cursor(grip.cursor())
                    .on_mouse_down(MouseButton::Left, start(grip, monitor))
            })
            .collect::<Vec<_>>();

        let bounds_cell = self.screen_bounds.clone();
        let this = cx.entity().downgrade();
        // Records the screen's box, and while a drag is live follows the pointer anywhere in the
        // window -- the equivalent of pointer capture, so leaving the screen doesn't drop it.
        let tracker = canvas(
            move |bounds, _, _| bounds_cell.set(Some(bounds)),
            move |_, _, window, _| {
                if !dragging {
                    return;
                }
                let on_move = this.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        let _ = on_move.update(cx, |this, cx| this.move_drag(event.position, cx));
                    }
                });
                let on_up = this.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        let _ = on_up.update(cx, |this, cx| this.end_drag(window, cx));
                    }
                });
            },
        )
        .absolute()
        .inset_0();

        div()
            .relative()
            .mx_auto()
            .w(px(width))
            .h(px(height))
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .bg(c(theme::SURFACE))
            .cursor(CursorStyle::Crosshair)
            .child(tracker)
            .on_mouse_down(MouseButton::Left, start(Grip::New, monitor))
            .child(
                region_box(&region)
                    .bg(c(theme::SURFACE_RAISED))
                    .border_2()
                    .border_color(c(theme::ACCENT_HOVER))
                    .cursor(if dragging {
                        CursorStyle::ClosedHand
                    } else {
                        CursorStyle::OpenHand
                    })
                    .on_mouse_down(MouseButton::Left, start(Grip::Move, monitor))
                    .children(handles),
            )
            .child(screen_label(&monitor.name))
            .into_any_element()
    }

    fn editor_panel(&self, monitor: &MonitorInfo, cx: &mut Context<Self>) -> AnyElement {
        let region = self.region_of(monitor, cx);
        let summary = if region.is_full() {
            format!("whole screen · {}", logical_size(monitor, &region))
        } else {
            format!(
                "{} at {}%, {}%",
                logical_size(monitor, &region),
                percent(region.x),
                percent(region.y)
            )
        };

        let presets = PRESETS.iter().enumerate().map(|(index, (label, preset))| {
            let current = *preset == region;
            let monitor = monitor.clone();
            let preset = *preset;
            div()
                .rounded(theme::RADIUS_MD)
                // Selected: raised surface plus a carmine edge, never a fill.
                .when(current, |this| {
                    this.bg(c(theme::SURFACE_RAISED))
                        .border_l_2()
                        .border_color(c(theme::ACCENT_HOVER))
                })
                .child(
                    Button::new(("preset", index), *label)
                        .variant(Variant::Quiet)
                        .size(Size::Compact)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.commit(&monitor, preset, window, cx)
                        })),
                )
        });

        let fields = self.fields.iter().map(|(_, label, state)| {
            div()
                .flex_1()
                .min_w(px(96.))
                .flex()
                .flex_col()
                .gap_1()
                .child(ui::hint(*label))
                .child(
                    div()
                        .h(theme::CONTROL_COMPACT)
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded(theme::RADIUS_MD)
                        .border_1()
                        .border_color(c(theme::BORDER_STRONG))
                        .bg(c(theme::BG))
                        .child(
                            Input::new(state)
                                .appearance(false)
                                .flex_1()
                                .text_size(px(13.)),
                        )
                        .child(ui::readout("%")),
                )
        });

        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::SURFACE))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(ui::label(format!("Area on {}", monitor.name)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(ui::readout(summary))
                            .child(
                                Button::new("done", "Done")
                                    .variant(Variant::Quiet)
                                    .size(Size::Compact)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.select(None, window, cx)
                                    })),
                            ),
                    ),
            )
            .child(div().flex().flex_wrap().gap(px(6.)).children(presets))
            .child(div().flex().flex_wrap().gap_2().children(fields))
            .into_any_element()
    }

    fn monitor_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let mut list = ui::card();
        for (index, monitor) in store.monitors.iter().enumerate() {
            let enabled = store.monitor_enabled(&monitor.id);
            let region = store.monitor_region(&monitor.id);
            let id = monitor.id.clone();
            let entity = self.store.clone();
            if index > 0 {
                list = list.child(ui::seam());
            }
            list = list.child(
                div()
                    .id(("monitor-row", index))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .cursor_pointer()
                    .hover(|style| style.bg(c(theme::SURFACE_RAISED)))
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |store, cx| {
                            store.set_monitor_enabled(id.clone(), !enabled, cx)
                        })
                    })
                    .child(ui::checkbox(enabled))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .child(monitor.name.clone()),
                    )
                    .child(ui::readout(format!("{}×{}", monitor.width, monitor.height)))
                    .when(enabled && !region.is_full(), |this| {
                        this.child(ui::readout(format!(
                            "{}% × {}% area",
                            percent(region.width),
                            percent(region.height)
                        )))
                    })
                    .when(monitor.primary, |this| {
                        this.child(
                            div()
                                .px_2()
                                .py(px(2.))
                                .rounded_full()
                                .border_1()
                                .border_color(c(theme::BORDER))
                                .bg(c(theme::BG))
                                .text_size(px(10.))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(c(theme::MUTED))
                                .child("Primary"),
                        )
                    }),
            );
        }
        list.into_any_element()
    }

    fn placeholder(text: impl Into<gpui_kit::SharedString>) -> gpui_kit::Div {
        div()
            .p_6()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_dashed()
            .border_color(c(theme::BORDER_STRONG))
            .child(ui::hint(text))
    }
}

/// An area rectangle, positioned by its region within the screen.
fn region_box(region: &MonitorRegion) -> gpui_kit::Div {
    div()
        .absolute()
        .left(gpui_kit::relative(region.x as f32))
        .top(gpui_kit::relative(region.y as f32))
        .w(gpui_kit::relative(region.width as f32))
        .h(gpui_kit::relative(region.height as f32))
}

/// Drawn last, on its own ground, so a full-screen area can't hide which screen this is.
fn screen_label(name: &str) -> impl IntoElement {
    div()
        .absolute()
        .left(px(5.))
        .bottom(px(4.))
        .max_w(gpui_kit::relative(0.9))
        .px(px(3.))
        .rounded(theme::RADIUS_SM)
        .bg(c(theme::BG))
        .font_family(theme::FONT_MONO)
        .text_size(px(10.))
        .text_color(c(theme::MUTED))
        .truncate()
        .child(name.to_string())
}

impl Render for MonitorsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let body: AnyElement = if store.monitors_loading {
            Self::placeholder("Detecting monitors...").into_any_element()
        } else if let Some(error) = store.monitors_error.clone() {
            let entity = self.store.clone();
            Self::placeholder("")
                .child(
                    ui::label("Monitors couldn't be detected")
                        .text_size(px(12.))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD),
                )
                .child(ui::hint(error).max_w(px(448.)))
                .child(
                    Button::new("retry-monitors", "Try again")
                        .size(Size::Compact)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |store, cx| store.load_monitors(cx))
                        }),
                )
                .into_any_element()
        } else if store.monitors.is_empty() {
            Self::placeholder("No monitors were detected.").into_any_element()
        } else {
            let usable = store
                .monitors
                .iter()
                .filter(|m| store.monitor_enabled(&m.id))
                .count();
            let selected = self.selected_monitor(cx);
            let diagram = div()
                .id("diagram")
                .p_4()
                .rounded(theme::RADIUS_MD)
                .border_1()
                .border_color(c(theme::BORDER))
                .bg(c(theme::BG))
                // A press on the diagram's ground, not on a screen, means "stop editing".
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.select(None, window, cx)),
                )
                .child(match &selected {
                    Some(monitor) => self.editor_screen(monitor, cx),
                    None => self.overview(cx),
                });

            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(diagram)
                .child(match &selected {
                    Some(monitor) => self.editor_panel(monitor, cx),
                    None => ui::readout(if usable == 0 {
                        "Every monitor is switched off, so there is nowhere for popups to go."
                    } else {
                        "Click a screen to set the area popups may use on it."
                    })
                    .into_any_element(),
                })
                .child(self.monitor_list(cx))
                .into_any_element()
        };

        ui::page(
            "monitors",
            "Monitors",
            "Choose which screens popup windows may use, and how much of each one. Drag out an area on a screen to keep popups inside it; the rest of that screen is left alone.",
            [body],
        )
    }
}
