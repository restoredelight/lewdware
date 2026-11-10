use crate::{
    MediaKind, PreviewSource,
    element::{ImageElement, Images},
    worker::{Command, Event, Worker},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    Bounds, Context, FocusHandle, KeyDownEvent, MouseButton, Pixels, RenderImage, Subscription,
    Window, canvas, div, px, relative,
};
use shared_gpui::{
    icons::icon,
    theme::{self, c},
    ui,
};

/// The controls bar's height under a video or audio preview, for callers sizing a box to hug the
/// media: the picture gets whatever is left.
pub const CONTROLS_HEIGHT: f32 = 61.;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

/// A single preview. Call `load` to replace it and `clear` to stop it. Dropping the entity
/// stops its worker even if the window is gone; on_release_in frees its last painted image.
pub struct MediaView {
    worker: Option<Worker>,
    source: Option<PreviewSource>,
    image: Option<Arc<RenderImage>>,
    /// Halved copies of a still `image`, largest first.
    smaller: Vec<Arc<RenderImage>>,
    images: Rc<RefCell<Images>>,
    loading: bool,
    playing: bool,
    ended: bool,
    duration: Option<Duration>,
    position: Duration,
    volume: f32,
    unmuted_volume: f32,
    looping: bool,
    error: Option<String>,
    warning: Option<String>,
    focus: FocusHandle,
    seek_bounds: Rc<Cell<Bounds<Pixels>>>,
    volume_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Whether a timed preview draws its play/seek/volume bar. Off for a thumbnail-sized preview.
    controls: bool,
    _release: Subscription,
}

