use std::time::{Duration, Instant};

use anyhow::Result;
use egui::{RichText, TextEdit};
use tiny_skia::{IntSize, Pixmap, PixmapMut};
use winit::{
    dpi::{LogicalPosition, PhysicalPosition},
    event::{Touch, WindowEvent},
};

use crate::{
    egui::{EguiCPUWindow, EguiGpuRenderer},
    lua::{self, ChoiceWindowOption, TextStyle},
    media::ImageData,
    text_font,
    video::{NextFrame, VideoDecoder, VideoFrame, VideoPixelFormat},
    window::{
        gpu_renderer::{DecorationOverlay, GpuRenderer, GpuRendererType},
        header::HEADER_HEIGHT,
        inner_window::InnerWindow,
        surface::Buffer,
    },
};

pub enum WindowType {
    Image(ImageWindow),
    Video(VideoWindow),
    Prompt(PromptWindow),
    Choice(ChoiceWindow),
    Text(TextWindow),
}

impl WindowType {
    pub fn inner_window(&self) -> &InnerWindow {
        match self {
            Self::Image(image_window) => &image_window.inner_window,
            Self::Video(video_window) => &video_window.inner_window,
            Self::Prompt(prompt_window) => &prompt_window.inner_window,
            Self::Choice(choice_window) => &choice_window.inner_window,
            Self::Text(text_window) => &text_window.inner_window,
        }
    }

    pub fn inner_window_mut(&mut self) -> &mut InnerWindow {
        match self {
            Self::Image(image_window) => &mut image_window.inner_window,
            Self::Video(video_window) => &mut video_window.inner_window,
            Self::Prompt(prompt_window) => &mut prompt_window.inner_window,
            Self::Choice(choice_window) => &mut choice_window.inner_window,
            Self::Text(text_window) => &mut text_window.inner_window,
        }
    }

    /// Consume this `WindowType`, dropping all rendering resources and returning the
    /// underlying `InnerWindow`. Field declaration order on each variant ensures that
    /// egui/overlay resources (which hold `Arc<Window>` clones) are dropped first.
    pub fn into_inner_window(self) -> InnerWindow {
        match self {
            Self::Image(w) => w.inner_window,
            Self::Video(w) => w.inner_window,
            Self::Prompt(w) => w.inner_window,
            Self::Choice(w) => w.inner_window,
            Self::Text(w) => w.inner_window,
        }
    }
}

/// A window displaying an image.
pub struct ImageWindow {
    pub inner_window: InnerWindow,
    image: Pixmap,
    gpu_renderer: Option<GpuRenderer>,
    frame_buffer: Vec<u8>,
}

