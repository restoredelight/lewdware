use std::time::{Duration, Instant};

use anyhow::Result;
use egui::{RichText, TextEdit};
use tiny_skia::{IntSize, Pixmap};
use winit::{dpi::{LogicalPosition, PhysicalPosition}, event::{Touch, WindowEvent}};

use crate::{egui::EguiCPUWindow, lua::{self, ChoiceWindowOption}, media::ImageData, video::{NextFrame, VideoDecoder}, window::{header::HEADER_HEIGHT, inner_window::InnerWindow}};

pub enum WindowType<'a> {
    Image(ImageWindow<'a>),
    Video(VideoWindow<'a>),
    Prompt(PromptWindow<'a>),
    Choice(ChoiceWindow<'a>),
}

impl<'a> WindowType<'a> {
    pub fn inner_window(&self) -> &InnerWindow<'_> {
        match self {
            Self::Image(image_window) => &image_window.inner_window,
            Self::Video(video_window) => &video_window.inner_window,
            Self::Prompt(prompt_window) => &prompt_window.inner_window,
            Self::Choice(choice_window) => &choice_window.inner_window,
        }
    }

    pub fn inner_window_mut(&mut self) -> &mut InnerWindow<'a> {
        match self {
            Self::Image(image_window) => &mut image_window.inner_window,
            Self::Video(video_window) => &mut video_window.inner_window,
            Self::Prompt(prompt_window) => &mut prompt_window.inner_window,
            Self::Choice(choice_window) => &mut choice_window.inner_window,
        }
    }
}

/// A window displaying an image. Image windows are rendered using softbuffer.
pub struct ImageWindow<'a> {
    inner_window: InnerWindow<'a>,
    image: Pixmap,
}

impl<'a> ImageWindow<'a> {
    /// Create a new image window.
    ///
    /// * `close_button`: Whether to display a close button on the window.
    /// * `moving`: Whether to move the window around the screen.
    pub fn new(inner_window: InnerWindow<'a>, image: ImageData) -> Result<Self> {
        let width = image.width();
        let height = image.height();

        let image_pixmap =
            Pixmap::from_vec(image.into_vec(), IntSize::from_wh(width, height).unwrap()).unwrap();

        Ok(Self {
            inner_window,
            image: image_pixmap,
        })
    }

    pub fn draw(&mut self) -> Result<()> {
        self.inner_window.start_render()?;
        let render = self.inner_window.render_decorations()?;

        self.inner_window.render_pixmap(&self.image)?;

        if render {
            self.inner_window.present()?;
        }

        Ok(())
    }
}

/// A video popup, rendered using wgpu.
pub struct VideoWindow<'a> {
    inner_window: InnerWindow<'a>,
    video_player: VideoDecoder,
    last_frame_time: Instant,
    duration: Option<Duration>,
    loop_video: bool,
    paused: bool,
}

impl<'a> VideoWindow<'a> {
    /// Create a new video popup.
    ///
    /// * `close_button`: Whether to display a close button on the window.
    /// * `play_audio`: Whether to play the video's audio.
    pub fn new(
        mut inner_window: InnerWindow<'a>,
        mut video_player: VideoDecoder,
        loop_video: bool,
    ) -> anyhow::Result<Self> {
        inner_window.init_video_texture(
            video_player.native_width(),
            video_player.native_height(),
            video_player.full_range(),
            video_player.pixel_format(),
        )?;

        video_player.play();

        inner_window.window().request_redraw();

        Ok(Self {
            inner_window,
            video_player,
            last_frame_time: Instant::now(),
            duration: None,
            loop_video,
            paused: false,
        })
    }

    pub fn update(&mut self) -> Result<bool> {
        let mut render = false;

        self.inner_window.start_render()?;

        render = render || self.inner_window.render_decorations()?;

        match self.video_player.next_frame() {
            NextFrame::Ready(frame) => {
                self.inner_window.render_frame(&frame)?;

                render = true;
            }
            NextFrame::Finish => {
                return Ok(true);
            }
            NextFrame::None => {
                // println!("No frame received");
            }
        }

        if render {
            self.inner_window.present()?;
        }

        Ok(false)
    }

    pub fn pause(&mut self) {
        self.video_player.pause();
        self.paused = true;

        if let Some(duration) = self.duration.take() {
            self.duration = Some(duration - self.last_frame_time.elapsed());
        }
    }

    pub fn play(&mut self) {
        self.paused = false;
        self.last_frame_time = Instant::now();

        self.video_player.play();
    }
}


/// A prompt window, rendered using `egui`.
pub struct PromptWindow<'a> {
    inner_window: InnerWindow<'a>,
    egui_window: EguiCPUWindow,
    text: Option<String>,
    placeholder: Option<String>,
    value: String,
}