impl MediaView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _release = cx.on_release_in(window, |this, window, _| {
            this.worker = None;
            this.images.borrow_mut().clear(window);
        });
        Self {
            worker: None,
            source: None,
            image: None,
            smaller: Vec::new(),
            images: Rc::new(RefCell::new(Images::default())),
            loading: false,
            playing: false,
            ended: false,
            duration: None,
            position: Duration::ZERO,
            volume: 0.8,
            unmuted_volume: 0.8,
            looping: false,
            error: None,
            warning: None,
            focus: cx.focus_handle(),
            seek_bounds: Rc::new(Cell::new(Bounds::default())),
            volume_bounds: Rc::new(Cell::new(Bounds::default())),
            controls: true,
            _release,
        }
    }

    /// Whether a video or audio preview shows its controls bar.
    pub fn set_controls(&mut self, controls: bool, cx: &mut Context<Self>) {
        self.controls = controls;
        cx.notify();
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.worker = None;
        self.source = None;
        self.image = None;
        self.smaller.clear();
        self.images.borrow_mut().clear(window);
        self.loading = false;
        self.playing = false;
        self.ended = false;
        self.position = Duration::ZERO;
        self.duration = None;
        self.error = None;
        self.warning = None;
        cx.notify();
    }

    pub fn begin_loading(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear(window, cx);
        self.loading = true;
    }

    pub fn load(&mut self, source: PreviewSource, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_loading(window, cx);
        self.duration = source.duration;
        match Worker::start(source.clone(), self.volume, self.looping) {
            Ok(worker) => {
                self.worker = Some(worker);
                self.source = Some(source);
            }
            Err(error) => self.fail(format!("{error:#}"), cx),
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub fn fail(&mut self, error: String, cx: &mut Context<Self>) {
        self.worker = None;
        self.loading = false;
        self.playing = false;
        self.error = Some(error);
        cx.notify();
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.error.is_some() {
            return;
        }
        if let Some(worker) = &self.worker {
            if self.ended {
                worker.send(Command::Seek(Duration::ZERO));
                self.position = Duration::ZERO;
                self.ended = false;
            }
            self.playing = !self.playing;
            worker.send(Command::Play(self.playing));
            cx.notify();
        }
    }

    fn seek(&mut self, to: Duration, cx: &mut Context<Self>) {
        if self.loading || self.error.is_some() {
            return;
        }
        let to = self
            .duration
            .map(|d| to.min(d.saturating_sub(Duration::from_millis(100))))
            .unwrap_or(to);
        if let Some(worker) = &self.worker {
            worker.send(Command::Seek(to));
            self.position = to;
            self.ended = false;
            // A paused seek still needs polling until its preview frame arrives.
            self.loading = true;
            cx.notify();
        }
    }

    /// The playback level, 0 to 1. Also what the volume control shows.
    pub fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        self.volume = volume.clamp(0., 1.);
        if self.volume > 0. {
            self.unmuted_volume = self.volume;
        }
        if let Some(worker) = &self.worker {
            worker.send(Command::Volume(self.volume));
        }
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self
            .source
            .as_ref()
            .is_some_and(|source| source.kind != MediaKind::Image)
        {
            return;
        }
        if event.keystroke.modifiers.control || event.keystroke.modifiers.platform {
            return;
        }
        match event.keystroke.key.as_str() {
            "space" => self.toggle(cx),
            "left" => self.seek(self.position.saturating_sub(Duration::from_secs(5)), cx),
            "right" => self.seek(self.position + Duration::from_secs(5), cx),
            "home" => self.seek(Duration::ZERO, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut events = Vec::new();
        let mut disconnected = false;
        if let Some(worker) = &self.worker {
            loop {
                match worker.events.try_recv() {
                    Ok(event) => events.push(event),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        for event in events {
            match event {
                Event::Ready { duration } => {
                    self.duration = duration;
                    if self
                        .source
                        .as_ref()
                        .is_some_and(|s| s.kind == MediaKind::Audio)
                    {
                        self.loading = false;
                    }
                }
                Event::Frame {
                    image,
                    smaller,
                    position,
                } => {
                    self.image = Some(image);
                    self.smaller = smaller;
                    self.position = position;
                    self.loading = false;
                }
                Event::Position(position) => {
                    self.position = position;
                    self.loading = false;
                }
                Event::Ended => {
                    self.playing = false;
                    self.ended = true;
                    // A frame's position is when it starts showing, so the last one reads a frame
                    // short of the end; at the end, the bar should be full.
                    if let Some(duration) = self.duration {
                        self.position = duration;
                    }
                }
                Event::Error(error) => self.fail(error, cx),
                Event::Warning(warning) => self.warning = Some(warning),
            }
        }
        if disconnected && self.worker.is_some() && (self.loading || self.playing) {
            self.fail("The preview worker stopped unexpectedly".into(), cx);
        }
    }

    fn controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.worker.is_some() && !self.loading && self.error.is_none();
        let playing = self.playing;
        let looping = self.looping;
        let volume = self.volume;
        let bounds = self.seek_bounds.clone();
        let fraction = self
            .duration
            .map(|d| self.position.as_secs_f32() / d.as_secs_f32().max(0.001))
            .unwrap_or(0.)
            .clamp(0., 1.);
        // Narrow (a portrait clip, the inspector): drop the volume bar and the duration rather
        // than wrap, so the bar keeps its height (`CONTROLS_HEIGHT`). Measured off the seek bar,
        // which spans the controls, as of the last frame.
        let width = f32::from(self.seek_bounds.get().size.width);
        let compact = width > 0. && width < 340.;
        let failed = self.error.clone();
        let time = if failed.is_some() {
            "Can't play this file".to_string()
        } else if compact {
            clock(self.position)
        } else {
            format!(
                "{} / {}",
                clock(self.position),
                self.duration.map(clock).unwrap_or_else(|| "--:--".into())
            )
        };
        let volume_bounds = self.volume_bounds.clone();
        let set_volume_at = |this: &mut Self, x: Pixels, cx: &mut Context<Self>| {
            let b = this.volume_bounds.get();
            let fraction = ((x - b.origin.x) / b.size.width.max(px(1.))).clamp(0., 1.);
            this.set_volume(fraction, cx);
        };

        div()
            .flex_none()
            .h(px(CONTROLS_HEIGHT))
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .py_2()
            .border_t_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .id("preview-seek")
                    .flex_none()
                    .relative()
                    .h(px(12.))
                    .w_full()
                    .cursor_pointer()
                    .child(
                        canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(4.))
                            .h(px(4.))
                            .w_full()
                            .rounded_full()
                            .bg(c(theme::BORDER_STRONG)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(4.))
                            .h(px(4.))
                            .w(relative(fraction))
                            .rounded_full()
                            .bg(c(theme::ACCENT)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui_kit::MouseDownEvent, _, cx| {
                            if this.loading {
                                return;
                            }
                            if let Some(duration) = this.duration {
                                let b = this.seek_bounds.get();
                                let fraction =
                                    ((event.position.x - b.origin.x) / b.size.width).clamp(0., 1.);
                                this.seek(duration.mul_f32(fraction), cx);
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        control(
                            "preview-play",
                            if playing {
                                IconName::Pause
                            } else {
                                IconName::Play
                            },
                            if playing { "Pause" } else { "Play" },
                            enabled,
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle(cx))),
                    )
                    .child(
                        control(
                            "preview-restart",
                            IconName::RotateCcw,
                            "Restart",
                            enabled,
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.seek(Duration::ZERO, cx))),
                    )
                    .child(
                        control(
                            "preview-loop",
                            IconName::Repeat,
                            // Not "Loop: on/off": a tooltip already open keeps its text, so it
                            // read as the state before the click. The button's look shows it.
                            "Loop",
                            enabled,
                            looping,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.looping = !this.looping;
                            if let Some(worker) = &this.worker {
                                worker.send(Command::Loop(this.looping));
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        div()
                            .id("preview-time")
                            .ml_1()
                            .min_w_0()
                            .child(
                                ui::readout(time).truncate().when(failed.is_some(), |this| {
                                    this.text_color(c(theme::DANGER))
                                }),
                            )
                            .when_some(failed, |this, error| {
                                this.tooltip(move |window, cx| {
                                    Tooltip::new(error.clone()).build(window, cx)
                                })
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        control(
                            "preview-mute",
                            if volume == 0. {
                                IconName::VolumeX
                            } else {
                                IconName::Volume2
                            },
                            if volume == 0. { "Unmute" } else { "Mute" },
                            enabled,
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            let volume = if this.volume == 0. {
                                this.unmuted_volume
                            } else {
                                0.
                            };
                            this.set_volume(volume, cx);
                        })),
                    )
                    .when(!compact, |this| {
                        this.child(
                            div()
                                .id("preview-volume")
                                .flex_none()
                                .relative()
                                .w(px(64.))
                                .h(px(12.))
                                .cursor_pointer()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(format!("Volume {:.0}%", volume * 100.))
                                        .build(window, cx)
                                })
                                .child(
                                    canvas(move |b, _, _| volume_bounds.set(b), |_, _, _, _| {})
                                        .absolute()
                                        .size_full(),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .top(px(4.))
                                        .h(px(4.))
                                        .w_full()
                                        .rounded_full()
                                        .bg(c(theme::BORDER_STRONG)),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .top(px(4.))
                                        .h(px(4.))
                                        .w(relative(volume))
                                        .rounded_full()
                                        .bg(c(theme::TEXT)),
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(
                                        move |this, event: &gpui_kit::MouseDownEvent, _, cx| {
                                            set_volume_at(this, event.position.x, cx)
                                        },
                                    ),
                                )
                                .on_mouse_move(cx.listener(
                                    move |this, event: &gpui_kit::MouseMoveEvent, _, cx| {
                                        if event.pressed_button == Some(MouseButton::Left) {
                                            set_volume_at(this, event.position.x, cx)
                                        }
                                    },
                                )),
                        )
                    }),
            )
    }
}

/// A small icon button in the controls bar, its tooltip naming it. `active` lights it (loop on).
fn control(
    id: &'static str,
    name: IconName,
    tooltip: &'static str,
    enabled: bool,
    active: bool,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_MD)
        .border_1()
        .text_color(c(if active { theme::TEXT } else { theme::MUTED }))
        // On: a raised, outlined button, so it still reads as on under the pointer, where hover
        // gives any button the raised surface.
        .map(|this| {
            if active {
                this.bg(c(theme::SURFACE_RAISED))
                    .border_color(c(theme::BORDER_STRONG))
            } else {
                this.border_color(gpui_kit::transparent_black())
            }
        })
        .child(icon(name).size(px(15.)))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .when(!enabled, |this| this.opacity(0.4))
        .when(enabled, |this| {
            this.cursor_pointer().hover(|style| {
                style
                    .bg(c(theme::SURFACE_RAISED))
                    .text_color(c(theme::TEXT))
            })
        })
}

impl Render for MediaView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drain(cx);
        if self.loading || self.playing {
            if let Some(worker) = &self.worker {
                worker.poll();
            }
            window.request_animation_frame();
        }
        let timed = self
            .source
            .as_ref()
            .is_some_and(|s| s.kind != MediaKind::Image);
        let transparent = self.source.as_ref().is_some_and(|s| {
            matches!(
                s.kind,
                MediaKind::Video { transparent: true } | MediaKind::Image
            )
        });
        let content = if let Some(image) = &self.image {
            ImageElement {
                image: image.clone(),
                smaller: self.smaller.clone(),
                transparent,
                images: self.images.clone(),
            }
            .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .child(ui::hint(if self.loading {
                    "Loading preview..."
                } else if self.error.is_some() {
                    "Preview unavailable"
                } else if timed {
                    "Audio preview"
                } else {
                    "Select a file to preview"
                }))
                .into_any_element()
        };
        // Audio has no picture, so no picture area: the controls are the preview.
        let audio = self
            .source
            .as_ref()
            .is_some_and(|s| s.kind == MediaKind::Audio);
        div()
            .id("media-preview")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .flex()
            .flex_col()
            .bg(c(theme::BG))
            // Clicking the picture plays or pauses it, like a video player.
            .when(!audio, |this| {
                this.child(
                    div()
                        .id("preview-content")
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .when(timed, |this| {
                            this.on_click(cx.listener(|this, _, _, cx| this.toggle(cx)))
                        })
                        .child(content),
                )
            })
            // Audio has no room for it: its controls bar says it can't play instead.
            .children(self.error.clone().filter(|_| !audio).map(|error| {
                div()
                    .p_3()
                    .text_size(px(12.))
                    .text_color(c(theme::DANGER))
                    .child(error)
            }))
            .children(
                self.warning
                    .clone()
                    .map(|warning| div().px_3().pb_2().child(ui::hint(warning))),
            )
            .when(timed && self.controls, |this| this.child(self.controls(cx)))
    }
}

fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs_f64();
    format!("{}:{:05.2}", (seconds / 60.) as u64, seconds % 60.)
}
