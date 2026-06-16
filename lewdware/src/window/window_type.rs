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
    video::{NextFrame, VideoDecoder, VideoFrame, VideoPixelFormat},
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
            let gpu_renderer = GpuRenderer::new_image(inner_window.wgpu_state(), outer_size.width, outer_size.height, inner_window.opacity, inner_window.premultiplied_alpha());
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

/// A video popup, rendered using wgpu (GPU path) or software YUV conversion (CPU fallback).
pub struct VideoWindow<'a> {
    pub inner_window: InnerWindow<'a>,
    video_player: VideoDecoder,
    last_frame_time: Instant,
    duration: Option<Duration>,
    paused: bool,
    // Present when the window was initialised with GPU support.
    gpu_renderer: Option<GpuRenderer>,
    // GPU path: RGBA overlay for decorations / UI.
    ui_frame_buffer: Vec<u8>,
    // CPU path: ARGB pixel buffer sized to inner_size (display area).
    cpu_frame_buffer: Vec<u32>,
}

impl<'a> VideoWindow<'a> {
    pub fn new(
        inner_window: InnerWindow<'a>,
        mut video_player: VideoDecoder,
        _loop_video: bool,
    ) -> anyhow::Result<Self> {
        let outer_size = inner_window.outer_size();
        let inner_size = inner_window.inner_size();

        let (gpu_renderer, ui_frame_buffer) = if inner_window.is_gpu() {
            let ui_frame_buffer = vec![0u8; (outer_size.width * outer_size.height * 4) as usize];
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
                inner_window.premultiplied_alpha(),
            );
            (Some(gpu_renderer), ui_frame_buffer)
        } else {
            (None, Vec::new())
        };

