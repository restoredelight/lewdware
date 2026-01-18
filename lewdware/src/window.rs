//! Handles the different popup windows. We draw to image windows using `softbuffer` (which works
//! on the CPU), and render videos using `pixels` (which works on the GPU, using `wgpu`). Prompt
//! windows are also drawn using `wgpu`. We do this because having too many GPU rendered windows
//! can exhaust the device's VRAM, causing a crash. However, we still want to use the GPU to render
//! videos for smooth playback.

use std::{
    cell::{LazyCell, OnceCell},
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use egui::{RichText, TextEdit};
use pixels::{Pixels, PixelsBuilder, SurfaceTexture};
use tiny_skia::{
    BlendMode, Color, IntSize, Paint, Path, PathBuilder, Pixmap, PixmapMut, PixmapPaint, Rect,
    Stroke, Transform,
};
use tokio::sync::mpsc;
use winit::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, PhysicalUnit},
    event::WindowEvent,
    window::Window as WinitWindow,
};

use crate::{
    egui::{EguiCPUWindow, WgpuState},
    error::{LewdwareError, MonitorError},
    header::{HEADER_HEIGHT, Header},
    lua::{self, ChoiceWindowOption, Coord, Easing, MoveOpts},
    media::{FileOrPath, ImageData, Video},
    utils::resolve_coord,
    video::{NextFrame, VideoDecoder, VideoFrame, copy_frame_pixmap},
};

pub enum Window<'a> {
    Image(ImageWindow),
    Video(VideoWindow<'a>),
    Prompt(PromptWindow),
    Choice(ChoiceWindow),
}

impl Window<'_> {
    pub fn inner_window(&self) -> &InnerWindow {
        match self {
            Self::Image(image_window) => &image_window.inner_window,
            Self::Video(video_window) => &video_window.inner_window,
            Self::Prompt(prompt_window) => &prompt_window.inner_window,
            Self::Choice(choice_window) => &choice_window.inner_window,
        }
    }

    pub fn inner_window_mut(&mut self) -> &mut InnerWindow {
        match self {
            Self::Image(image_window) => &mut image_window.inner_window,
            Self::Video(video_window) => &mut video_window.inner_window,
            Self::Prompt(prompt_window) => &mut prompt_window.inner_window,
            Self::Choice(choice_window) => &mut choice_window.inner_window,
        }
    }

    pub fn window_type(&self) -> &'static str {
        match self {
            Self::Image(image_window) => "image",
            Self::Video(video_window) => "video",
            Self::Prompt(prompt_window) => "prompt",
            Self::Choice(choice_window) => "choice",
        }
    }
}

/// A window displaying an image. Image windows are rendered using softbuffer.
pub struct ImageWindow {
    inner_window: InnerWindow,
    image: Option<ImageData>,
    _context: softbuffer::Context<Arc<WinitWindow>>,
    surface: softbuffer::Surface<Arc<WinitWindow>, Arc<WinitWindow>>,
    pixmap: Pixmap,
    border: bool,
    header: Option<Header>,
    inner_size: PhysicalSize<u32>,
    outer_size: PhysicalSize<u32>,
}

struct MovementState {
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    last_moved: Instant,
}

#[derive(Clone, Copy)]
enum Direction {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ImageWindow {
    /// Create a new image window.
    ///
    /// * `close_button`: Whether to display a close button on the window.
    /// * `moving`: Whether to move the window around the screen.
    pub fn new(
        window: WinitWindow,
        initial_position: LogicalPosition<u32>,
        image: ImageData,
        lua_event_tx: mpsc::UnboundedSender<lua::Event>,
        header: bool,
        border: bool,
    ) -> Result<Self> {
        let window = Arc::new(window);

        let inner_window = InnerWindow::new(window.clone(), initial_position, lua_event_tx);

        let context = softbuffer::Context::new(window.clone()).map_err(|err| anyhow!("{}", err))?;
        let surface =
            softbuffer::Surface::new(&context, window.clone()).map_err(|err| anyhow!("{}", err))?;

        let (outer_size, inner_size) = calculate_size(window.clone(), border);

        let scale_factor = window.scale_factor();

        let header = header.then(|| Header::new(window.clone(), inner_size.clone(), scale_factor));

        let mut pixmap = Pixmap::new(outer_size.width, outer_size.height).unwrap();

        if border {
            draw_border(&mut pixmap.as_mut(), &outer_size);
        }

        Ok(Self {
            inner_window,
            image: Some(image),
            border: true,
            _context: context,
            header,
            surface,
            pixmap,
            inner_size,
            outer_size,
        })
    }

