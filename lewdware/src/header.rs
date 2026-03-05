use std::{sync::{Arc, LazyLock}};

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use tiny_skia::{Color, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};
use winit::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize},
    window::Window,
};

pub struct Header {
    window: Arc<Window>,
    hover: bool,
    clicked: bool,
    needs_redraw: bool,
    text_changed: bool,
    background_drawn: bool,
    pixmap: Pixmap,
    size: LogicalSize<u32>,
    physical_size: PhysicalSize<u32>,
    scale_factor: f64,
    title: Option<String>,
    closeable: bool,
}

pub const HEADER_HEIGHT: u32 = 24;

static FONT: LazyLock<Option<FontArc>> = LazyLock::new(|| {
    let font_definitions = egui::FontDefinitions::default();
    let font_data = font_definitions
        .font_data
        .get("Ubuntu-Light");

    if let Some(data) = font_data {
        FontArc::try_from_vec(data.font.to_vec()).ok()
    } else {
        None
    }
});

impl Header {
    pub fn new(
        window: Arc<Window>,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
        title: Option<String>,
        closeable: bool,
    ) -> Self {
        let header_size =
            LogicalSize::new(window_size.to_logical(scale_factor).width, HEADER_HEIGHT);
        let physical_size = header_size.to_physical(scale_factor);

        // Test Text
        let pixmap = Pixmap::new(physical_size.width, physical_size.height).unwrap();

        Self {
            window,
            hover: false,
            clicked: false,
            needs_redraw: true,
            text_changed: title.is_some(),
            background_drawn: false,
            pixmap,
            physical_size,
            size: header_size,
            scale_factor,
            title,
            closeable,
        }
    }

    fn draw_background(&mut self) {
        let grey = Color::from_rgba8(227, 229, 231, 255);

        let mut paint = Paint::default();
        paint.set_color(grey.clone());

        let transform = Transform::from_scale(self.scale_factor as f32, self.scale_factor as f32);

        let header_rect =
            Rect::from_xywh(0.0, 0.0, self.size.width as f32, self.size.height as f32).unwrap();

        self.pixmap.fill_rect(header_rect, &paint, transform.clone(), None);

        self.background_drawn = true;
    }

