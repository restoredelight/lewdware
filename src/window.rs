use std::{
    num::NonZeroU32,
    sync::{atomic::{AtomicBool, Ordering}, Arc},
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use egui_wgpu::wgpu;
use image::DynamicImage;
use pixels::{Pixels, PixelsBuilder, SurfaceTexture};
use tempfile::NamedTempFile;
use winit::{dpi::PhysicalPosition, event::WindowEvent, window::Window};

use crate::{
    buffer::{PixelsWrapper, SoftBufferWrapper, draw_close_button, is_over_close_button},
    egui::EguiWindow,
    media::Video,
    video::VideoDecoder,
};

pub struct ImageWindow {
    pub window: Arc<Window>,
    pub created: Instant,
    image: Option<DynamicImage>,
    close_button: bool,
    cursor_over_button: bool,
    _context: softbuffer::Context<Arc<Window>>,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    width: u32,
    height: u32,
}

impl ImageWindow {
    pub fn new(window: Window, image: DynamicImage, close_button: bool) -> Result<Self> {
        let window = Arc::new(window);

        let context = softbuffer::Context::new(window.clone()).map_err(|err| anyhow!("{}", err))?;
        let surface =
            softbuffer::Surface::new(&context, window.clone()).map_err(|err| anyhow!("{}", err))?;

        let width = image.width();
        let height = image.height();

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
        })
    }

    pub fn draw(&mut self) -> Result<()> {
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

            let rgba_image = image.to_rgba8();

            for (i, pixel) in rgba_image.pixels().enumerate() {
                let r = pixel[0] as u32;
                let g = pixel[1] as u32;
                let b = pixel[2] as u32;
                let a = pixel[3] as u32;

                buffer[i] = (a << 24) | (r << 16) | (g << 8) | b;
            }

            buffer
        } else {
            if !self.close_button {
                return Ok(());
            }

            self.surface
                .buffer_mut()
                .map_err(|err| anyhow!("{}", err))?
        };

        if self.close_button {
            draw_close_button(
                &mut SoftBufferWrapper::new(&mut buffer, self.width as usize, self.height as usize),
                self.window.scale_factor(),
                self.cursor_over_button,
            );
        }

        buffer.present().map_err(|err| anyhow!("{}", err))?;

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
        if self.cursor_over_button {
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

    // fn is_over_close_button(&self, x: f64, y: f64) -> bool {
    //     let scale_factor = self.window.scale_factor();
    //     let size = self.window.inner_size();
    //     let button_size = to_physical_size(9, scale_factor);
    //     let margin = to_physical_size(5, scale_factor);
    //
    //     let button_left = size.width as usize - button_size - margin;
    //     let button_top = margin;
    //     let button_right = size.width as usize - margin;
    //     let button_bottom = margin + button_size;
    //
    //     x >= button_left as f64
    //         && x <= button_right as f64
    //         && y >= button_top as f64
    //         && y <= button_bottom as f64
    // }

    // fn is_over_close_button(&self, x: f64, y: f64) -> bool {
    //     let scale_factor = self.window.scale_factor();
    //     let size = self.window.inner_size();
    //     let button_size = to_physical_size(16, scale_factor).max(12); // Match new button size
    //     let margin = to_physical_size(8, scale_factor).max(6);
    //
    //     let button_left = size.width as usize - button_size - margin;
    //     let button_top = margin;
    //     let button_right = size.width as usize - margin;
    //     let button_bottom = margin + button_size;
    //
    //     x >= button_left as f64
    //         && x <= button_right as f64
    //         && y >= button_top as f64
    //         && y <= button_bottom as f64
    // }

    // fn draw_close_button(&self, buffer: &mut [u32], width: usize) {
    //     let scale_factor = self.window.scale_factor();
    //     let radius = to_physical_size(8, scale_factor);
    //     let margin = to_physical_size(5, scale_factor);
    //
    //     let middle_x = width.saturating_sub(radius + margin);
    //     let middle_y = margin + radius;
    //
    //     let color = 0xFF124CEA;
    //
    //     draw_square_around(
    //         buffer,
    //         to_physical_size(4, scale_factor),
    //         middle_x,
    //         middle_y,
    //         color,
    //         width,
    //     );
    //
    //     let offset = to_physical_size(4, scale_factor);
    //     let smaller_size = to_physical_size(2, scale_factor);
    //
    //     for (x, y) in [
    //         (middle_x + offset, middle_y + offset),
    //         (middle_x + offset, middle_y - offset),
    //         (middle_x - offset, middle_y + offset),
    //         (middle_x - offset, middle_y - offset),
    //     ] {
    //         draw_square_around(buffer, smaller_size, x, y, color, width);
    //     }
    //
    //     let offset = to_physical_size(7, scale_factor);
    //
    //     for (x, y) in [
    //         (middle_x + offset, middle_y + offset),
    //         (middle_x + offset, middle_y - offset),
    //         (middle_x - offset, middle_y + offset),
    //         (middle_x - offset, middle_y - offset),
    //     ] {
    //         draw_square_around(buffer, smaller_size, x, y, color, width);
    //     }
    // }
}