    pub fn draw(&mut self) -> Result<()> {
        self.surface
            .resize(
                NonZeroU32::new(self.outer_size.width).context("Window has 0 width")?,
                NonZeroU32::new(self.outer_size.height).context("Window has 0 height")?,
            )
            .map_err(|err| anyhow!("{}", err))?;

        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|err| anyhow!("{}", err))?;

        let scale_factor = self.inner_window.window.scale_factor();

        let border_offset = if self.border {
            PhysicalUnit::from_logical::<_, u32>(1, scale_factor).0
        } else {
            0
        };

        if let Some(header) = &mut self.header {
            let header_pixmap = header.draw();

            copy_pixmap(
                &mut self.pixmap.as_mut(),
                &header_pixmap,
                border_offset,
                border_offset,
            );
        }

        if let Some(image) = self.image.take() {
            let width = image.width();
            let height = image.height();

            let image_pixmap =
                Pixmap::from_vec(image.into_vec(), IntSize::from_wh(width, height).unwrap())
                    .unwrap();

            let physical_header_height: u32 = if self.header.is_some() {
                PhysicalUnit::from_logical::<_, u32>(HEADER_HEIGHT, scale_factor).0
            } else {
                0
            };

            copy_pixmap(
                &mut self.pixmap.as_mut(),
                &image_pixmap,
                border_offset,
                physical_header_height + border_offset,
            );
        }

        render_pixmap_softbuffer(&mut buffer, &self.pixmap);

        buffer.present().map_err(|err| anyhow!("{}", err))?;

        Ok(())
    }

    pub fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if let Some(header) = &mut self.header {
            header.handle_cursor_moved(position);
        }
    }

    pub fn handle_cursor_left(&mut self) {
        if let Some(header) = &mut self.header {
            header.handle_cursor_left();
        }
    }

    pub fn handle_mouse_down(&mut self) {
        if let Some(header) = &mut self.header {
            header.handle_mouse_down();
        }
    }

    pub fn handle_mouse_up(&mut self) -> bool {
        if let Some(header) = &mut self.header {
            header.handle_mouse_up()
        } else {
            false
        }
    }
}

fn draw_border(pixmap: &mut PixmapMut, outer_size: &PhysicalSize<u32>) {
    let border = PathBuilder::from_rect(
        Rect::from_xywh(
            1.0,
            1.0,
            (outer_size.width + 2) as f32,
            (outer_size.height + 2) as f32,
        )
        .unwrap(),
    );
    let mut paint = Paint::default();
    paint.set_color(Color::BLACK);
    pixmap.stroke_path(
        &border,
        &paint,
        &Stroke::default(),
        Transform::default(),
        None,
    );
}

fn calculate_size(
    window: Arc<WinitWindow>,
    border: bool,
) -> (PhysicalSize<u32>, PhysicalSize<u32>) {
    let outer_size = window.inner_size();

    let inner_size = if border {
        let logical_size = outer_size.to_logical::<u32>(window.scale_factor());
        LogicalSize::new(logical_size.width - 2, logical_size.height - 2)
            .to_physical(window.scale_factor())
    } else {
        outer_size.clone()
    };

    (outer_size, inner_size)
}

/// A video popup, rendered using pixels.
pub struct VideoWindow<'a> {
    inner_window: InnerWindow,
    buffer: Buffer<'a>,
    decoder: VideoDecoder,
    last_frame_time: Instant,
    duration: Option<Duration>,
    border: bool,
    header: Option<Header>,
    wgpu_error: Arc<AtomicBool>,
    loop_video: bool,
    inner_size: PhysicalSize<u32>,
    outer_size: PhysicalSize<u32>,
    paused: bool,
}

enum Buffer<'a> {
    Pixels(Pixels<'a>),
    Softbuffer {
        _context: softbuffer::Context<Arc<WinitWindow>>,
        surface: softbuffer::Surface<Arc<WinitWindow>, Arc<WinitWindow>>,
        pixmap: Pixmap,
    },
}