    fn draw_text(&mut self) {
        if let (Some(text), Some(font)) = (&self.title, &*FONT) {
            let font_size = 14.0 * self.scale_factor as f32;
            let scale = PxScale::from(font_size);
            let scaled_font = font.as_scaled(scale);

            let text_width = text
                .chars()
                .map(|c| scaled_font.glyph_id(c))
                .fold(0.0, |acc, id| acc + scaled_font.h_advance(id));

            let padding = 10.0 * self.scale_factor as f32;
            let safe_right = if self.closeable {
                let physical_button_size = self.physical_size.height as f32 * 1.5;
                self.physical_size.width as f32 - physical_button_size
            } else {
                self.physical_size.width as f32 - 10.0 * self.scale_factor as f32
            };

            // We center the text unless it overflows, in which case we align left.
            let centered_x = (self.physical_size.width as f32 - text_width) / 2.0;
            let mut pen_x = if centered_x >= padding && (centered_x + text_width) <= safe_right {
                centered_x
            } else {
                padding
            };

            // pen_y is the baseline. Center the cap-height/ascent in the header.
            let pen_y = (self.physical_size.height as f32 / 2.0) + (scaled_font.ascent() / 2.0)
                - (1.0 * self.scale_factor as f32);

            let mut text_pixmap =
                Pixmap::new(self.physical_size.width, self.physical_size.height).unwrap();
            let pixmap_width = text_pixmap.width() as i32;
            let pixmap_height = text_pixmap.height() as i32;
            let data = text_pixmap.data_mut();

            for c in text.chars() {
                let glyph_id = scaled_font.glyph_id(c);
                let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(pen_x, pen_y));

                if let Some(outlined) = font.outline_glyph(glyph) {
                    let bounds = outlined.px_bounds();

                    outlined.draw(|x, y, c| {
                        let px = bounds.min.x as i32 + x as i32;
                        let py = bounds.min.y as i32 + y as i32;

                        if px >= 0 && px < pixmap_width && py >= 0 && py < pixmap_height {
                            let idx = ((py * pixmap_width + px) * 4) as usize;
                            let alpha = (c * 255.0) as u8;

                            data[idx] = 0;
                            data[idx + 1] = 0;
                            data[idx + 2] = 0;
                            data[idx + 3] = alpha;
                        }
                    });
                }

                pen_x += scaled_font.h_advance(glyph_id);
            }

            self.pixmap.draw_pixmap(
                0,
                0,
                text_pixmap.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                Transform::identity(),
                None,
            );

            self.text_changed = false;
        }
    }

    fn draw_close_button(&mut self) {
        let transform = Transform::from_scale(self.scale_factor as f32, self.scale_factor as f32);

        let button_size = (self.size.height as f32) * 1.5;

        let close_rect = Rect::from_xywh(
            self.size.width as f32 - button_size,
            0.0,
            button_size,
            self.size.height as f32,
        )
        .unwrap();

        let mut paint = Paint::default();
        let grey = Color::from_rgba8(227, 229, 231, 255);

        match (self.clicked, self.hover) {
            (true, _) => {
                paint.set_color(Color::from_rgba(0.9, 0.0, 0.0, 1.0).unwrap());
            }
            (false, true) => {
                paint.set_color(Color::from_rgba(1.0, 0.0, 0.0, 1.0).unwrap());
            }
            (false, false) => {
                paint.set_color(grey.clone());
            }
        };

        self.pixmap.fill_rect(close_rect, &paint, transform.clone(), None);

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

        self.pixmap.stroke_path(&path, &paint, &Stroke::default(), transform.clone(), None);

        let mut right_line = PathBuilder::new();
        right_line.move_to(cross_middle_x - cross_offset, cross_middle_y + cross_offset);
        right_line.line_to(cross_middle_x + cross_offset, cross_middle_y - cross_offset);

        let path = right_line.finish().unwrap();

        self.pixmap.stroke_path(&path, &paint, &Stroke::default(), transform.clone(), None);
    }

    pub fn draw(&mut self) -> Option<&Pixmap> {
        if !self.needs_redraw {
            return None;
        }

        if !self.background_drawn || self.text_changed {
            self.draw_background();
        }

        if self.text_changed {
            self.draw_text();
        }

        if self.closeable {
            self.draw_close_button();
        }

        Some(&self.pixmap)
    }

    fn over_close_button(&self, position: PhysicalPosition<f64>) -> bool {
        let position: LogicalPosition<u32> = position.to_logical(self.scale_factor);
        let button_size = (self.size.height as f32) * 1.5;
        position.x + button_size as u32 >= self.size.width && position.y <= self.size.height
    }

    fn request_redraw(&mut self) {
        self.needs_redraw = true;
        self.window.request_redraw();
    }

    pub fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if self.closeable {
            let over_close_button = self.over_close_button(position);

            if !self.hover && over_close_button {
                self.hover = true;
                self.request_redraw();
            } else if self.hover && !over_close_button {
                self.hover = false;
                self.request_redraw();
            }
        }
    }

    pub fn handle_cursor_left(&mut self) {
        if self.closeable {
            if self.hover || self.clicked {
                self.hover = false;
                self.clicked = false;
                self.request_redraw();
            }
        }
    }

    pub fn handle_mouse_down(&mut self) {
        if self.closeable {
            if self.hover {
                if !self.clicked {
                    self.clicked = true;
                    self.request_redraw();
                }
            }
        }
    }

    pub fn handle_mouse_up(&mut self) -> bool {
        if self.closeable {
            if self.hover && self.clicked {
                return true;
            }

            if self.clicked {
                self.clicked = false;
                self.request_redraw();
            }

            false
        } else {
            false
        }
    }

    pub fn set_title(&mut self, text: Option<String>) {
        self.title = text;
        self.text_changed = true;
        self.request_redraw();
    }
}
