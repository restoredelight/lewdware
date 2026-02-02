use std::sync::Arc;

use tiny_skia::{Color, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};
use winit::{dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize}, window::Window};

pub struct Header {
    window: Arc<Window>,
    hover: bool,
    clicked: bool,
    size: LogicalSize<u32>,
    physical_size: PhysicalSize<u32>,
    scale_factor: f64,
}

pub const HEADER_HEIGHT: u32 = 24;

impl Header {
    pub fn new(window: Arc<Window>, window_size: PhysicalSize<u32>, scale_factor: f64) -> Self {
        let header_size = LogicalSize::new(window_size.to_logical(scale_factor).width, HEADER_HEIGHT);
        let physical_size = header_size.to_physical(scale_factor);

        Self {
            window,
            hover: false,
            clicked: false,
            physical_size,
            size: header_size,
            scale_factor,
        }
    }

    pub fn draw(
        &self,
    ) -> Pixmap {
        let mut pixmap = Pixmap::new(self.physical_size.width, self.physical_size.height).unwrap();

        let grey = Color::from_rgba8(227, 229, 231, 255);

        let mut paint = Paint::default();
        paint.set_color(grey.clone());

        let transform = Transform::from_scale(self.scale_factor as f32, self.scale_factor as f32);

        let header_rect =
            Rect::from_xywh(0.0, 0.0, self.size.width as f32, self.size.height as f32).unwrap();

        pixmap.fill_rect(header_rect, &paint, transform.clone(), None);

        let button_size = (self.size.height as f32) * 1.5;

        let close_rect = Rect::from_xywh(
            self.size.width as f32 - button_size,
            0.0,
            button_size,
            self.size.height as f32,
        ).unwrap();

        match (self.clicked, self.hover) {
            (true, _) => {
                paint.set_color(Color::from_rgba(0.9, 0.0, 0.0, 1.0).unwrap());
            },
            (false, true) => {
                paint.set_color(Color::from_rgba(1.0, 0.0, 0.0, 1.0).unwrap());
            }
            (false, false) => {
                paint.set_color(grey.clone());
            },
        };

        pixmap.fill_rect(close_rect, &paint, transform.clone(), None);

        if self.clicked || self.hover {
            paint.set_color(Color::WHITE);
        } else {
            paint.set_color(Color::BLACK);
        }

        let cross_middle_x = self.size.width as f32 - (button_size / 2.0);
        let cross_middle_y = (self.size.height as f32) / 2.0;

        let cross_offset = (self.size.height as f32) / 6.0;

        let mut left_line = PathBuilder::new();
        left_line.move_to(cross_middle_x - cross_offset, cross_middle_y - cross_offset);
        left_line.line_to(cross_middle_x + cross_offset, cross_middle_y + cross_offset);

        let path = left_line.finish().unwrap();

        pixmap.stroke_path(&path, &paint, &Stroke::default(), transform.clone(), None);

        let mut right_line = PathBuilder::new();
        right_line.move_to(cross_middle_x - cross_offset, cross_middle_y + cross_offset);
        right_line.line_to(cross_middle_x + cross_offset, cross_middle_y - cross_offset);

        let path = right_line.finish().unwrap();

        pixmap.stroke_path(&path, &paint, &Stroke::default(), transform.clone(), None);

        pixmap
    }

    fn over_close_button(&self, position: PhysicalPosition<f64>) -> bool {
        let position: LogicalPosition<u32> = position.to_logical(self.scale_factor);
        let button_size = (self.size.height as f32) * 1.5;
        position.x + button_size as u32 >= self.size.width && position.y <= self.size.height
    }

    pub fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        let over_close_button = self.over_close_button(position);

        if !self.hover && over_close_button {
            self.hover = true;
            self.window.request_redraw();
        } else if self.hover && !over_close_button {
            self.hover = false;
            self.window.request_redraw();
        }
    }

    pub fn handle_cursor_left(&mut self) {
        if self.hover || self.clicked {
            self.hover = false;
            self.clicked = false;
            self.window.request_redraw();
        }
    }

    pub fn handle_mouse_down(&mut self) {
        if self.hover {
            if !self.clicked {
                self.clicked = true;
                self.window.request_redraw();
            }
        }
    }

    pub fn handle_mouse_up(&mut self) -> bool {
        if self.hover && self.clicked {
            return true;
        }

        if self.clicked {
            self.clicked = false;
            self.window.request_redraw();
        }

        false
    }

    // pub fn render_softbuffer(&self, buffer: &mut softbuffer::Buffer<'_, Arc<Window>, Arc<Window>>) {
    //     let pixmap = self.create_pixmap();
    //     let data = pixmap.data();
    //
    //     for index in 0..(self.physical_size.width * self.physical_size.height) as usize {
    //         let r = data[index * 4] as u32;
    //         let g = data[index * 4 + 1] as u32;
    //         let b = data[index * 4 + 2] as u32;
    //         let a = data[index * 4 + 3] as u32;
    //
    //         buffer[index] = (a << 24) | (r << 16) | (g << 8) | b;
    //     }
    // }
    //
    // pub fn render_pixels(&self, buffer: &mut [u8]) {
    //     let pixmap = self.create_pixmap();
    //
    //     buffer.copy_from_slice(pixmap.data());
    // }
}