// fn draw_close_button_(
//     buffer: &mut [u32],
//     width: usize,
//     height: usize,
//     scale_factor: f64,
//     cursor_over_button: bool,
// ) {
//     let button_size = to_physical_size(16, scale_factor).max(12);
//     let margin = to_physical_size(8, scale_factor).max(6);
//
//     let button_x = width.saturating_sub(button_size + margin);
//     let button_y = margin;
//
//     // Draw button background with retro border
//     draw_button_background(
//         buffer,
//         button_x,
//         button_y,
//         button_size,
//         width,
//         height,
//         cursor_over_button,
//     );
//
//     // Draw the X pattern
//     draw_x_pattern(
//         buffer,
//         button_x,
//         button_y,
//         button_size,
//         width,
//         height,
//         cursor_over_button,
//     );
// }
//
// fn draw_button_background(
//     buffer: &mut [u32],
//     x: usize,
//     y: usize,
//     size: usize,
//     width: usize,
//     height: usize,
//     cursor_over_button: bool,
// ) {
//     let bg_color = if cursor_over_button {
//         0xFF3A5A8A // Lighter blue on hover
//     } else {
//         0xFF2A4A7A // Medium blue background
//     };
//     let border_light = 0xFF6A8ABA; // Lighter border (top-left)
//     let border_dark = 0xFF1A2A3A; // Dark border (bottom-right)
//
//     // Draw main background (leave 1px border on all sides)
//     for px in 1..(size - 1) {
//         for py in 1..(size - 1) {
//             if x + px < width && y + py < height {
//                 let idx = (y + py) * width + (x + px);
//                 if idx < buffer.len() {
//                     buffer[idx] = bg_color;
//                 }
//             }
//         }
//     }
//
//     // Draw 3D-style borders more carefully
//     if !cursor_over_button {
//         // Raised button borders
//         // Top border (light)
//         for i in 0..size {
//             if x + i < width && y < height {
//                 let idx = y * width + (x + i);
//                 if idx < buffer.len() {
//                     buffer[idx] = border_light;
//                 }
//             }
//         }
//
//         // Left border (light)
//         for i in 0..size {
//             if x < width && y + i < height {
//                 let idx = (y + i) * width + x;
//                 if idx < buffer.len() {
//                     buffer[idx] = border_light;
//                 }
//             }
//         }
//
//         // Bottom border (dark)
//         for i in 0..size {
//             if x + i < width && y + size - 1 < height {
//                 let idx = (y + size - 1) * width + (x + i);
//                 if idx < buffer.len() {
//                     buffer[idx] = border_dark;
//                 }
//             }
//         }
//
//         // Right border (dark)
//         for i in 0..size {
//             if x + size - 1 < width && y + i < height {
//                 let idx = (y + i) * width + (x + size - 1);
//                 if idx < buffer.len() {
//                     buffer[idx] = border_dark;
//                 }
//             }
//         }
//     } else {
//         // Pressed button borders (inverted)
//         // Top border (dark)
//         for i in 0..size {
//             if x + i < width && y < height {
//                 let idx = y * width + (x + i);
//                 if idx < buffer.len() {
//                     buffer[idx] = border_dark;
//                 }
//             }
//         }
//
//         // Left border (dark)
//         for i in 0..size {
//             if x < width && y + i < height {
//                 let idx = (y + i) * width + x;
//                 if idx < buffer.len() {
//                     buffer[idx] = border_dark;
//                 }
//             }
//         }
//
//         // Bottom border (light)
//         for i in 0..size {
//             if x + i < width && y + size - 1 < height {
//                 let idx = (y + size - 1) * width + (x + i);
//                 if idx < buffer.len() {
//                     buffer[idx] = border_light;
//                 }
//             }
//         }
//
//         // Right border (light)
//         for i in 0..size {
//             if x + size - 1 < width && y + i < height {
//                 let idx = (y + i) * width + (x + size - 1);
//                 if idx < buffer.len() {
//                     buffer[idx] = border_light;
//                 }
//             }
//         }
//     }
// }
//
// fn draw_x_pattern(
//     buffer: &mut [u32],
//     button_x: usize,
//     button_y: usize,
//     button_size: usize,
//     width: usize,
//     height: usize,
//     cursor_over_button: bool,
// ) {
//     let x_color = if cursor_over_button {
//         0xFFFFFFFF // White on hover
//     } else {
//         0xFFE0E0E0 // Light gray normally
//     };
//
//     // Adjust X size based on button size to prevent it from being too small
//     let x_size = if button_size < 14 {
//         button_size - 4 // For very small buttons, use most of the space
//     } else {
//         button_size / 3 // Normal size for larger buttons
//     }
//     .max(4); // Ensure minimum X size
//
//     // For very small buttons, use single pixels instead of 2x2 blocks
//     let block_size = if button_size < 14 { 1 } else { 2 };
//
//     // Calculate start positions to center the X properly
//     // Use the actual drawable area (button_size - 2 for borders) for centering
//     let drawable_size = button_size - 2; // Account for 1px borders
//     let x_start_x = button_x + 1 + (drawable_size - x_size) / 2;
//     let x_start_y = button_y + 1 + (drawable_size - x_size) / 2;
//
//     // Draw thicker, more pixelated X
//     for i in 0..x_size {
//         // Main diagonal (\)
//         let px1 = x_start_x + i;
//         let py1 = x_start_y + i;
//
//         // Anti-diagonal (/)
//         let px2 = x_start_x + i;
//         let py2 = x_start_y + x_size - 1 - i;
//
//         // Draw blocks (1x1 for small buttons, 2x2 for larger ones)
//         for dx in 0..block_size {
//             for dy in 0..block_size {
//                 // Main diagonal block
//                 if px1 + dx < width && py1 + dy < height {
//                     let idx1 = (py1 + dy) * width + (px1 + dx);
//                     if idx1 < buffer.len() {
//                         buffer[idx1] = x_color;
//                     }
//                 }
//
//                 // Anti-diagonal block
//                 if px2 + dx < width && py2 + dy < height {
//                     let idx2 = (py2 + dy) * width + (px2 + dx);
//                     if idx2 < buffer.len() {
//                         buffer[idx2] = x_color;
//                     }
//                 }
//             }
//         }
//     }
// }
//
// fn draw_square_around(
//     buffer: &mut [u32],
//     radius: usize,
//     middle_x: usize,
//     middle_y: usize,
//     color: u32,
//     width: usize,
// ) {
//     for x in (middle_x - radius)..(middle_x + radius) {
//         for y in (middle_y - radius)..(middle_y + radius) {
//             let idx = y * width + x;
//
//             buffer[idx] = color;
//         }
//     }
// }
//
// fn to_physical_size(size: usize, scale_factor: f64) -> usize {
//     (size as f64 * scale_factor) as usize
// }

