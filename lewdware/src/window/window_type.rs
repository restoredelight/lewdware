use std::time::{Duration, Instant};

use anyhow::Result;
use egui::{RichText, TextEdit};
use tiny_skia::{IntSize, Pixmap, PixmapMut};
use winit::{
    dpi::{LogicalPosition, PhysicalPosition},
    event::{Touch, WindowEvent},
};

use crate::{
    egui::EguiCPUWindow,
    lua::{self, ChoiceWindowOption},
    media::ImageData,
    video::{NextFrame, VideoDecoder},
    window::{
        gpu_renderer::{GpuRenderer, GpuRendererType},
        header::HEADER_HEIGHT,
        inner_window::InnerWindow,
        surface::Buffer,
    },
};

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

/// A window displaying an image.
pub struct ImageWindow<'a> {
    pub inner_window: InnerWindow<'a>,
    image: Pixmap,
    gpu_renderer: Option<GpuRenderer>,
    frame_buffer: Vec<u8>,
}

impl<'a> ImageWindow<'a> {
    pub fn new(inner_window: InnerWindow<'a>, image: ImageData) -> Result<Self> {
        let width = image.width();
        let height = image.height();

        let image_pixmap =
            Pixmap::from_vec(image.into_vec(), IntSize::from_wh(width, height).unwrap()).unwrap();

        let (gpu_renderer, frame_buffer) = if inner_window.is_gpu() {
            let outer_size = inner_window.outer_size();
            let frame_buffer = vec![0; (outer_size.width * outer_size.height * 4) as usize];
            let gpu_renderer = GpuRenderer::new_image(inner_window.wgpu_state(), outer_size.width, outer_size.height, inner_window.opacity);
            (Some(gpu_renderer), frame_buffer)
        } else {
            (None, Vec::new())
        };

        Ok(Self {
            inner_window,
            image: image_pixmap,
            gpu_renderer,
            frame_buffer,
        })
    }

    pub fn draw(&mut self) -> Result<()> {
        self.inner_window.start_render()?;
        let (x, y) = self.inner_window.inner_offset();

        // Check if opacity changed
        if let Some(gpu_renderer) = &self.gpu_renderer {
            gpu_renderer.set_opacity(self.inner_window.wgpu_state(), self.inner_window.opacity);
        }

        if let Some(gpu_renderer) = &mut self.gpu_renderer {
            let outer_size = self.inner_window.outer_size();
            {
                let pixmap = PixmapMut::from_bytes(&mut self.frame_buffer, outer_size.width, outer_size.height).unwrap();
                let mut buffer = Buffer::Pixmap(pixmap);

                buffer.copy_from_pixmap(&self.image, x, y);
                self.inner_window.render_decorations(&mut buffer)?;
            }

            gpu_renderer.upload_frame_buffer(&self.inner_window.wgpu_state().queue, &self.frame_buffer, outer_size.width, outer_size.height);

            let pipeline = self.inner_window.wgpu_state().get_pipeline(self.inner_window.surface_format().unwrap());
            
            self.inner_window.draw_wgpu(|rpass, _x, _y| {
                if let GpuRendererType::Image { bind_group, .. } = &gpu_renderer.renderer_type {
                    rpass.set_pipeline(&pipeline);
                    rpass.set_bind_group(0, bind_group, &[]);
                    rpass.set_bind_group(1, &gpu_renderer.window_bind_group, &[]);
                    rpass.draw(0..4, 0..1);
                }
            })?;
        } else {
            let image = &self.image;
            self.inner_window.draw_softbuffer(|buffer| {
                buffer.copy_from_pixmap(image, x, y);
            })?;
        }

        Ok(())
    }
}

/// A video popup, rendered using wgpu.
pub struct VideoWindow<'a> {
    pub inner_window: InnerWindow<'a>,
    video_player: VideoDecoder,
    last_frame_time: Instant,
    duration: Option<Duration>,
    paused: bool,
    gpu_renderer: GpuRenderer,
    ui_frame_buffer: Vec<u8>,
}

impl<'a> VideoWindow<'a> {
    pub fn new(
        inner_window: InnerWindow<'a>,
        mut video_player: VideoDecoder,
        _loop_video: bool,
    ) -> anyhow::Result<Self> {
        let outer_size = inner_window.outer_size();
        let ui_frame_buffer = vec![0; (outer_size.width * outer_size.height * 4) as usize];
        let gpu_renderer = GpuRenderer::new_video(
            inner_window.wgpu_state(),
            inner_window.surface_format().unwrap(),
            video_player.native_width(),
            video_player.native_height(),
            video_player.full_range(),
            video_player.pixel_format(),
            video_player.packed_alpha(),
            outer_size.width,
            outer_size.height,
            inner_window.opacity,
        );

        video_player.play();

        inner_window.window().request_redraw();

        Ok(Self {
            inner_window,
            video_player,
            last_frame_time: Instant::now(),
            duration: None,
            paused: false,
            gpu_renderer,
            ui_frame_buffer,
        })
    }