fn make_softbuffer(
    window: Arc<WinitWindow>,
) -> Result<(
    softbuffer::Context<Arc<WinitWindow>>,
    softbuffer::Surface<Arc<WinitWindow>, Arc<WinitWindow>>,
)> {
    let context = softbuffer::Context::new(window.clone()).map_err(|err| anyhow!("{}", err))?;
    let surface =
        softbuffer::Surface::new(&context, window.clone()).map_err(|err| anyhow!("{}", err))?;

    Ok((context, surface))
}

impl VideoWindow<'_> {
    /// Create a new video popup.
    ///
    /// * `close_button`: Whether to display a close button on the window.
    /// * `play_audio`: Whether to play the video's audio.
    pub fn new(
        wgpu_state: &WgpuState,
        window: WinitWindow,
        video: FileOrPath,
        width: u32,
        height: u32,
        play_audio: bool,
        loop_video: bool,
        initial_position: LogicalPosition<u32>,
        header: bool,
        border: bool,
        lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    ) -> anyhow::Result<Self> {
        let window = Arc::new(window);

        let (outer_size, inner_size) = calculate_size(window.clone(), border);

        let scale_factor = window.scale_factor();

        let inner_window = InnerWindow::new(window.clone(), initial_position, lua_event_tx);

        let video_size = LogicalSize::new(width, height).to_physical(scale_factor);

        let decoder = VideoDecoder::new(
            video,
            video_size.width,
            video_size.height,
            play_audio,
            loop_video,
        )?;

        let buffer = if wgpu_state.error.load(Ordering::Acquire) {
            let (context, surface) = make_softbuffer(window.clone())?;

            let mut pixmap = Pixmap::new(outer_size.width, outer_size.height).unwrap();

            if border {
                draw_border(&mut pixmap.as_mut(), &outer_size);
            }

            Buffer::Softbuffer {
                _context: context,
                surface,
                pixmap,
            }
        } else {
            let surface_texture =
                SurfaceTexture::new(outer_size.width, outer_size.height, window.clone());

            let mut pixels =
                PixelsBuilder::new(outer_size.width, outer_size.height, surface_texture)
                    .build_with_instance(
                        &wgpu_state.instance,
                        &wgpu_state.adapter,
                        &wgpu_state.device,
                        &wgpu_state.queue,
                    )?;

            let mut pixmap =
                PixmapMut::from_bytes(pixels.frame_mut(), outer_size.width, outer_size.height)
                    .unwrap();

            if border {
                draw_border(&mut pixmap, &outer_size);
            }

            Buffer::Pixels(pixels)
        };

        let header = header.then(|| Header::new(window.clone(), inner_size.clone(), scale_factor));

        Ok(Self {
            inner_window,
            buffer,
            decoder,
            last_frame_time: Instant::now(),
            duration: None,
            wgpu_error: wgpu_state.error.clone(),
            loop_video,
            border,
            header,
            inner_size,
            outer_size,
            paused: false,
        })
    }

    pub fn update(&mut self) -> anyhow::Result<bool> {
        match &self.buffer {
            Buffer::Pixels(pixels) => {
                if self.wgpu_error.load(Ordering::Acquire) {
                    let (context, surface) = make_softbuffer(self.inner_window.window.clone())?;

                    let mut pixmap =
                        Pixmap::new(self.outer_size.width, self.outer_size.height).unwrap();

                    if self.border {
                        draw_border(&mut pixmap.as_mut(), &self.outer_size);
                    }

                    self.buffer = Buffer::Softbuffer {
                        _context: context,
                        surface,
                        pixmap,
                    };

                    return self.update();
                }
            }
            _ => {}
        }

        let scale_factor = self.inner_window.window.scale_factor();

        let border_offset = if self.border {
            PhysicalUnit::from_logical::<_, u32>(1, scale_factor).0
        } else {
            0
        };

        if let Some(header_pixmap) = self.header.as_ref().map(|header| header.draw()) {
            let mut pixmap = self.get_pixmap();

            copy_pixmap(&mut pixmap, &header_pixmap, border_offset, border_offset);
        }

        let (close, frame) = self.get_next_frame()?;

        if close {
            return Ok(true);
        }

        if let Some(frame) = frame {
            let scale_factor = self.inner_window.window.scale_factor();

            let border_offset = if self.border {
                PhysicalUnit::from_logical::<_, u32>(1, scale_factor).0
            } else {
                0
            };

            let physical_header_height: u32 = if self.header.is_some() {
                PhysicalUnit::from_logical::<_, u32>(HEADER_HEIGHT, scale_factor).0
            } else {
                0
            };

            copy_frame_pixmap(
                &frame.frame,
                &mut self.get_pixmap(),
                physical_header_height + border_offset,
                border_offset,
            )?;
        }

        self.present()?;

        Ok(false)
    }

    fn get_pixmap(&'_ mut self) -> PixmapMut<'_> {
        match &mut self.buffer {
            Buffer::Pixels(pixels) => PixmapMut::from_bytes(
                pixels.frame_mut(),
                self.outer_size.width,
                self.outer_size.height,
            )
            .unwrap(),
            Buffer::Softbuffer {
                _context,
                surface: _,
                pixmap,
            } => pixmap.as_mut(),
        }
    }

    fn present(&mut self) -> anyhow::Result<()> {
        match &mut self.buffer {
            Buffer::Pixels(pixels) => {
                pixels.render()?;
            }
            Buffer::Softbuffer {
                _context,
                surface,
                pixmap,
            } => {
                surface
                    .resize(
                        NonZeroU32::new(self.outer_size.width).unwrap(),
                        NonZeroU32::new(self.outer_size.height).unwrap(),
                    )
                    .unwrap();

                let mut buffer = surface.buffer_mut().map_err(|err| anyhow!("{err}"))?;

                render_pixmap_softbuffer(&mut buffer, &pixmap);

                buffer.present().map_err(|err| anyhow!("{err}"))?;
            }
        }

        Ok(())
    }

    fn get_next_frame(&mut self) -> anyhow::Result<(bool, Option<VideoFrame>)> {
        if self.paused
            || self
                .duration
                .is_some_and(|duration| self.last_frame_time.elapsed() < duration)
        {
            return Ok((false, None));
        }

        let frame = loop {
            match self.decoder.next_frame() {
                NextFrame::Ready(frame) => break frame,
                NextFrame::Finish => {
                    let _ = self
                        .inner_window
                        .lua_event_tx
                        .send(lua::Event::VideoFinish {
                            id: self.inner_window.window.id(),
                        });

                    if !self.loop_video {
                        return Ok((true, None));
                    }
                }
                NextFrame::None => return Ok((false, None)),
                NextFrame::Error(err) => return Err(err),
            }
        };

        self.duration = Some(frame.duration);
        self.last_frame_time = Instant::now();

        Ok((false, Some(frame)))
    }

    pub fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if let Some(header) = &mut self.header {
            header.handle_cursor_moved(position);
        }
    }

    pub fn handle_cursor_left(&mut self) {
        if let Some(header) = &mut self.header {
            header.handle_cursor_left();
        }
    }

    pub fn handle_mouse_down(&mut self) {
        if let Some(header) = &mut self.header {
            header.handle_mouse_down();
        }
    }

    pub fn handle_mouse_up(&mut self) -> bool {
        if let Some(header) = &mut self.header {
            header.handle_mouse_up()
        } else {
            false
        }
    }

    pub fn pause(&mut self) {
        self.decoder.pause();
        self.paused = true;

        if let Some(duration) = self.duration.take() {
            self.duration = Some(duration - self.last_frame_time.elapsed());
        }
    }

    pub fn play(&mut self) {
        self.paused = false;
        self.last_frame_time = Instant::now();

        self.decoder.play();
    }
}