impl ImageWindow {
    pub fn new(inner_window: InnerWindow, image: ImageData) -> Result<Self> {
        let width = image.width();
        let height = image.height();

        let image_pixmap =
            Pixmap::from_vec(image.into_vec(), IntSize::from_wh(width, height).unwrap()).unwrap();

        let (gpu_renderer, frame_buffer) = if inner_window.is_gpu() {
            let outer_size = inner_window.outer_size();
            let frame_buffer = vec![0; (outer_size.width * outer_size.height * 4) as usize];
            let gpu_renderer = GpuRenderer::new_image(
                inner_window.wgpu_state(),
                outer_size.width,
                outer_size.height,
                inner_window.opacity,
                inner_window.premultiplied_alpha(),
                inner_window.force_opaque(),
            );
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

    pub fn draw(&mut self) -> Result<Option<wgpu::SubmissionIndex>> {
        self.inner_window.start_render()?;
        let (x, y) = self.inner_window.inner_offset();

        // Check if opacity changed
        if let Some(gpu_renderer) = &self.gpu_renderer {
            gpu_renderer.set_opacity(self.inner_window.wgpu_state(), self.inner_window.opacity);
        }

        if let Some(gpu_renderer) = &mut self.gpu_renderer {
            let outer_size = self.inner_window.outer_size();
            {
                let pixmap = PixmapMut::from_bytes(
                    &mut self.frame_buffer,
                    outer_size.width,
                    outer_size.height,
                )
                .unwrap();
                let mut buffer = Buffer::Pixmap(pixmap);

                buffer.copy_from_pixmap(&self.image, x, y);
                self.inner_window.render_decorations(&mut buffer)?;
            }

            gpu_renderer.upload_frame_buffer(
                &self.inner_window.wgpu_state().queue,
                &self.frame_buffer,
                outer_size.width,
                outer_size.height,
            );

            let pipeline = self
                .inner_window
                .wgpu_state()
                .get_pipeline(self.inner_window.surface_format().unwrap());

            return self.inner_window.draw_wgpu(|rpass, _x, _y| {
                if let GpuRendererType::Image { bind_group, .. } = &gpu_renderer.renderer_type {
                    rpass.set_pipeline(&pipeline);
                    rpass.set_bind_group(0, bind_group, &[]);
                    rpass.set_bind_group(1, &gpu_renderer.window_bind_group, &[]);
                    rpass.draw(0..4, 0..1);
                }
            });
        } else {
            let image = &self.image;
            self.inner_window.draw_softbuffer(|buffer| {
                buffer.copy_from_pixmap(image, x, y);
            })?;
        }

        Ok(None)
    }
}

/// A video popup, rendered using wgpu (GPU path) or software YUV conversion (CPU fallback).
pub struct VideoWindow {
    pub inner_window: InnerWindow,
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

impl VideoWindow {
    pub fn new(
        inner_window: InnerWindow,
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
                inner_window.force_opaque(),
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

pub struct PromptWindow {
    text: Option<String>,
    placeholder: Option<String>,
    value: String,
    egui_cpu: Option<EguiCPUWindow>,
    egui_gpu: Option<EguiGpuRenderer>,
    decoration_overlay: Option<DecorationOverlay>,
    // Declared last so it drops last: egui's Arc<Window> clone is released first.
    pub inner_window: InnerWindow,
}

impl PromptWindow {
    pub fn new(
        inner_window: InnerWindow,
        text: Option<String>,
        placeholder: Option<String>,
        initial_value: Option<String>,
    ) -> Result<Self> {
        let (egui_cpu, egui_gpu, decoration_overlay) = if inner_window.is_gpu() {
            let surface_format = inner_window.surface_format().unwrap();
            let inner_size = inner_window.inner_size();
            let egui_gpu = EguiGpuRenderer::new(
                inner_window.wgpu_state(),
                inner_window.window(),
                inner_size,
                inner_window.opacity,
                inner_window.premultiplied_alpha(),
                inner_window.force_opaque(),
                inner_window.background_color(),
                None,
            )?;
            let decoration_overlay = if inner_window.decorations() {
                let outer_size = inner_window.outer_size();
                Some(DecorationOverlay::new(
                    inner_window.wgpu_state(),
                    outer_size.width,
                    outer_size.height,
                    inner_window.premultiplied_alpha(),
                    inner_window.opacity,
                    inner_window.force_opaque(),
                ))
            } else {
                None
            };
            let _ = surface_format; // only needed to confirm surface is GPU
            (None, Some(egui_gpu), decoration_overlay)
        } else {
            let egui_cpu = EguiCPUWindow::new(
                inner_window.window().clone(),
                inner_window.background_color(),
                None,
            )?;
            (Some(egui_cpu), None, None)
        };

        Ok(Self {
            text,
            placeholder,
            value: initial_value.unwrap_or_default(),
            egui_cpu,
            egui_gpu,
            decoration_overlay,
            inner_window,
        })
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        let translated = if self.inner_window.decorations() {
            Some(translate_event_position(
                event.clone(),
                self.inner_window.window().scale_factor(),
            ))
        } else {
            None
        };
        let translated_ref = translated.as_ref().unwrap_or(event);

        if let Some(egui_gpu) = &mut self.egui_gpu {
            egui_gpu.handle_event(self.inner_window.window(), translated_ref);
        } else if let Some(egui_cpu) = &mut self.egui_cpu {
            egui_cpu.handle_event(translated_ref);
        }
    }

    pub fn render(&mut self) -> Result<()> {
        self.inner_window.start_render()?;

        let id = self.inner_window.window().id();
        let lua_event_tx = self.inner_window.lua_event_tx().clone();
        let inner_size = self.inner_window.inner_size();
        let (ox, oy) = self.inner_window.inner_offset();
        let opacity = self.inner_window.opacity;

        if self.egui_gpu.is_some() {
            let wgpu_state = self.inner_window.wgpu_state().clone();
            let window = self.inner_window.window().clone();

            // Render egui into the intermediate texture.
            let text = self.text.clone();
            let placeholder = self.placeholder.clone();
            self.egui_gpu.as_mut().unwrap().render_to_texture(
                &wgpu_state,
                &window,
                inner_size,
                |ui| {
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.heading("Repeat after me");
                            ui.add_space(20.0);

                            if let Some(text) = &text {
                                ui.label(RichText::new(text).heading());
                            }

                            let mut prompt = TextEdit::singleline(&mut self.value);
                            if let Some(placeholder) = &placeholder {
                                prompt = prompt.hint_text(placeholder);
                            }
                            let response = ui.add(prompt);
                            response.request_focus();

                            ui.add_space(ui.available_height() - 50.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new("Submit")).clicked() {
                                    if let Err(err) = lua_event_tx.send(lua::Event::PromptSubmit {
                                        id,
                                        text: self.value.clone(),
                                    }) {
                                        tracing::error!("{err}");
                                    }
                                }
                            });
                        });
                    });
                },
            )?;

            // Upload header pixmap to decoration overlay if it changed.
            let decoration_overlay = &mut self.decoration_overlay;
            self.inner_window.with_header_pixmap(|pixmap| {
                if let Some(overlay) = decoration_overlay {
                    overlay.upload_header(&wgpu_state.queue, pixmap, ox, ox);
                }
            });

            // Update opacity for both layers.
            if let Some(overlay) = &self.decoration_overlay {
                overlay.set_opacity(&wgpu_state.queue, opacity);
            }
            self.egui_gpu
                .as_ref()
                .unwrap()
                .set_opacity(&wgpu_state.queue, opacity);

            // Blit egui texture and decoration overlay into the surface.
            let surface_format = self.inner_window.surface_format().unwrap();
            let pipeline = wgpu_state.get_pipeline(surface_format);
            let egui_bind_group = &self.egui_gpu.as_ref().unwrap().bind_group;
            let egui_window_bind_group = &self.egui_gpu.as_ref().unwrap().window_bind_group;
            let decoration_overlay = self.decoration_overlay.as_ref();

            self.inner_window.draw_wgpu(|rpass, x, y| {
                // Egui layer: inner viewport.
                rpass.set_pipeline(&pipeline);
                rpass.set_bind_group(0, egui_bind_group, &[]);
                rpass.set_bind_group(1, egui_window_bind_group, &[]);
                rpass.set_viewport(
                    x as f32,
                    y as f32,
                    inner_size.width as f32,
                    inner_size.height as f32,
                    0.0,
                    1.0,
                );
                rpass.draw(0..4, 0..1);

                // Decoration overlay: full outer viewport.
                if let Some(overlay) = decoration_overlay {
                    overlay.render(rpass, &pipeline);
                }
            })?;
        } else {
            // CPU (softbuffer) path — unchanged behaviour.
            let egui_cpu = self.egui_cpu.as_mut().unwrap();
            self.inner_window.draw_softbuffer(|buffer| {
                let mut egui_buffer = vec![0u32; (inner_size.width * inner_size.height) as usize];
                let mut buffer_ref = egui_software_backend::BufferMutRef::new(
                    bytemuck::cast_slice_mut(&mut egui_buffer),
                    inner_size.width as usize,
                    inner_size.height as usize,
                );

                let _ = egui_cpu.redraw(&mut buffer_ref, |ui| {
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
                            }
                            let response = ui.add(prompt);
                            response.request_focus();

                            ui.add_space(ui.available_height() - 50.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new("Submit")).clicked() {
                                    if let Err(err) = lua_event_tx.send(lua::Event::PromptSubmit {
                                        id,
                                        text: self.value.clone(),
                                    }) {
                                        tracing::error!("{err}");
                                    }
                                }
                            });
                        });
                    });
                });

                buffer.copy_from_u32_buf(&egui_buffer, inner_size.width, ox, oy);
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

