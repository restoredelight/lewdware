//! Handles the different popup windows. We draw to image windows using `softbuffer` (which works
//! on the CPU), and render videos using `pixels` (which works on the GPU, using `wgpu`). Prompt
//! windows are also drawn using `wgpu`. We do this because having too many GPU rendered windows
//! can exhaust the device's VRAM, causing a crash. However, we still want to use the GPU to render
//! videos for smooth playback.

use std::{
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use pixels::{Pixels, PixelsBuilder, SurfaceTexture};
use rand::{rng, seq::IndexedRandom};
use winit::{dpi::PhysicalPosition, event::WindowEvent, window::Window};

use crate::{
    buffer::{PixelsWrapper, SoftBufferWrapper, draw_close_button, is_over_close_button},
    egui::{EguiWindow, WgpuState},
    media::{Image, Video},
    video::VideoDecoder,
};

/// A window displaying an image. Image windows are rendered using softbuffer.
pub struct ImageWindow {
    pub window: Arc<Window>,
    pub created: Instant,
    pub moving: bool,
    image: Option<Image>,
    close_button: bool,
    cursor_over_button: bool,
    _context: softbuffer::Context<Arc<Window>>,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    width: u32,
    height: u32,
    moving_direction: Direction,
    last_moved: Instant,
    window_visible: bool,
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
    pub fn new(window: Window, image: Image, close_button: bool, moving: bool) -> Result<Self> {
        let window = Arc::new(window);

        let context = softbuffer::Context::new(window.clone()).map_err(|err| anyhow!("{}", err))?;
        let surface =
            softbuffer::Surface::new(&context, window.clone()).map_err(|err| anyhow!("{}", err))?;

        let width = image.width();
        let height = image.height();

        let mut rng = rng();
        let moving_direction = *[
            Direction::TopLeft,
            Direction::TopRight,
            Direction::BottomLeft,
            Direction::BottomRight,
        ]
        .choose(&mut rng)
        .unwrap();

        Ok(Self {
            window,
            image: Some(image),
            created: Instant::now(),
            close_button,
            cursor_over_button: false,
            _context: context,
            surface,
            width,
            height,
            moving,
            moving_direction,
            last_moved: Instant::now(),
            window_visible: false,
        })
    }

    pub fn draw(&mut self) -> Result<()> {
        // We only need to render the image once
        let mut buffer = if let Some(image) = self.image.take() {
            self.surface
                .resize(
                    NonZeroU32::new(image.width()).unwrap(),
                    NonZeroU32::new(image.height()).unwrap(),
                )
                .map_err(|err| anyhow!("{}", err))?;

            let mut buffer = self
                .surface
                .buffer_mut()
                .map_err(|err| anyhow!("{}", err))?;

            for (i, pixel) in image.pixels().enumerate() {
                let r = pixel[0] as u32;
                let g = pixel[1] as u32;
                let b = pixel[2] as u32;
                let a = pixel[3] as u32;

                buffer[i] = (a << 24) | (r << 16) | (g << 8) | b;
            }

            buffer
        } else {
            self.surface
                .buffer_mut()
                .map_err(|err| anyhow!("{}", err))?
        };

        // let size = self.window.inner_size();
        //
        // let font = FontArc::try_from_slice(include_bytes!("ChicagoKare-Regular.ttf")).unwrap();
        //
        // draw_text_ab_glyph_with_outline(
        //     &mut SoftBufferWrapper::new(&mut buffer, self.width as usize, self.height as usize),
        //     size.width as usize,
        //     size.height as usize,
        //     &font,
        //     20.0,
        //     10.0,
        //     20.0,
        //     "Kill yourself",
        //     0xFF000000,
        //     Some(0xFF000000),
        //     2.0,
        // );

        if self.close_button {
            draw_close_button(
                &mut SoftBufferWrapper::new(&mut buffer, self.width as usize, self.height as usize),
                self.window.scale_factor(),
                self.cursor_over_button,
            );
        }

        buffer.present().map_err(|err| anyhow!("{}", err))?;

        if !self.window_visible {
            self.window.set_visible(true);
            self.window_visible = true;
        }

        Ok(())
    }

    /// If the window is a moving window, update its position.
    pub fn update_position(&mut self) -> Result<()> {
        let window_size = self.window.inner_size();
        let monitor_size = self.window.current_monitor().map(|x| x.size()).unwrap();
        let window_position = self.window.outer_position()?;

        let delta = (self.last_moved.elapsed().as_secs_f64() * 300.0) as i32;

        match self.moving_direction {
            Direction::TopLeft => {
                if window_position.x <= 0 {
                    self.moving_direction = Direction::TopRight;
                    return self.update_position();
                } else if window_position.y <= 0 {
                    self.moving_direction = Direction::BottomLeft;
                    return self.update_position();
                }

                self.window.set_outer_position(PhysicalPosition::new(
                    window_position.x - delta,
                    window_position.y - delta,
                ));
            }
            Direction::TopRight => {
                if window_position.x + window_size.width as i32 >= monitor_size.width as i32 {
                    self.moving_direction = Direction::TopLeft;
                    return self.update_position();
                } else if window_position.y <= 0 {
                    self.moving_direction = Direction::BottomRight;
                    return self.update_position();
                }

                self.window.set_outer_position(PhysicalPosition::new(
                    window_position.x + delta,
                    window_position.y - delta,
                ));
            }
            Direction::BottomLeft => {
                if window_position.x <= 0 {
                    self.moving_direction = Direction::BottomRight;
                    return self.update_position();
                } else if window_position.y + window_size.height as i32
                    >= monitor_size.height as i32
                {
                    self.moving_direction = Direction::TopLeft;
                    return self.update_position();
                }

                self.window.set_outer_position(PhysicalPosition::new(
                    window_position.x - delta,
                    window_position.y + delta,
                ));
            }
            Direction::BottomRight => {
                if window_position.x + window_size.width as i32 >= monitor_size.width as i32 {
                    self.moving_direction = Direction::BottomLeft;
                    return self.update_position();
                } else if window_position.y + window_size.height as i32
                    >= monitor_size.height as i32
                {
                    self.moving_direction = Direction::TopRight;
                    return self.update_position();
                }

                self.window.set_outer_position(PhysicalPosition::new(
                    window_position.x + delta,
                    window_position.y + delta,
                ));
            }
        }

        if delta != 0 {
            self.last_moved = Instant::now();
        }

        Ok(())
    }

    pub fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if self.close_button {
            let was_over_button = self.cursor_over_button;
            self.cursor_over_button = is_over_close_button(
                position.x,
                position.y,
                self.width,
                self.window.scale_factor(),
            );

            if was_over_button != self.cursor_over_button {
                self.window.request_redraw();
            }
        }
    }

    pub fn handle_mouse_left_window(&mut self) {
        if self.close_button && self.cursor_over_button {
            self.cursor_over_button = false;
            self.window.request_redraw();
        }
    }

    /// Handle a click event. Returns true if the window should be closed.
    pub fn handle_click(&mut self) -> bool {
        if self.close_button {
            self.cursor_over_button
        } else {
            true
        }
    }
}