pub struct VideoWindow<'a> {
    pub window: Arc<Window>,
    pub created: Instant,
    pixels: Pixels<'a>,
    decoder: VideoDecoder,
    last_frame_time: Instant,
    duration: Option<Duration>,
    _tempfile: NamedTempFile,
    close_button: bool,
    close_button_changed: bool,
    cursor_over_button: bool,
    width: u32,
    height: u32,
    closed: Arc<AtomicBool>,
}

impl<'a> VideoWindow<'a> {
    pub fn new(
        wgpu_instance: &wgpu::Instance,
        window: Window,
        video: Video,
        tempfile: NamedTempFile,
        close_button: bool,
    ) -> anyhow::Result<Self> {
        let window = Arc::new(window);
        let decoder = VideoDecoder::new(tempfile.path().to_str().unwrap(), &video)?;

        let width = video.width as u32;
        let height = video.height as u32;

        let surface_texture = SurfaceTexture::new(width, height, window.clone());

        let pixels = PixelsBuilder::new(width, height, surface_texture)
            .build_with_instance(wgpu_instance)?;

        let closed = Arc::new(AtomicBool::new(false));
        let closed_clone = closed.clone();

        pixels.device().on_uncaptured_error(Box::new(move |err| {
            eprintln!("wgpu error: {}", err);
            closed_clone.store(true, Ordering::Relaxed);
        }));

        Ok(Self {
            window,
            pixels,
            decoder,
            last_frame_time: Instant::now(),
            duration: None,
            _tempfile: tempfile,
            created: Instant::now(),
            close_button,
            close_button_changed: false,
            cursor_over_button: false,
            width,
            height,
            closed,
        })
    }

    pub fn update(&mut self) -> anyhow::Result<()> {
        if (self.closed.load(Ordering::Relaxed)) {
            return Ok(());
        }

        let mut render = false;

        if self
            .duration
            .is_none_or(|duration| self.last_frame_time.elapsed() >= duration)
        {
            let frame = self.decoder.next_frame()?;
            self.decoder
                .copy_frame(&frame.frame, self.pixels.frame_mut());
            // self.pixels.render()?;
            self.duration = Some(frame.duration);
            self.last_frame_time = Instant::now();
            render = true;
        }

        if render || (self.close_button && self.close_button_changed) {
            draw_close_button(
                &mut PixelsWrapper::new(&mut self.pixels),
                self.window.scale_factor(),
                self.cursor_over_button,
            );

            render = true;
        }

        if render {
            self.pixels.render()?;
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
        if self.cursor_over_button {
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

    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

pub struct PromptWindow<'a> {
    pub window: Arc<Window>,
    egui_window: EguiWindow<'a>,
    prompt: String,
    user_input: String,
    closed: bool,
}

impl<'a> PromptWindow<'a> {
    pub fn new(wgpu_instance: &wgpu::Instance, window: Window, prompt: String) -> Result<Self> {
        let window = Arc::new(window);
        let window_clone = window.clone();

        Ok(Self {
            window,
            egui_window: EguiWindow::new(wgpu_instance, window_clone)?,
            prompt,
            user_input: String::new(),
            closed: false,
        })
    }

    pub fn handle_event(&mut self, event: &WindowEvent) {
        if self.egui_window.handle_event(event) {
            self.window.request_redraw();
        }
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
        self.closed || self.egui_window.closed()
    }
}