pub struct ChoiceWindow {
    text: Option<String>,
    options: Vec<ChoiceWindowOption>,
    egui_cpu: Option<EguiCPUWindow>,
    egui_gpu: Option<EguiGpuRenderer>,
    decoration_overlay: Option<DecorationOverlay>,
    // Declared last so it drops last: egui's Arc<Window> clone is released first.
    pub inner_window: InnerWindow,
}

impl ChoiceWindow {
    pub fn new(
        inner_window: InnerWindow,
        text: Option<String>,
        options: Vec<ChoiceWindowOption>,
    ) -> Result<Self> {
        let (egui_cpu, egui_gpu, decoration_overlay) = if inner_window.is_gpu() {
            let inner_size = inner_window.inner_size();
            let egui_gpu = EguiGpuRenderer::new(
                inner_window.wgpu_state(),
                inner_window.window(),
                inner_size,
                inner_window.opacity,
                inner_window.premultiplied_alpha(),
                inner_window.force_opaque(),
                inner_window.background_color(),
                None,
            )?;
            let decoration_overlay = if inner_window.decorations() {
                let outer_size = inner_window.outer_size();
                Some(DecorationOverlay::new(
                    inner_window.wgpu_state(),
                    outer_size.width,
                    outer_size.height,
                    inner_window.premultiplied_alpha(),
                    inner_window.opacity,
                    inner_window.force_opaque(),
                ))
            } else {
                None
            };
            (None, Some(egui_gpu), decoration_overlay)
        } else {
            let egui_cpu = EguiCPUWindow::new(
                inner_window.window().clone(),
                inner_window.background_color(),
                None,
            )?;
            (Some(egui_cpu), None, None)
        };

        Ok(Self {
            text,
            options,
            egui_cpu,
            egui_gpu,
            decoration_overlay,
            inner_window,
        })
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        let translated = if self.inner_window.decorations() {
            Some(translate_event_position(
                event.clone(),
                self.inner_window.window().scale_factor(),
            ))
        } else {
            None
        };
        let translated_ref = translated.as_ref().unwrap_or(event);

        if let Some(egui_gpu) = &mut self.egui_gpu {
            egui_gpu.handle_event(self.inner_window.window(), translated_ref);
        } else if let Some(egui_cpu) = &mut self.egui_cpu {
            egui_cpu.handle_event(translated_ref);
        }
    }