    pub fn update(&mut self) -> Result<bool> {
        self.inner_window.start_render()?;

        self.gpu_renderer.set_opacity(self.inner_window.wgpu_state(), self.inner_window.opacity);

        let mut render = false;

        let outer_size = self.inner_window.outer_size();
        self.ui_frame_buffer.fill(0);
        let decorations_rendered = {
            let pixmap = PixmapMut::from_bytes(&mut self.ui_frame_buffer, outer_size.width, outer_size.height).unwrap();
            let mut buffer = Buffer::Pixmap(pixmap);
            self.inner_window.render_decorations(&mut buffer)?
        };

        if decorations_rendered {
            self.gpu_renderer.upload_frame_buffer(&self.inner_window.wgpu_state().queue, &self.ui_frame_buffer, outer_size.width, outer_size.height);
            render = true;
        }

        match self.video_player.next_frame() {
            NextFrame::Ready(frame) => {
                if let GpuRendererType::Video(video_renderer) = &mut self.gpu_renderer.renderer_type {
                    video_renderer.update_video(self.inner_window.wgpu_state(), &frame);
                }
                render = true;
            }
            NextFrame::Finish => {
                return Ok(true);
            }
            NextFrame::None => {}
        }

        if render {
            let gpu_renderer = &self.gpu_renderer;
            let inner_size = self.inner_window.inner_size();
            self.inner_window.draw_wgpu(|rpass, x, y| {
                if let GpuRendererType::Video(video) = &gpu_renderer.renderer_type {
                    let (vid_pipeline, vid_bind_group) = video.video_pipeline_and_bind_group();
                    rpass.set_pipeline(vid_pipeline);
                    rpass.set_bind_group(0, vid_bind_group, &[]);
                    rpass.set_bind_group(1, &gpu_renderer.window_bind_group, &[]);
                    
                    rpass.set_viewport(
                        x as f32,
                        y as f32,
                        inner_size.width as f32,
                        inner_size.height as f32,
                        0.0,
                        1.0,
                    );
                    rpass.draw(0..4, 0..1);

                    rpass.set_pipeline(video.ui_pipeline());
                    rpass.set_bind_group(0, video.ui_bind_group(), &[]);
                    rpass.set_bind_group(1, &gpu_renderer.window_bind_group, &[]);
                    rpass.set_viewport(
                        0.0,
                        0.0,
                        outer_size.width as f32,
                        outer_size.height as f32,
                        0.0,
                        1.0,
                    );
                    rpass.draw(0..4, 0..1);
                }
            })?;
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

pub struct PromptWindow<'a> {
    pub inner_window: InnerWindow<'a>,
    egui_window: EguiCPUWindow,
    text: Option<String>,
    placeholder: Option<String>,
    value: String,
    gpu_renderer: Option<GpuRenderer>,
    frame_buffer: Vec<u8>,
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

        let (gpu_renderer, frame_buffer) = if inner_window.is_gpu() {
            let outer_size = inner_window.outer_size();
            let frame_buffer = vec![0; (outer_size.width * outer_size.height * 4) as usize];
            let gpu_renderer = GpuRenderer::new_image(inner_window.wgpu_state(), outer_size.width, outer_size.height, inner_window.opacity);
            (Some(gpu_renderer), frame_buffer)
        } else {
            (None, Vec::new())
        };

        Ok(Self {
            inner_window,
            egui_window,
            text,
            placeholder,
            value: initial_value.unwrap_or_default(),
            gpu_renderer,
            frame_buffer,
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

        if let Some(gpu_renderer) = &self.gpu_renderer {
            gpu_renderer.set_opacity(self.inner_window.wgpu_state(), self.inner_window.opacity);
        }

        let id = self.inner_window.window().id();
        let lua_event_tx = self.inner_window.lua_event_tx().clone();
        
        let inner_size = self.inner_window.inner_size();
        let outer_size = self.inner_window.outer_size();
        let (x, y) = self.inner_window.inner_offset();

        if let Some(gpu_renderer) = &mut self.gpu_renderer {
            self.frame_buffer.fill(0);
            
            let mut egui_buffer = vec![0u32; (inner_size.width * inner_size.height) as usize];
            let mut buffer_ref = egui_software_backend::BufferMutRef::new(
                bytemuck::cast_slice_mut(&mut egui_buffer),
                inner_size.width as usize,
                inner_size.height as usize,
            );

            let _ = self.egui_window.redraw(&mut buffer_ref, |ui| {
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
            });

            {
                let pixmap = PixmapMut::from_bytes(&mut self.frame_buffer, outer_size.width, outer_size.height).unwrap();
                let mut buffer = Buffer::Pixmap(pixmap);
                buffer.copy_from_u32_buf(&egui_buffer, inner_size.width, x, y);
                self.inner_window.render_decorations(&mut buffer)?;
            }

            gpu_renderer.upload_frame_buffer(&self.inner_window.wgpu_state().queue, &self.frame_buffer, outer_size.width, outer_size.height);

            let pipeline = self.inner_window.wgpu_state().get_pipeline(self.inner_window.surface_format().unwrap());
            
            self.inner_window.draw_wgpu(|rpass, _x, _y| {
                if let GpuRendererType::Image { bind_group, .. } = &gpu_renderer.renderer_type {
                    rpass.set_pipeline(&pipeline);
                    rpass.set_bind_group(0, bind_group, &[]);
                    rpass.set_bind_group(1, &gpu_renderer.window_bind_group, &[]);
                    rpass.draw(0..4, 0..1);
                }
            })?;
        } else {
            self.inner_window.draw_softbuffer(|buffer| {
                let mut egui_buffer = vec![0u32; (inner_size.width * inner_size.height) as usize];
                let mut buffer_ref = egui_software_backend::BufferMutRef::new(
                    bytemuck::cast_slice_mut(&mut egui_buffer),
                    inner_size.width as usize,
                    inner_size.height as usize,
                );

                let _ = self.egui_window.redraw(&mut buffer_ref, |ui| {
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
                });

                buffer.copy_from_u32_buf(&egui_buffer, inner_size.width, x, y);
            })?;
        }

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
    pub inner_window: InnerWindow<'a>,
    egui_window: EguiCPUWindow,
    text: Option<String>,
    options: Vec<ChoiceWindowOption>,
    gpu_renderer: Option<GpuRenderer>,
    frame_buffer: Vec<u8>,
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

        let (gpu_renderer, frame_buffer) = if inner_window.is_gpu() {
            let outer_size = inner_window.outer_size();
            let frame_buffer = vec![0; (outer_size.width * outer_size.height * 4) as usize];
            let gpu_renderer = GpuRenderer::new_image(inner_window.wgpu_state(), outer_size.width, outer_size.height, inner_window.opacity);
            (Some(gpu_renderer), frame_buffer)
        } else {
            (None, Vec::new())
        };

        Ok(Self {
            inner_window,
            egui_window,
            text,
            options,
            gpu_renderer,
            frame_buffer,
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

        if let Some(gpu_renderer) = &self.gpu_renderer {
            gpu_renderer.set_opacity(self.inner_window.wgpu_state(), self.inner_window.opacity);
        }

        let id = self.inner_window.window().id();
        let lua_event_tx = self.inner_window.lua_event_tx().clone();
        
        let inner_size = self.inner_window.inner_size();
        let outer_size = self.inner_window.outer_size();
        let (x, y) = self.inner_window.inner_offset();

        if let Some(gpu_renderer) = &mut self.gpu_renderer {
            self.frame_buffer.fill(0);
            
            let mut egui_buffer = vec![0u32; (inner_size.width * inner_size.height) as usize];
            let mut buffer_ref = egui_software_backend::BufferMutRef::new(
                bytemuck::cast_slice_mut(&mut egui_buffer),
                inner_size.width as usize,
                inner_size.height as usize,
            );

            let _ = self.egui_window.redraw(&mut buffer_ref, |ui| {
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
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
            });

            {
                let pixmap = PixmapMut::from_bytes(&mut self.frame_buffer, outer_size.width, outer_size.height).unwrap();
                let mut buffer = Buffer::Pixmap(pixmap);
                buffer.copy_from_u32_buf(&egui_buffer, inner_size.width, x, y);
                self.inner_window.render_decorations(&mut buffer)?;
            }

            gpu_renderer.upload_frame_buffer(&self.inner_window.wgpu_state().queue, &self.frame_buffer, outer_size.width, outer_size.height);

            let pipeline = self.inner_window.wgpu_state().get_pipeline(self.inner_window.surface_format().unwrap());
            
            self.inner_window.draw_wgpu(|rpass, _x, _y| {
                if let GpuRendererType::Image { bind_group, .. } = &gpu_renderer.renderer_type {
                    rpass.set_pipeline(&pipeline);
                    rpass.set_bind_group(0, bind_group, &[]);
                    rpass.set_bind_group(1, &gpu_renderer.window_bind_group, &[]);
                    rpass.draw(0..4, 0..1);
                }
            })?;
        } else {
            self.inner_window.draw_softbuffer(|buffer| {
                let mut egui_buffer = vec![0u32; (inner_size.width * inner_size.height) as usize];
                let mut buffer_ref = egui_software_backend::BufferMutRef::new(
                    bytemuck::cast_slice_mut(&mut egui_buffer),
                    inner_size.width as usize,
                    inner_size.height as usize,
                );

                let _ = self.egui_window.redraw(&mut buffer_ref, |ui| {
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
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
                });

                buffer.copy_from_u32_buf(&egui_buffer, inner_size.width, x, y);
            })?;
        }

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
