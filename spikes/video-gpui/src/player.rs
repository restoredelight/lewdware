//! The spike's one view: a video, a control bar and a stats line.

use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use gpui_kit::{
    App, Bounds, Context, FocusHandle, Hsla, ImageSource, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, ObjectFit, ParentElement, Pixels, Render,
    RenderImage, StatefulInteractiveElement, Styled, StyledImage, Task, Window, canvas, div, hsla,
    img, px, rgb,
};

use crate::{
    Args, Present, Pump, Scale,
    audio::AudioOut,
    element::{VideoElement, fit, paint_checkerboard},
    stats::{FrameRecord, Stats},
    video::{NextFrame, Options, VideoDecoder, VideoFrame},
};

pub struct Player {
    args: Args,
    audio_out: Option<AudioOut>,
    decoder: Option<VideoDecoder>,
    error: Option<String>,
    current: Option<VideoFrame>,
    /// `current` has been taken from the decoder but not yet handed to gpui.
    unshown: bool,
    /// Frames replaced since the last paint; the element frees their atlas tiles.
    retired: Vec<Arc<RenderImage>>,
    awaiting_preview: bool,
    playing: bool,
    ended: bool,
    looping: Arc<AtomicBool>,
    volume: f32,
    target_size: Arc<AtomicU64>,
    focus: FocusHandle,
    seek_bounds: Rc<Cell<Bounds<Pixels>>>,
    paint_time: Rc<Cell<(Duration, u32)>>,
    stats: Stats,
    status: String,
    measuring: bool,
    measure_audio_start: Option<Duration>,
    renders_at_tick: u32,
    _tasks: Vec<Task<()>>,
}