    pub fn render(&mut self) -> Result<()> {
        self.inner_window.start_render()?;

        let id = self.inner_window.window().id();
        let lua_event_tx = self.inner_window.lua_event_tx().clone();
        let inner_size = self.inner_window.inner_size();
        let (ox, oy) = self.inner_window.inner_offset();
        let opacity = self.inner_window.opacity;

        if self.egui_gpu.is_some() {
            let wgpu_state = self.inner_window.wgpu_state().clone();
            let window = self.inner_window.window().clone();

            let text = self.text.clone();
            let options = self.options.clone();
            self.egui_gpu.as_mut().unwrap().render_to_texture(
                &wgpu_state,
                &window,
                inner_size,
                |ui| {
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.add_space(20.0);

                            if let Some(text) = &text {
                                ui.label(RichText::new(text).heading());
                            }

                            ui.add_space(ui.available_height() - 100.0);

                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center)
                                    .with_main_wrap(true)
                                    .with_main_align(egui::Align::Center)
                                    .with_main_justify(true),
                                |ui| {
                                    for option in &options {
                                        if ui.button(&option.label).clicked() {
                                            let _ = lua_event_tx.send(lua::Event::ChoiceSelect {
                                                id,
                                                option_id: option.id.clone(),
                                            });
                                        }
                                        ui.add_space(5.0);
                                    }
                                },
                            );
                        });
                    });
                },
            )?;

            let decoration_overlay = &mut self.decoration_overlay;
            self.inner_window.with_header_pixmap(|pixmap| {
                if let Some(overlay) = decoration_overlay {
                    overlay.upload_header(&wgpu_state.queue, pixmap, ox, ox);
                }
            });

            if let Some(overlay) = &self.decoration_overlay {
                overlay.set_opacity(&wgpu_state.queue, opacity);
            }
            self.egui_gpu
                .as_ref()
                .unwrap()
                .set_opacity(&wgpu_state.queue, opacity);

            let surface_format = self.inner_window.surface_format().unwrap();
            let pipeline = wgpu_state.get_pipeline(surface_format);
            let egui_bind_group = &self.egui_gpu.as_ref().unwrap().bind_group;
            let egui_window_bind_group = &self.egui_gpu.as_ref().unwrap().window_bind_group;
            let decoration_overlay = self.decoration_overlay.as_ref();

            self.inner_window.draw_wgpu(|rpass, x, y| {
                rpass.set_pipeline(&pipeline);
                rpass.set_bind_group(0, egui_bind_group, &[]);
                rpass.set_bind_group(1, egui_window_bind_group, &[]);
                rpass.set_viewport(
                    x as f32,
                    y as f32,
                    inner_size.width as f32,
                    inner_size.height as f32,
                    0.0,
                    1.0,
                );
                rpass.draw(0..4, 0..1);

                if let Some(overlay) = decoration_overlay {
                    overlay.render(rpass, &pipeline);
                }
            })?;
        } else {
            let egui_cpu = self.egui_cpu.as_mut().unwrap();
            self.inner_window.draw_softbuffer(|buffer| {
                let mut egui_buffer = vec![0u32; (inner_size.width * inner_size.height) as usize];
                let mut buffer_ref = egui_software_backend::BufferMutRef::new(
                    bytemuck::cast_slice_mut(&mut egui_buffer),
                    inner_size.width as usize,
                    inner_size.height as usize,
                );

                let _ = egui_cpu.redraw(&mut buffer_ref, |ui| {
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
                            );
                        });
                    });
                });

                buffer.copy_from_u32_buf(&egui_buffer, inner_size.width, ox, oy);
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

