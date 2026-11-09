use crate::{
    MediaKind, PreviewSource,
    element::{ImageElement, Images},
    worker::{Command, Event, Worker},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    Bounds, Context, FocusHandle, KeyDownEvent, MouseButton, Pixels, RenderImage, Subscription,
    Window, canvas, div, px, relative,
};
use shared_gpui::{
    theme::{self, c},
    ui::{self, Button, Size, Variant},
};
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
            _release,
        }
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.worker = None;
        self.source = None;
        self.image = None;
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

    fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
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
                Event::Frame { image, position } => {
                    self.image = Some(image);
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
        let button = |id, label| {
            Button::new(id, label)
                .variant(Variant::Quiet)
                .size(Size::Compact)
                .disabled(!enabled)
        };
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        button("preview-play", if playing { "Pause" } else { "Play" })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle(cx))),
                    )
                    .child(
                        button("preview-restart", "Restart")
                            .on_click(cx.listener(|this, _, _, cx| this.seek(Duration::ZERO, cx))),
                    )
                    .child(div().flex_1())
                    .child(ui::readout(format!(
                        "{} / {}",
                        clock(self.position),
                        self.duration.map(clock).unwrap_or_else(|| "--:--".into())
                    ))),
            )
            .child(
                div()
                    .id("preview-seek")
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
                            .bg(c(theme::BORDER_STRONG)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(4.))
                            .h(px(4.))
                            .w(relative(fraction))
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
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        button(
                            "preview-loop",
                            if looping { "Loop: on" } else { "Loop: off" },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.looping = !this.looping;
                            if let Some(worker) = &this.worker {
                                worker.send(Command::Loop(this.looping));
                            }
                            cx.notify();
                        })),
                    )
                    .child(div().flex_1())
                    .child(
                        button("preview-mute", if volume == 0. { "Unmute" } else { "Mute" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                let volume = if this.volume == 0. {
                                    this.unmuted_volume
                                } else {
                                    0.
                                };
                                this.set_volume(volume, cx);
                            })),
                    )
                    .child(button("preview-quieter", "-").on_click(
                        cx.listener(|this, _, _, cx| this.set_volume(this.volume - 0.1, cx)),
                    ))
                    .child(ui::readout(format!("Volume {:.0}%", volume * 100.)))
                    .child(button("preview-louder", "+").on_click(
                        cx.listener(|this, _, _, cx| this.set_volume(this.volume + 0.1, cx)),
                    )),
            )
    }
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
        div()
            .id("media-preview")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .flex()
            .flex_col()
            .bg(c(theme::BG))
            .child(div().flex_1().min_h_0().overflow_hidden().child(content))
            .children(self.error.clone().map(|error| {
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
            .when(timed, |this| this.child(self.controls(cx)))
    }
}

fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs_f64();
    format!("{}:{:05.2}", (seconds / 60.) as u64, seconds % 60.)
}