/// A prompt window, rendered using `egui`.
pub struct PromptWindow {
    inner_window: InnerWindow,
    egui_window: EguiCPUWindow,
    title: Option<String>,
    text: Option<String>,
    placeholder: Option<String>,
    value: String,
}

impl PromptWindow {
    pub fn new(
        window: WinitWindow,
        position: LogicalPosition<u32>,
        title: Option<String>,
        text: Option<String>,
        placeholder: Option<String>,
        initial_value: Option<String>,
        lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    ) -> Result<Self> {
        let window = Arc::new(window);
        let inner_window = InnerWindow::new(window.clone(), position, lua_event_tx);

        Ok(Self {
            inner_window,
            egui_window: EguiCPUWindow::new(window)?,
            title,
            text,
            placeholder,
            value: initial_value.unwrap_or_default(),
        })
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        self.egui_window.handle_event(event);
    }

    pub fn render(&mut self) -> Result<()> {
        self.egui_window.redraw(|ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.heading("Repeat after me");
                    ui.add_space(20.0);

                    if let Some(title) = &self.title {
                        ui.label(RichText::new(title).heading());
                    }

                    if let Some(text) = &self.text {
                        ui.label(text);
                    }

                    let mut prompt = TextEdit::singleline(&mut self.value);

                    if let Some(placeholder) = &self.placeholder {
                        prompt = prompt.hint_text(placeholder);
                    };

                    let response = ui.add(prompt);
                    response.request_focus();

                    ui.add_space(ui.available_height() - 50.0);

                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("Submit")).clicked() {
                            if let Err(err) =
                                self.inner_window
                                    .lua_event_tx
                                    .send(lua::Event::PromptSubmit {
                                        id: self.inner_window.window.id(),
                                        text: self.value.clone(),
                                    })
                            {
                                eprintln!("{err}");
                            }
                        }
                    })
                })
            });
        })?;

        Ok(())
    }

    pub fn set_title(&mut self, title: Option<String>) {
        self.title = title;
        self.inner_window.window.request_redraw();
    }

    pub fn set_text(&mut self, text: Option<String>) {
        self.text = text;
        self.inner_window.window.request_redraw();
    }

    pub fn set_value(&mut self, value: Option<String>) {
        self.value = value.unwrap_or_default();
        self.inner_window.window.request_redraw();
    }
}