/// A window displaying static text. Unlike `PromptWindow`/`ChoiceWindow`, the egui content has
/// no interactive widgets, so (per egui's repaint-on-demand model) it only ever redraws when
/// `set_text()` is called or the window is first shown — the same one-shot behaviour as
/// `ImageWindow`.
pub struct TextWindow {
    text: String,
    style: TextStyle,
    egui_cpu: Option<EguiCPUWindow>,
    egui_gpu: Option<EguiGpuRenderer>,
    decoration_overlay: Option<DecorationOverlay>,
    // Declared last so it drops last: egui's Arc<Window> clone is released first.
    pub inner_window: InnerWindow,
}

impl TextWindow {
    pub fn new(inner_window: InnerWindow, text: String, style: TextStyle) -> Result<Self> {
        let font_definitions = text_font::build_font_definitions(style.font);

        // Unlike Prompt/Choice (which fall back to egui's opaque light theme), an unset
        // `background_color` means a fully transparent panel for text windows.
        let background_color = Some(inner_window.background_color().unwrap_or(lua::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }));

        let (egui_cpu, egui_gpu, decoration_overlay) = if inner_window.is_gpu() {
            let inner_size = inner_window.inner_size();
            let egui_gpu = EguiGpuRenderer::new(
                inner_window.wgpu_state(),
                inner_window.window(),
                inner_size,
                inner_window.opacity,
                inner_window.premultiplied_alpha(),
                inner_window.force_opaque(),
                background_color,
                font_definitions,
            )?;
            let decoration_overlay = if inner_window.decorations() {
                let outer_size = inner_window.outer_size();
                Some(DecorationOverlay::new(
                    inner_window.wgpu_state(),
                    outer_size.width,
                    outer_size.height,
                    inner_window.premultiplied_alpha(),
                    inner_window.opacity,
                    inner_window.force_opaque(),
                ))
            } else {
                None
            };
            (None, Some(egui_gpu), decoration_overlay)
        } else {
            let egui_cpu = EguiCPUWindow::new(
                inner_window.window().clone(),
                background_color,
                font_definitions,
            )?;
            (Some(egui_cpu), None, None)
        };