impl<'a> PromptWindow<'a> {
    pub fn new(
        inner_window: InnerWindow<'a>,
        text: Option<String>,
        placeholder: Option<String>,
        initial_value: Option<String>,
    ) -> Result<Self> {
        let egui_window = EguiCPUWindow::new(
            inner_window.window().clone(),
            inner_window.is_gpu(),
            inner_window.transparent(),
        )?;

        Ok(Self {
            inner_window,
            egui_window,
            text,
            placeholder,
            value: initial_value.unwrap_or_default(),
        })
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        let event = if self.inner_window.decorations() {
            &translate_event_position(event.clone(), self.inner_window.window().scale_factor())
        } else {
            event
        };

        self.egui_window.handle_event(event);
    }

    pub fn render(&mut self) -> Result<()> {
        self.inner_window.start_render()?;

        let id = self.inner_window.window().id();
        let lua_event_tx = self.inner_window.lua_event_tx().clone();

        self.inner_window.render_with_softbuffer_buffer(|buffer| {
            self.egui_window.redraw(buffer, |ui| {
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        ui.heading("Repeat after me");
                        ui.add_space(20.0);

                        if let Some(text) = &self.text {
                            ui.label(RichText::new(text).heading());
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
                                if let Err(err) = lua_event_tx.send(lua::Event::PromptSubmit {
                                    id,
                                    text: self.value.clone(),
                                }) {
                                    eprintln!("{err}");
                                }
                            }
                        })
                    })
                });
            })
        })?;

        self.inner_window.present()?;

        Ok(())
    }

    pub fn set_text(&mut self, text: Option<String>) {
        self.text = text;
        self.inner_window.window().request_redraw();
    }

    pub fn set_value(&mut self, value: Option<String>) {
        self.value = value.unwrap_or_default();
        self.inner_window.window().request_redraw();
    }
}


pub struct ChoiceWindow<'a> {
    inner_window: InnerWindow<'a>,
    egui_window: EguiCPUWindow,
    text: Option<String>,
    options: Vec<ChoiceWindowOption>,
}

impl<'a> ChoiceWindow<'a> {
    pub fn new(
        inner_window: InnerWindow<'a>,
        text: Option<String>,
        options: Vec<ChoiceWindowOption>,
    ) -> Result<Self> {
        let egui_window = EguiCPUWindow::new(
            inner_window.window().clone(),
            inner_window.is_gpu(),
            inner_window.transparent(),
        )?;

        Ok(Self {
            inner_window,
            egui_window,
            text,
            options,
        })
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        let event = if self.inner_window.decorations() {
            &translate_event_position(event.clone(), self.inner_window.window().scale_factor())
        } else {
            event
        };

        self.egui_window.handle_event(event);
    }

    pub fn render(&mut self) -> Result<()> {
        self.inner_window.start_render()?;
        self.inner_window.render_decorations()?;

        let id = self.inner_window.window().id();
        let lua_event_tx = self.inner_window.lua_event_tx().clone();

        self.inner_window.render_with_softbuffer_buffer(|buffer| {
            self.egui_window.redraw(buffer, |ui| {
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        // ui.heading("Repeat after me");
                        ui.add_space(20.0);

                        if let Some(text) = &self.text {
                            ui.label(RichText::new(text).heading());
                        }

                        ui.add_space(ui.available_height() - 100.0);

                        ui.with_layout(
                            egui::Layout::left_to_right(egui::Align::Center)
                                .with_main_wrap(true)
                                .with_main_align(egui::Align::Center)
                                .with_main_justify(true),
                            |ui| {
                                for option in &self.options {
                                    if ui.button(&option.label).clicked() {
                                        let _ = lua_event_tx.send(lua::Event::ChoiceSelect {
                                            id,
                                            option_id: option.id.clone(),
                                        });
                                    }
                                    ui.add_space(5.0);
                                }
                            },
                        )
                    })
                });
            })
        })?;

        self.inner_window.present()?;

        Ok(())
    }

    pub fn set_text(&mut self, text: Option<String>) {
        self.text = text;
        self.inner_window.window().request_redraw();
    }

    pub fn set_options(&mut self, options: Vec<ChoiceWindowOption>) {
        self.options = options;
        self.inner_window.window().request_redraw();
    }
}


fn translate_event_position(event: WindowEvent, scale_factor: f64) -> WindowEvent {
    match event {
        WindowEvent::CursorMoved {
            device_id,
            position,
        } => WindowEvent::CursorMoved {
            device_id,
            position: translate_position(position, scale_factor),
        },
        WindowEvent::Touch(Touch {
            device_id,
            phase,
            location,
            force,
            id,
        }) => WindowEvent::Touch(Touch {
            device_id,
            phase,
            location: translate_position(location, scale_factor),
            force,
            id,
        }),
        event => event,
    }
}

fn translate_position(position: PhysicalPosition<f64>, scale_factor: f64) -> PhysicalPosition<f64> {
    let mut logical_position: LogicalPosition<f64> = position.to_logical(scale_factor);
    logical_position.x -= 1.0;
    logical_position.y -= 1.0 + HEADER_HEIGHT as f64;

    return logical_position.to_physical(scale_factor);
}