impl Player {
    pub fn new(args: Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let audio_out = if args.no_audio {
            None
        } else {
            match AudioOut::open() {
                Ok(out) => Some(out),
                Err(err) => {
                    tracing::error!("no audio output: {err:#}");
                    None
                }
            }
        };

        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let mut tasks = Vec::new();

        if args.pump == Pump::Timer {
            // Poll the pacing logic on a short timer and only re-render when a new frame is due.
            tasks.push(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(2))
                        .await;
                    let Ok(()) = this.update(cx, |this, cx| {
                        if this.poll() {
                            cx.notify();
                        }
                    }) else {
                        return;
                    };
                }
            }));
        }

        // Once a second: the stats line, and the end of a `--measure` run.
        tasks.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let Ok(done) = this.update(cx, |this, cx| {
                    let done = this.tick();
                    cx.notify();
                    done
                }) else {
                    return;
                };
                if done {
                    let _ = cx.update(|cx| cx.quit());
                    return;
                }
            }
        }));

        let looping = Arc::new(AtomicBool::new(!args.no_loop));
        let volume = args.volume;
        let mut player = Self {
            args,
            audio_out,
            decoder: None,
            error: None,
            current: None,
            unshown: false,
            retired: Vec::new(),
            awaiting_preview: false,
            playing: false,
            ended: false,
            looping,
            volume,
            target_size: Arc::new(AtomicU64::new(0)),
            focus,
            seek_bounds: Rc::new(Cell::new(Bounds::default())),
            paint_time: Rc::new(Cell::new((Duration::ZERO, 0))),
            stats: Stats::new(),
            status: String::new(),
            measuring: false,
            measure_audio_start: None,
            renders_at_tick: 0,
            _tasks: tasks,
        };
        let autoplay = !player.args.spin;
        player.open_at(Duration::ZERO, autoplay);
        player
    }

    fn open_at(&mut self, start: Duration, play: bool) {
        // Drop the old decoder first, so its threads stop and its audio goes quiet.
        self.decoder = None;
        let opts = Options {
            source: self.args.source.clone(),
            packed_alpha: self.args.transparent,
            volume: self.volume,
            start,
            threads: self.args.threads,
            target_size: if self.args.scale == Scale::Ffmpeg {
                self.target_size.clone()
            } else {
                Arc::new(AtomicU64::new(0))
            },
        };
        match VideoDecoder::new(opts, self.looping.clone(), self.audio_out.as_ref()) {
            Ok(decoder) => {
                self.decoder = Some(decoder);
                self.awaiting_preview = true;
                self.playing = play;
                self.ended = false;
            }
            Err(err) => self.error = Some(format!("{err:#}")),
        }
    }

    fn adopt(&mut self, frame: VideoFrame) {
        if self.unshown {
            self.stats.superseded += 1;
        }
        if let Some(old) = self.current.take() {
            self.retired.push(old.image);
        }
        self.current = Some(frame);
        self.unshown = true;
    }

    /// Advance playback. Returns whether there's something new to draw.
    fn poll(&mut self) -> bool {
        let Some(decoder) = &mut self.decoder else {
            return false;
        };
        if self.awaiting_preview {
            let Some(frame) = decoder.take_preview() else {
                return false;
            };
            self.awaiting_preview = false;
            if self.playing {
                decoder.play();
            }
            self.adopt(frame);
            return true;
        }
        if !self.playing {
            return false;
        }
        match decoder.next_frame() {
            NextFrame::Ready(frame) => {
                self.adopt(frame);
                true
            }
            NextFrame::Finish => {
                self.playing = false;
                self.ended = true;
                true
            }
            NextFrame::None => false,
        }
    }

    fn toggle_play(&mut self) {
        if self.ended {
            self.open_at(Duration::ZERO, true);
            return;
        }
        self.playing = !self.playing;
        if let Some(decoder) = &mut self.decoder {
            if self.playing {
                decoder.play();
            } else {
                decoder.pause();
            }
        }
    }

    fn position(&self) -> Duration {
        self.current
            .as_ref()
            .map(|f| f.pass_pts)
            .unwrap_or_default()
    }

    fn duration(&self) -> Option<Duration> {
        self.decoder.as_ref().and_then(|d| d.duration)
    }

    fn seek(&mut self, to: Duration) {
        let to = match self.duration() {
            Some(d) => to.min(d.saturating_sub(Duration::from_millis(100))),
            None => to,
        };
        let play = self.playing || self.ended;
        self.open_at(to, play);
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0., 1.);
        if let Some(decoder) = &mut self.decoder {
            decoder.set_volume(self.volume);
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let step = Duration::from_secs(5);
        match event.keystroke.key.as_str() {
            "space" => self.toggle_play(),
            "left" => self.seek(self.position().saturating_sub(step)),
            "right" => self.seek(self.position() + step),
            "home" => self.seek(Duration::ZERO),
            "up" => self.set_volume(self.volume + 0.1),
            "down" => self.set_volume(self.volume - 0.1),
            "l" => {
                let on = !self.looping.load(Ordering::Relaxed);
                if let Some(decoder) = &self.decoder {
                    decoder.set_loop(on);
                } else {
                    self.looping.store(on, Ordering::Relaxed);
                }
            }
            "q" | "escape" => cx.quit(),
            _ => return,
        }
        cx.notify();
    }

    /// The once-a-second stats line. Returns true when a `--measure` run is over.
    fn tick(&mut self) -> bool {
        let elapsed = self.stats.started.elapsed();
        let (paint_total, paints) = self.paint_time.get();
        self.stats.paint_time = paint_total;
        self.stats.paints = paints;
        let (cpu, rss, gpu) = self.stats.tick();

        // Warm-up over: start measuring from a clean slate.
        // Never skip the GPU guard when the warm-up happens to end on this tick.
        if self.args.measure.is_some()
            && !self.measuring
            && elapsed >= self.args.warmup
            && gpu <= self.args.gpu_limit
        {
            self.measuring = true;
            self.stats.reset();
            self.paint_time.set((Duration::ZERO, 0));
            self.measure_audio_start = self.decoder.as_ref().and_then(|d| d.reference_clock());
            self.renders_at_tick = 0;
            return false;
        }

        let rps = self.stats.renders.saturating_sub(self.renders_at_tick);
        self.renders_at_tick = self.stats.renders;
        let Some(decoder) = &self.decoder else {
            return false;
        };
        let converted = decoder.stats.converted.load(Ordering::Relaxed).max(1);
        let convert_ms =
            decoder.stats.convert_us.load(Ordering::Relaxed) as f64 / converted as f64 / 1000.;
        let drift_ms = match (decoder.reference_clock(), &self.current) {
            (Some(clock), Some(frame)) => (clock.as_secs_f64() - frame.pts.as_secs_f64()) * 1000.,
            _ => 0.,
        };
        let size = self
            .current
            .as_ref()
            .map(|f| {
                let s = f.image.size(0);
                format!("{}x{}", s.width.0, s.height.0)
            })
            .unwrap_or_default();

        self.status = format!(
            "renders/s {rps}  cpu {cpu:.0}%  rss {rss:.0}MB  gpu {gpu:.0}MB  shown {}  gaps {}  late-skips {}  convert {convert_ms:.1}ms  drift {drift_ms:+.0}ms  frame {size}",
            self.stats.frames.len(),
            self.stats.gaps(),
            decoder.skipped_late,
        );
        tracing::info!("{}", self.status);

        if gpu > self.args.gpu_limit {
            tracing::warn!("GPU memory {gpu:.0}MB passed --gpu-limit; stopping");
            self.print_summary(drift_ms, convert_ms, size, true);
            return true;
        }

        if let Some(run) = self.args.measure
            && self.measuring
            && self.stats.started.elapsed() >= run
        {
            self.print_summary(drift_ms, convert_ms, size, false);
            return true;
        }
        false
    }

    fn print_summary(&self, drift_ms: f64, convert_ms: f64, size: String, aborted: bool) {
        let decoder = self.decoder.as_ref().expect("measured with a decoder");
        let wall = self.stats.started.elapsed().as_secs_f64();
        let nominal = decoder.nominal_frame;
        let expected = (wall / nominal.as_secs_f64()).round() as u64;
        let (_, p50, p95, max) = self.stats.lateness_percentiles();
        let late = self
            .stats
            .frames
            .iter()
            .filter(|f| f.lateness_ms > nominal.as_secs_f64() * 1000.)
            .count();
        let audio_rate = match (self.measure_audio_start, decoder.reference_clock()) {
            (Some(a), Some(b)) => (b.as_secs_f64() - a.as_secs_f64()) / wall,
            _ => 0.,
        };
        let render_ms =
            self.stats.render_time.as_secs_f64() * 1000. / self.stats.renders.max(1) as f64;
        let paint_ms =
            self.stats.paint_time.as_secs_f64() * 1000. / self.stats.paints.max(1) as f64;
        let decoded = decoder.stats.converted.load(Ordering::Relaxed).max(1);
        let decode_ms =
            decoder.stats.decode_us.load(Ordering::Relaxed) as f64 / decoded as f64 / 1000.;

        println!(
            "SUMMARY{} clip={} present={:?} scale={:?} pump={:?} frame={size} secs={wall:.1} \
             cpu={:.1}% rss_start={:.0} rss_end={:.0} rss_max={:.0} gpu_start={:.0} gpu_end={:.0} gpu_max={:.0} \
             shown={} expected={expected} gaps={} late_skips={} superseded={} late_gt_1frame={late} \
             lateness_ms_p50={p50:.1} p95={p95:.1} max={max:.1} drift_end_ms={drift_ms:+.1} audio_rate={audio_rate:.4} \
             gpu_growth_mb_s={:.1} renders={} render_ms={render_ms:.2} paint_ms={paint_ms:.2} decode_ms={decode_ms:.2} convert_ms={convert_ms:.2} audio={}",
            if aborted { " ABORTED(gpu-limit)" } else { "" },
            self.args
                .source
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            self.args.present,
            self.args.scale,
            self.args.pump,
            self.stats.cpu_pct_total(),
            self.stats.rss_start,
            crate::stats::rss_mb(),
            self.stats.rss_max,
            self.stats.gpu_start,
            crate::stats::gpu_mb(),
            self.stats.gpu_max,
            self.stats.frames.len(),
            self.stats.gaps(),
            decoder.skipped_late,
            self.stats.superseded,
            (crate::stats::gpu_mb() - self.stats.gpu_start) / wall,
            self.stats.renders,
            decoder.has_audio(),
        );
    }

    fn video(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(decoder) = &self.decoder else {
            return div().into_any_element();
        };
        let native = (decoder.native_width, decoder.native_height);
        let image = self.current.as_ref().map(|f| f.image.clone());

        match self.args.present {
            Present::Element => VideoElement {
                image,
                native,
                checkerboard: self.args.transparent,
                retire: std::mem::take(&mut self.retired),
                target_size: self.target_size.clone(),
                paint_time: self.paint_time.clone(),
            }
            .into_any_element(),
            Present::Naive | Present::ImgDrop => {
                if self.args.present == Present::ImgDrop {
                    for old in self.retired.drain(..) {
                        cx.drop_image(old, Some(window));
                    }
                } else {
                    // Naive: never freed. Keep nothing alive here either, so any growth seen is
                    // gpui's own atlas.
                    self.retired.clear();
                }
                let transparent = self.args.transparent;
                let mut container = div().size_full().relative();
                if transparent {
                    container = container.child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                paint_checkerboard(window, fit(bounds, native))
                            },
                        )
                        .absolute()
                        .size_full(),
                    );
                }
                if let Some(image) = image {
                    container = container.child(
                        img(ImageSource::Render(image))
                            .absolute()
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    );
                }
                container.into_any_element()
            }
        }
    }

    fn controls(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let text: Hsla = hsla(0., 0., 0.92, 1.);
        let muted: Hsla = hsla(0., 0., 0.6, 1.);
        let accent = rgb(0xd62828);

        let position = self.position();
        let duration = self.duration();
        let fraction = duration
            .map(|d| (position.as_secs_f32() / d.as_secs_f32().max(0.001)).clamp(0., 1.))
            .unwrap_or(0.);
        let looping = self.looping.load(Ordering::Relaxed);

        let button = |id: &'static str, label: String| {
            div()
                .id(id)
                .px_3()
                .py_1()
                .border_1()
                .border_color(muted)
                .text_color(text)
                .cursor_pointer()
                .child(label)
        };

        let seek_bounds = self.seek_bounds.clone();

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(hsla(0., 0., 0.08, 1.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        button(
                            "play",
                            if self.playing {
                                "Pause".into()
                            } else {
                                "Play".into()
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_play();
                            cx.notify();
                        })),
                    )
                    .child(div().text_color(text).child(format!(
                        "{} / {}",
                        clock(position),
                        duration.map(clock).unwrap_or_else(|| "--".into())
                    )))
                    .child(
                        // Seek bar: click anywhere to jump there.
                        div()
                            .id("seek")
                            .flex_1()
                            .h(px(10.))
                            .relative()
                            .bg(hsla(0., 0., 0.25, 1.))
                            .cursor_pointer()
                            .child(
                                canvas(
                                    move |bounds, _, _| seek_bounds.set(bounds),
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .h_full()
                                    .w(gpui_kit::relative(fraction))
                                    .bg(accent),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                    let bounds = this.seek_bounds.get();
                                    let Some(duration) = this.duration() else {
                                        return;
                                    };
                                    let x =
                                        (event.position.x - bounds.origin.x) / bounds.size.width;
                                    this.seek(duration.mul_f32(x.clamp(0., 1.)));
                                    cx.notify();
                                }),
                            ),
                    )
                    .child(
                        button(
                            "loop",
                            format!("Loop: {}", if looping { "on" } else { "off" }),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            let on = !this.looping.load(Ordering::Relaxed);
                            this.looping.store(on, Ordering::Relaxed);
                            cx.notify();
                        })),
                    )
                    .child(button("vol-down", "-".into()).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.set_volume(this.volume - 0.1);
                            cx.notify();
                        },
                    )))
                    .child(
                        div()
                            .text_color(text)
                            .child(format!("Vol {:.0}%", self.volume * 100.)),
                    )
                    .child(
                        button("vol-up", "+".into()).on_click(cx.listener(|this, _, _, cx| {
                            this.set_volume(this.volume + 0.1);
                            cx.notify();
                        })),
                    ),
            )
            .child(div().text_xs().text_color(muted).child(format!(
                "{:?} / scale {:?} / pump {:?}{}   {}",
                self.args.present,
                self.args.scale,
                self.args.pump,
                if self.ended { " / ended" } else { "" },
                self.status
            )))
    }
}