pub struct ChoiceWindow {
    inner_window: InnerWindow,
    egui_window: EguiCPUWindow,
    title: Option<String>,
    text: Option<String>,
    options: Vec<ChoiceWindowOption>,
}

impl ChoiceWindow {
    pub fn new(
        window: WinitWindow,
        position: LogicalPosition<u32>,
        title: Option<String>,
        text: Option<String>,
        options: Vec<ChoiceWindowOption>,
        lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    ) -> Result<Self> {
        let window = Arc::new(window);
        let inner_window = InnerWindow::new(window.clone(), position, lua_event_tx);

        Ok(Self {
            inner_window,
            egui_window: EguiCPUWindow::new(window)?,
            title,
            text,
            options,
        })
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        self.egui_window.handle_event(event);
    }

    pub fn render(&mut self) -> Result<()> {
        self.egui_window.redraw(|ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.heading("Repeat after me");
                    ui.add_space(20.0);

                    if let Some(title) = &self.title {
                        ui.label(RichText::new(title).heading());
                    }

                    if let Some(text) = &self.text {
                        ui.label(text);
                    }

                    ui.add_space(ui.available_height() - 50.0);

                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        for option in &self.options {
                            if ui.button(&option.label).clicked() {
                                let _ =
                                    self.inner_window
                                        .lua_event_tx
                                        .send(lua::Event::ChoiceSelect {
                                            id: self.inner_window.window.id(),
                                            option_id: option.id.clone(),
                                        });
                            }
                        }
                    })
                })
            });
        })?;

        Ok(())
    }

    pub fn set_title(&mut self, title: Option<String>) {
        self.title = title;
        self.inner_window.window.request_redraw();
    }

    pub fn set_text(&mut self, text: Option<String>) {
        self.text = text;
        self.inner_window.window.request_redraw();
    }

    pub fn set_options(&mut self, options: Vec<ChoiceWindowOption>) {
        self.options = options;
        self.inner_window.window.request_redraw();
    }
}

pub struct InnerWindow {
    window: Arc<WinitWindow>,
    position: LogicalPosition<u32>,
    lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    current_move: Option<Move>,
}

struct Move {
    id: u64,
    from: LogicalPosition<u32>,
    to: LogicalPosition<u32>,
    duration: Duration,
    start: Instant,
    easing: Easing,
}