/// A video popup, rendered using pixels.
pub struct VideoWindow<'a> {
    pub window: Arc<Window>,
    pub created: Instant,
    pixels: Pixels<'a>,
    decoder: VideoDecoder,
    last_frame_time: Instant,
    duration: Option<Duration>,
    close_button: bool,
    close_button_changed: bool,
    cursor_over_button: bool,
    width: u32,
    height: u32,
    window_visible: bool,
}

impl<'a> VideoWindow<'a> {
    /// Create a new video popup.
    ///
    /// * `close_button`: Whether to display a close button on the window.
    /// * `play_audio`: Whether to play the video's audio.
    pub fn new(
        wgpu_state: &WgpuState,
        window: Window,
        video: Video,
        close_button: bool,
        play_audio: bool,
    ) -> anyhow::Result<Self> {
        let window = Arc::new(window);

        let width = video.width as u32;
        let height = video.height as u32;

        let decoder = VideoDecoder::new(video, play_audio)?;

        let surface_texture = SurfaceTexture::new(width, height, window.clone());

        let pixels = PixelsBuilder::new(width, height, surface_texture).build_with_instance(
            &wgpu_state.instance,
            &wgpu_state.adapter,
            &wgpu_state.device,
            &wgpu_state.queue,
        )?;

        Ok(Self {
            window,
            pixels,
            decoder,
            last_frame_time: Instant::now(),
            duration: None,
            created: Instant::now(),
            close_button,
            close_button_changed: false,
            cursor_over_button: false,
            width,
            height,
            window_visible: false,
        })
    }