impl Render for Player {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = Instant::now();

        if self.args.pump == Pump::Vsync {
            // Poll once per frame, as the engine does on each redraw, and keep frames coming
            // while there's anything to wait for.
            self.poll();
            if self.playing || self.awaiting_preview || self.args.spin {
                window.request_animation_frame();
            }
        }

        if self.unshown {
            self.unshown = false;
            if let (Some(frame), Some(decoder)) = (&self.current, &self.decoder)
                && let Some(clock) = decoder.reference_clock()
            {
                self.stats.frames.push(FrameRecord {
                    sequence: frame.sequence,
                    lateness_ms: (clock.as_secs_f64() - frame.pts.as_secs_f64()) * 1000.,
                });
            }
        }

        let video = self.video(window, cx);
        let error = self.error.clone();
        let root = div()
            .id("root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .flex_col()
            .bg(hsla(0., 0., 0.02, 1.))
            .text_sm()
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(video)
                    .children(error.map(|e| div().p_4().text_color(rgb(0xff6060)).child(e))),
            )
            .child(self.controls(cx));

        self.stats.render_time += started.elapsed();
        self.stats.renders += 1;
        root
    }
}

fn clock(d: Duration) -> String {
    let s = d.as_secs_f64();
    format!("{}:{:05.2}", (s / 60.) as u64, s % 60.)
}

// Silence an unused-import warning on platforms where App isn't otherwise named.
#[allow(dead_code)]
fn _app(_: &App) {}