impl InnerWindow {
    fn new(
        window: Arc<WinitWindow>,
        position: LogicalPosition<u32>,
        lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    ) -> Self {
        Self {
            window,
            position,
            lua_event_tx,
            current_move: None,
        }
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn start_move(&mut self, id: u64, opts: MoveOpts) -> Result<(), LewdwareError> {
        let scale_factor = self.window.scale_factor();

        let size = self.window.inner_size().to_logical(scale_factor);

        let monitor_size = self
            .window
            .current_monitor()
            .ok_or(LewdwareError::MonitorError(MonitorError::WindowMonitorNotFound))?
            .size()
            .to_logical::<u32>(scale_factor);

        let x = match opts.x {
            Some(Coord::Pixel(x)) => Some(opts.anchor.resolve(x, size.width)),
            Some(Coord::Percent { percent }) => Some(opts.anchor.resolve(
                ((percent * monitor_size.width as f64) / 100.0).round() as u32,
                size.width,
            )),
            None => None,
        };

        let y = match opts.y {
            Some(Coord::Pixel(y)) => Some(opts.anchor.resolve(y, size.height)),
            Some(Coord::Percent { percent }) => Some(opts.anchor.resolve(
                ((percent * monitor_size.height as f64) / 100.0).round() as u32,
                size.height,
            )),
            None => None,
        };

        let new_position = if opts.relative {
            LogicalPosition::new(
                self.position.x + x.unwrap_or(0),
                self.position.y + y.unwrap_or(0),
            )
        } else {
            LogicalPosition::new(x.unwrap_or(self.position.x), y.unwrap_or(self.position.y))
        };

        let move_obj = Move {
            id: id,
            from: self.position.clone(),
            to: new_position,
            duration: Duration::from_millis(opts.duration),
            start: Instant::now(),
            easing: opts.easing,
        };

        self.current_move = Some(move_obj);

        Ok(())
    }

    pub fn update_position(&mut self) {
        if let Some(current_move) = &self.current_move {
            let percent = current_move
                .start
                .elapsed()
                .div_duration_f64(current_move.duration)
                .min(1.0);

            let new_position = LogicalPosition::new(
                ((current_move.to.x - current_move.from.x) as f64 * percent).round() as u32,
                ((current_move.to.y - current_move.from.y) as f64 * percent).round() as u32,
            );

            if new_position != self.position {
                let monitor_position = self
                    .window
                    .current_monitor()
                    .map(|monitor| monitor.position().to_logical(self.window.scale_factor()))
                    .unwrap_or(LogicalPosition::new(0, 0));

                self.window.set_outer_position(LogicalPosition::new(
                    monitor_position.x + new_position.x,
                    monitor_position.y + new_position.y,
                ));

                self.position = new_position;
            }

            if percent >= 1.0 {
                if let Err(err) = self.lua_event_tx.send(lua::Event::MoveFinish {
                    id: self.window.id(),
                    move_id: current_move.id,
                }) {
                    eprintln!("{err}");
                }

                self.current_move = None;
            }
        }
    }
}

impl Drop for InnerWindow {
    fn drop(&mut self) {
        if let Err(_) = self.lua_event_tx.send(lua::Event::WindowClosed {
            id: self.window.id(),
        }) {
            eprintln!("Event receiver closed");
        }
    }
}

fn render_pixmap_softbuffer(buffer: &mut [u32], pixmap: &Pixmap) {
    let data = pixmap.data();

    for index in 0..(pixmap.width() * pixmap.height()) as usize {
        let r = data[index * 4] as u32;
        let g = data[index * 4 + 1] as u32;
        let b = data[index * 4 + 2] as u32;
        let a = data[index * 4 + 3] as u32;

        buffer[index] = (a << 24) | (r << 16) | (g << 8) | b;
    }
}

fn copy_pixmap(destination: &mut PixmapMut<'_>, source: &Pixmap, x: u32, y: u32) {
    let offset = (y * destination.width()) as usize * 4;
    let dst_width = destination.width();
    let dst_data = &mut destination.data_mut()[offset..];

    if x == 0 && dst_width == source.width() {
        dst_data[..source.data().len()].copy_from_slice(source.data());
    } else {
        let src_data = source.data();

        for (i, row) in src_data
            .chunks_exact(source.width() as usize * 4)
            .enumerate()
        {
            let index = (dst_width * i as u32 + x) as usize * 4;

            dst_data[index..index + row.len()].copy_from_slice(row);
        }
    }
}