        Ok(Self {
            text,
            style,
            egui_cpu,
            egui_gpu,
            decoration_overlay,
            inner_window,
        })
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        let translated = if self.inner_window.decorations() {
            Some(translate_event_position(
                event.clone(),
                self.inner_window.window().scale_factor(),
            ))
        } else {
            None
        };
        let translated_ref = translated.as_ref().unwrap_or(event);

        if let Some(egui_gpu) = &mut self.egui_gpu {
            egui_gpu.handle_event(self.inner_window.window(), translated_ref);
        } else if let Some(egui_cpu) = &mut self.egui_cpu {
            egui_cpu.handle_event(translated_ref);
        }
    }

    pub fn render(&mut self) -> Result<()> {
        self.inner_window.start_render()?;

        let inner_size = self.inner_window.inner_size();
        let (ox, oy) = self.inner_window.inner_offset();
        let opacity = self.inner_window.opacity;

        if self.egui_gpu.is_some() {
            let wgpu_state = self.inner_window.wgpu_state().clone();
            let window = self.inner_window.window().clone();

            let text = self.text.clone();
            let style = self.style;
            self.egui_gpu.as_mut().unwrap().render_to_texture(
                &wgpu_state,
                &window,
                inner_size,
                |ui| paint_text(ui, &text, &style),
            )?;

            let decoration_overlay = &mut self.decoration_overlay;
            self.inner_window.with_header_pixmap(|pixmap| {
                if let Some(overlay) = decoration_overlay {
                    overlay.upload_header(&wgpu_state.queue, pixmap, ox, ox);
                }
            });

            if let Some(overlay) = &self.decoration_overlay {
                overlay.set_opacity(&wgpu_state.queue, opacity);
            }
            self.egui_gpu
                .as_ref()
                .unwrap()
                .set_opacity(&wgpu_state.queue, opacity);

            let surface_format = self.inner_window.surface_format().unwrap();
            let pipeline = wgpu_state.get_pipeline(surface_format);
            let egui_bind_group = &self.egui_gpu.as_ref().unwrap().bind_group;
            let egui_window_bind_group = &self.egui_gpu.as_ref().unwrap().window_bind_group;
            let decoration_overlay = self.decoration_overlay.as_ref();

            self.inner_window.draw_wgpu(|rpass, x, y| {
                rpass.set_pipeline(&pipeline);
                rpass.set_bind_group(0, egui_bind_group, &[]);
                rpass.set_bind_group(1, egui_window_bind_group, &[]);
                rpass.set_viewport(
                    x as f32,
                    y as f32,
                    inner_size.width as f32,
                    inner_size.height as f32,
                    0.0,
                    1.0,
                );
                rpass.draw(0..4, 0..1);

                if let Some(overlay) = decoration_overlay {
                    overlay.render(rpass, &pipeline);
                }
            })?;
        } else {
            let egui_cpu = self.egui_cpu.as_mut().unwrap();
            self.inner_window.draw_softbuffer(|buffer| {
                let mut egui_buffer = vec![0u32; (inner_size.width * inner_size.height) as usize];
                let mut buffer_ref = egui_software_backend::BufferMutRef::new(
                    bytemuck::cast_slice_mut(&mut egui_buffer),
                    inner_size.width as usize,
                    inner_size.height as usize,
                );

                let _ =
                    egui_cpu.redraw(&mut buffer_ref, |ui| paint_text(ui, &self.text, &self.style));

                buffer.copy_from_u32_buf(&egui_buffer, inner_size.width, ox, oy);
            })?;
        }

        Ok(())
    }

    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.inner_window.window().request_redraw();
    }
}