        let cpu_frame_buffer = vec![0u32; (inner_size.width * inner_size.height) as usize];

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
            cpu_frame_buffer,
        })
    }

    pub fn update(&mut self) -> Result<bool> {
        self.inner_window.start_render()?;

        if self.inner_window.is_gpu() {
            // --- GPU path ---
            if let Some(gpu_renderer) = &self.gpu_renderer {
                gpu_renderer.set_opacity(self.inner_window.wgpu_state(), self.inner_window.opacity);
            }

            let mut render = false;
            let outer_size = self.inner_window.outer_size();

            // self.ui_frame_buffer.fill(0);
            let decorations_rendered = {
                let pixmap = PixmapMut::from_bytes(
                    &mut self.ui_frame_buffer,
                    outer_size.width,
                    outer_size.height,
                )
                .unwrap();
                let mut buffer = Buffer::Pixmap(pixmap);
                self.inner_window.render_decorations(&mut buffer)?
            };

            if decorations_rendered {
                if let Some(gpu_renderer) = &self.gpu_renderer {
                    gpu_renderer.upload_frame_buffer(
                        &self.inner_window.wgpu_state().queue,
                        &self.ui_frame_buffer,
                        outer_size.width,
                        outer_size.height,
                    );
                }
                render = true;
            }

            match self.video_player.next_frame() {
                NextFrame::Ready(frame) => {
                    if let Some(gpu_renderer) = &mut self.gpu_renderer {
                        if let GpuRendererType::Video(video_renderer) =
                            &mut gpu_renderer.renderer_type
                        {
                            video_renderer.update_video(self.inner_window.wgpu_state(), &frame);
                        }
                    }
                    render = true;
                }
                NextFrame::Finish => return Ok(true),
                NextFrame::None => {}
            }

            if render {
                let gpu_renderer = self.gpu_renderer.as_ref();
                let inner_size = self.inner_window.inner_size();
                let outer_w = outer_size.width as f32;
                let outer_h = outer_size.height as f32;
                self.inner_window.draw_wgpu(|rpass, x, y| {
                    if let Some(gpu_renderer) = gpu_renderer {
                        if let GpuRendererType::Video(video) = &gpu_renderer.renderer_type {
                            let (vid_pipeline, vid_bind_group) =
                                video.video_pipeline_and_bind_group();
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
                            rpass.set_viewport(0.0, 0.0, outer_w, outer_h, 0.0, 1.0);
                            rpass.draw(0..4, 0..1);
                        }
                    }
                })?;
            }
        } else {
            // --- CPU path ---
            match self.video_player.next_frame() {
                NextFrame::Ready(frame) => {
                    if frame.frame.width() > 0 {
                        let inner_size = self.inner_window.inner_size();
                        let display_w = self.video_player.native_width();
                        let display_h = self.video_player.native_height();
                        let full_range = self.video_player.full_range();
                        let packed_alpha = self.video_player.packed_alpha();

                        match self.video_player.pixel_format() {
                            VideoPixelFormat::Yuv420p => render_yuv420p_to_argb(
                                &frame,
                                &mut self.cpu_frame_buffer,
                                inner_size.width,
                                inner_size.height,
                                display_w,
                                display_h,
                                full_range,
                                packed_alpha,
                            ),
                            VideoPixelFormat::Nv12 => render_nv12_to_argb(
                                &frame,
                                &mut self.cpu_frame_buffer,
                                inner_size.width,
                                inner_size.height,
                                display_w,
                                display_h,
                                full_range,
                                packed_alpha,
                            ),
                        }
                    }
                }
                NextFrame::Finish => return Ok(true),
                NextFrame::None => {}
            }

            let cpu_frame = &self.cpu_frame_buffer;
            let inner_size = self.inner_window.inner_size();
            let (x, y) = self.inner_window.inner_offset();
            self.inner_window.draw_softbuffer(|buffer| {
                buffer.copy_from_u32_buf(cpu_frame, inner_size.width, x, y);
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
            let gpu_renderer = GpuRenderer::new_image(inner_window.wgpu_state(), outer_size.width, outer_size.height, inner_window.opacity, inner_window.premultiplied_alpha());
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
            let gpu_renderer = GpuRenderer::new_image(inner_window.wgpu_state(), outer_size.width, outer_size.height, inner_window.opacity, inner_window.premultiplied_alpha());
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

// BT.709 YCbCr → linear RGB (clipped). Limited range scales Y from [16,235] and Cb/Cr from
// [16,240]; full range (JPEG / yuvj420p) maps [0,255] directly.
fn yuv_to_argb(y: u8, cb: u8, cr: u8, alpha: u8, full_range: bool) -> u32 {
    let (y_f, cb_f, cr_f) = if full_range {
        (y as f32 / 255.0, cb as f32 / 255.0 - 0.5, cr as f32 / 255.0 - 0.5)
    } else {
        (
            (y as f32 - 16.0) / 219.0,
            (cb as f32 - 128.0) / 224.0,
            (cr as f32 - 128.0) / 224.0,
        )
    };
    let r = ((y_f + 1.57480 * cr_f).clamp(0.0, 1.0) * 255.0) as u8;
    let g = ((y_f - 0.18732 * cb_f - 0.46812 * cr_f).clamp(0.0, 1.0) * 255.0) as u8;
    let b = ((y_f + 1.85560 * cb_f).clamp(0.0, 1.0) * 255.0) as u8;
    ((alpha as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Convert a YUV420P `VideoFrame` into ARGB u32 pixels scaled to `(dst_w, dst_h)`.
/// `packed_alpha`: top half = colour, bottom half = alpha-as-luma (same layout as packed MP4).
fn render_yuv420p_to_argb(
    frame: &VideoFrame,
    dst: &mut [u32],
    dst_w: u32,
    dst_h: u32,
    src_display_w: u32,
    src_display_h: u32,
    full_range: bool,
    packed_alpha: bool,
) {
    let f = &frame.frame;
    let y_data = f.data(0);
    let cb_data = f.data(1);
    let cr_data = f.data(2);
    let y_stride = f.stride(0) as usize;
    let cb_stride = f.stride(1) as usize;
    let cr_stride = f.stride(2) as usize;

    let sw = src_display_w as usize;
    let sh = src_display_h as usize;
    let dw = dst_w as usize;
    let dh = dst_h as usize;

    for dy in 0..dh {
        let sy = (dy * sh) / dh;
        let cy = sy / 2;
        let ay = sy + sh; // alpha row offset (packed only)
        for dx in 0..dw {
            let sx = (dx * sw) / dw;
            let cx = sx / 2;
            let y = y_data[sy * y_stride + sx];
            let cb = cb_data[cy * cb_stride + cx];
            let cr = cr_data[cy * cr_stride + cx];
            let alpha = if packed_alpha { y_data[ay * y_stride + sx] } else { 255 };
            dst[dy * dw + dx] = yuv_to_argb(y, cb, cr, alpha, full_range);
        }
    }
}

/// Convert an NV12 `VideoFrame` into ARGB u32 pixels scaled to `(dst_w, dst_h)`.
/// Handles both software NV12 (from `av_hwframe_transfer_data`) and the packed-alpha layout.
fn render_nv12_to_argb(
    frame: &VideoFrame,
    dst: &mut [u32],
    dst_w: u32,
    dst_h: u32,
    src_display_w: u32,
    src_display_h: u32,
    full_range: bool,
    packed_alpha: bool,
) {
    let f = &frame.frame;
    let y_data = f.data(0);
    let uv_data = f.data(1);
    let y_stride = f.stride(0) as usize;
    let uv_stride = f.stride(1) as usize;

    let sw = src_display_w as usize;
    let sh = src_display_h as usize;
    let dw = dst_w as usize;
    let dh = dst_h as usize;

    for dy in 0..dh {
        let sy = (dy * sh) / dh;
        let cy = sy / 2;
        let ay = sy + sh;
        for dx in 0..dw {
            let sx = (dx * sw) / dw;
            let cx = sx / 2;
            let y = y_data[sy * y_stride + sx];
            // UV plane: interleaved Cb Cr pairs
            let cb = uv_data[cy * uv_stride + cx * 2];
            let cr = uv_data[cy * uv_stride + cx * 2 + 1];
            let alpha = if packed_alpha { y_data[ay * y_stride + sx] } else { 255 };
            dst[dy * dw + dx] = yuv_to_argb(y, cb, cr, alpha, full_range);
        }
    }
}