    pub fn update(&mut self) -> anyhow::Result<()> {
        let mut render = false;

        if self
            .duration
            .is_none_or(|duration| self.last_frame_time.elapsed() >= duration)
        {
            let frame = self.decoder.next_frame()?;

            if let Some(frame) = frame {
                self.decoder
                    .copy_frame(&frame.frame, self.pixels.frame_mut());
                self.duration = Some(frame.duration);
                self.last_frame_time = Instant::now();
                render = true;
            }
        }

        if self.close_button && (render || self.close_button_changed) {
            draw_close_button(
                &mut PixelsWrapper::new(&mut self.pixels),
                self.window.scale_factor(),
                self.cursor_over_button,
            );

            render = true;
        }

        if render {
            self.pixels.render()?;

            if !self.window_visible {
                self.window.set_visible(true);
                self.window_visible = true;
            }
        }

        Ok(())
    }

    pub fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if self.close_button {
            let was_over_button = self.cursor_over_button;
            self.cursor_over_button = is_over_close_button(
                position.x,
                position.y,
                self.width,
                self.window.scale_factor(),
            );

            if was_over_button != self.cursor_over_button {
                self.window.request_redraw();
            }
        }
    }

    pub fn handle_mouse_left_window(&mut self) {
        if self.close_button && self.cursor_over_button {
            self.cursor_over_button = false;
            self.window.request_redraw();
        }
    }

    pub fn handle_click(&mut self) -> bool {
        if self.close_button {
            self.cursor_over_button
        } else {
            true
        }
    }
}

/// A prompt window, rendered using `egui`.
pub struct PromptWindow<'a> {
    pub window: Arc<Window>,
    egui_window: EguiWindow<'a>,
    prompt: String,
    user_input: String,
    closed: bool,
}

impl<'a> PromptWindow<'a> {
    pub fn new(wgpu_state: &WgpuState, window: Window, prompt: String) -> Result<Self> {
        let window = Arc::new(window);
        let window_clone = window.clone();

        Ok(Self {
            window,
            egui_window: EguiWindow::new(wgpu_state, window_clone)?,
            prompt,
            user_input: String::new(),
            closed: false,
        })
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        self.egui_window.handle_event(event);
    }

    pub fn render(&mut self) -> Result<()> {
        let mut user_input = self.user_input.clone();
        let prompt_text = self.prompt.clone();
        let mut closed = self.closed;

        self.egui_window.redraw(|ctx| {
            ctx.set_visuals(egui::Visuals::light());

            egui::CentralPanel::default().show(ctx, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.heading("Repeat after me");
                    ui.add_space(20.0);

                    ui.add(egui::Label::new(format!("\"{}\"", prompt_text)));

                    let response = ui.text_edit_singleline(&mut user_input);
                    response.request_focus();

                    ui.add_space(ui.available_height() - 50.0);

                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("Submit")).clicked() {
                            if user_input == prompt_text {
                                closed = true;
                            }

                            println!("User submitted: {}", user_input);
                        }
                    })
                })
            });
        })?;

        self.user_input = user_input;
        self.closed = closed;

        Ok(())
    }

    pub fn closed(&self) -> bool {
        self.closed
    }
}