/// Paint `text` styled by `style`, centred vertically and horizontally-aligned per
/// `style.align` within the available area. `border_color`/`bold` are faked by repainting the
/// same laid-out `Galley` at small offsets before the crisp final draw, since egui has no native
/// stroked-text support.
fn paint_text(ui: &mut egui::Ui, text: &str, style: &TextStyle) {
    // Keep the panel's fill (driven by `background_color`, via `visuals.panel_fill`) but drop its
    // default margin: the window is sized to fit the text exactly (see `calculate_text_popup_size`),
    // so a margin here would make the available area smaller than what was measured, causing text
    // to wrap (or clip) when it shouldn't.
    let frame = egui::Frame::central_panel(ui.style()).inner_margin(0);

    egui::CentralPanel::default().frame(frame).show_inside(ui, |ui| {
        let available = ui.available_rect_before_wrap();
        // `style.font_size` is always `FontSize::Value` by the time it reaches here — percentage
        // sizes are resolved once in `App::spawn_text`, while the monitor is known — so the
        // argument here is unused.
        let font_size = style.font_size.to_pixels(0);
        let font_id = egui::FontId::new(font_size, text_font::font_family(style.font));
        let color = to_color32(style.color);

        // The window was sized (see `calculate_text_popup_size`) with `2 * border_width` of extra
        // room baked in so the border stroke (drawn offset from the text on every side) doesn't
        // get clipped by the window bounds. Mirror that padding here so wrapping/positioning
        // match what was actually measured.
        let border_width = if style.border_color.is_some() {
            style.border_width
        } else {
            0.0
        };
        let wrap_width = (available.width() - border_width * 2.0).max(0.0);

        let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat {
            font_id,
            color,
            ..Default::default()
        });
        job.wrap.max_width = wrap_width;
        job.halign = text_font::to_egui_align(style.align);

        let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));

        // `halign` positions each row *relative to x=0* (LEFT: [0,w], Center: [-w/2,w/2],
        // RIGHT: [-w,0]), not within `[0, wrap_width]` — so the anchor we paint at has to shift
        // depending on alignment, not just sit at the left edge. Left/right also inset by
        // `border_width` so the border has room on the outer edge (center is already symmetric).
        let pos_x = match style.align {
            lua::TextAlign::Left => available.left() + border_width,
            lua::TextAlign::Center => available.center().x,
            lua::TextAlign::Right => available.right() - border_width,
        };
        let pos = egui::pos2(pos_x, available.center().y - galley.size().y / 2.0);

        let painter = ui.painter();

        if let Some(border_color) = style.border_color {
            let border_color = to_color32(border_color);
            let count = text_font::outline_sample_count(style.border_width);
            for offset in text_font::outline_offsets(count) {
                painter.galley_with_override_text_color(
                    pos + offset * style.border_width,
                    galley.clone(),
                    border_color,
                );
            }
        }

        if style.bold {
            const BOLD_OFFSET: f32 = 0.6;
            for offset in text_font::outline_offsets(8) {
                painter.galley_with_override_text_color(
                    pos + offset * BOLD_OFFSET,
                    galley.clone(),
                    color,
                );
            }
        }

        painter.galley_with_override_text_color(pos, galley, color);
    });
}

fn to_color32(c: lua::Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
        (c.a * 255.0).round() as u8,
    )
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
        (
            y as f32 / 255.0,
            cb as f32 / 255.0 - 0.5,
            cr as f32 / 255.0 - 0.5,
        )
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
            let alpha = if packed_alpha {
                y_data[ay * y_stride + sx]
            } else {
                255
            };
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
            let alpha = if packed_alpha {
                y_data[ay * y_stride + sx]
            } else {
                255
            };
            dst[dy * dw + dx] = yuv_to_argb(y, cb, cr, alpha, full_range);
        }
    }
}
