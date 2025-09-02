pub trait ColorBuffer {
    fn set_pixel(&mut self, x: usize, y: usize, color: u32) -> bool;
    fn width(&self) -> usize;
    fn height(&self) -> usize;
}

pub struct SoftBufferWrapper<'a> {
    buffer: &'a mut [u32],
    width: usize,
    height: usize,
}

impl<'a> SoftBufferWrapper<'a> {
    pub fn new(buffer: &'a mut [u32], width: usize, height: usize) -> Self {
        Self {
            buffer,
            width,
            height,
        }
    }
}

impl<'a> ColorBuffer for SoftBufferWrapper<'a> {
    fn set_pixel(&mut self, x: usize, y: usize, color: u32) -> bool {
        if x < self.width && y < self.height {
            let idx = y * self.width + x;
            if idx < self.buffer.len() {
                self.buffer[idx] = color;
                return true;
            }
        }
        false
    }

    fn width(&self) -> usize {
        self.width
    }
    fn height(&self) -> usize {
        self.height
    }
}

// Implementation for pixels crate
pub struct PixelsWrapper<'a, 'b> {
    pixels: &'a mut pixels::Pixels<'b>,
}

impl<'a, 'b> PixelsWrapper<'a, 'b> {
    pub fn new(pixels: &'a mut pixels::Pixels<'b>) -> Self {
        Self { pixels }
    }
}

impl<'a, 'b> ColorBuffer for PixelsWrapper<'a, 'b> {
    fn set_pixel(&mut self, x: usize, y: usize, color: u32) -> bool {
        let width = self.pixels.texture().width() as usize;
        let height = self.pixels.texture().height() as usize;

        if x < width && y < height {
            let frame = self.pixels.frame_mut();
            let idx = (y * width + x) * 4; // RGBA format

            if idx + 3 < frame.len() {
                frame[idx] = ((color >> 16) & 0xFF) as u8; // R
                frame[idx + 1] = ((color >> 8) & 0xFF) as u8; // G
                frame[idx + 2] = (color & 0xFF) as u8; // B
                frame[idx + 3] = ((color >> 24) & 0xFF) as u8; // A
                return true;
            }
        }
        false
    }

    fn width(&self) -> usize {
        self.pixels.texture().width() as usize
    }
    fn height(&self) -> usize {
        self.pixels.texture().height() as usize
    }
}

pub fn draw_close_button(
    buffer: &mut impl ColorBuffer,
    scale_factor: f64,
    cursor_over_button: bool,
) {
    let button_size = to_physical_size(20, scale_factor).max(16);
    let margin = to_physical_size(6, scale_factor).max(4);

    let button_x = buffer.width().saturating_sub(button_size + margin);
    let button_y = margin;

    draw_button_background(buffer, button_x, button_y, button_size, cursor_over_button);
    draw_x_pattern(buffer, button_x, button_y, button_size, cursor_over_button);
}

fn draw_button_background(
    buffer: &mut impl ColorBuffer,
    x: usize,
    y: usize,
    size: usize,
    cursor_over_button: bool,
) {
    let bg_color = if cursor_over_button {
        0xFF3A5A8A
    } else {
        0xFF2A4A7A
    };
    let border_light = 0xFF6A8ABA;
    let border_dark = 0xFF1A2A3A;

    // Draw main background
    for px in 1..(size - 1) {
        for py in 1..(size - 1) {
            buffer.set_pixel(x + px, y + py, bg_color);
        }
    }

    // Draw borders
    if !cursor_over_button {
        // Raised button borders
        for i in 0..size {
            buffer.set_pixel(x + i, y, border_light); // Top
            buffer.set_pixel(x, y + i, border_light); // Left
            buffer.set_pixel(x + i, y + size - 1, border_dark); // Bottom
            buffer.set_pixel(x + size - 1, y + i, border_dark); // Right
        }
    } else {
        // Pressed button borders (inverted)
        for i in 0..size {
            buffer.set_pixel(x + i, y, border_dark); // Top
            buffer.set_pixel(x, y + i, border_dark); // Left
            buffer.set_pixel(x + i, y + size - 1, border_light); // Bottom
            buffer.set_pixel(x + size - 1, y + i, border_light); // Right
        }
    }
}

fn draw_x_pattern(
    buffer: &mut impl ColorBuffer,
    button_x: usize,
    button_y: usize,
    button_size: usize,
    cursor_over_button: bool,
) {
    let x_color = if cursor_over_button {
        0xFFFFFFFF
    } else {
        0xFFE0E0E0
    };

    let x_size = if button_size <= 16 {
        // At 100% scale (16px button): use 8px X
        button_size / 2
    } else {
        // At 200% scale (40px button): use ~12px X
        button_size * 3 / 10
    }
    .max(6); // Minimum 6px X

    let block_size = 2;

    let drawable_size = button_size - 2;
    let x_start_x = button_x + 1 + (drawable_size - x_size) / 2;
    let x_start_y = button_y + 1 + (drawable_size - x_size) / 2;

    for i in 0..x_size {
        let px1 = x_start_x + i;
        let py1 = x_start_y + i;

        let px2 = x_start_x + i;
        let py2 = x_start_y + x_size - 1 - i;

        for dx in 0..block_size {
            for dy in 0..block_size {
                buffer.set_pixel(px1 + dx, py1 + dy, x_color);
                buffer.set_pixel(px2 + dx, py2 + dy, x_color);
            }
        }
    }
}

pub fn is_over_close_button(x: f64, y: f64, window_width: u32, scale_factor: f64) -> bool {
    let button_size = to_physical_size(20, scale_factor).max(16);
    let margin = to_physical_size(6, scale_factor).max(4);

    let button_left = window_width as usize - button_size - margin;
    let button_top = margin;
    let button_right = window_width as usize - margin;
    let button_bottom = margin + button_size;

    x >= button_left as f64
        && x <= button_right as f64
        && y >= button_top as f64
        && y <= button_bottom as f64
}

fn to_physical_size(size: usize, scale_factor: f64) -> usize {
    (size as f64 * scale_factor) as usize
}
