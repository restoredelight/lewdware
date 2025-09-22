use ab_glyph::{Font, FontArc, GlyphId, PxScale, ScaleFont, point};

/// A trait that allows us to work with both `pixels` and `softbuffer` buffers.
pub trait ColorBuffer {
    fn set_pixel(&mut self, x: usize, y: usize, color: u32) -> bool;
    fn get_pixel(&self, x: usize, y: usize) -> u32;
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

    fn get_pixel(&self, x: usize, y: usize) -> u32 {
        self.buffer[y * self.width + x]
    }

    fn width(&self) -> usize {
        self.width
    }
    fn height(&self) -> usize {
        self.height
    }
}

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

    fn get_pixel(&self, x: usize, y: usize) -> u32 {
        let width = self.pixels.texture().width() as usize;

        let frame = self.pixels.frame();
        let idx = (y * width + x) * 4;

        ((frame[idx + 3] as u32) << 24)
            | ((frame[idx] as u32) << 16)
            | ((frame[idx + 1] as u32) << 8)
            | (frame[idx + 2] as u32)
    }

    fn width(&self) -> usize {
        self.pixels.texture().width() as usize
    }
    fn height(&self) -> usize {
        self.pixels.texture().height() as usize
    }
}

/// Draw a close button on a buffer. This is done by manually editing the pixels.
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
        for i in 0..size {
            buffer.set_pixel(x + i, y, border_light);
            buffer.set_pixel(x, y + i, border_light);
            buffer.set_pixel(x + i, y + size - 1, border_dark);
            buffer.set_pixel(x + size - 1, y + i, border_dark);
        }
    } else {
        // Inverted borders
        for i in 0..size {
            buffer.set_pixel(x + i, y, border_dark);
            buffer.set_pixel(x, y + i, border_dark);
            buffer.set_pixel(x + i, y + size - 1, border_light);
            buffer.set_pixel(x + size - 1, y + i, border_light);
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
        button_size / 2
    } else {
        button_size * 3 / 10
    }
    .max(6);

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

fn blend_into_pixel(pixel: u32, src: u32, coverage: f32) -> Option<u32> {
    if coverage <= 0.0 {
        return None;
    }
    let src_a = ((src >> 24) & 0xFF) as f32 / 255.0 * coverage;
    if src_a <= 0.0 {
        return None;
    }
    let src_r = ((src >> 16) & 0xFF) as f32;
    let src_g = ((src >> 8) & 0xFF) as f32;
    let src_b = (src & 0xFF) as f32;

    let dst_a = ((pixel >> 24) & 0xFF) as f32 / 255.0;
    let dst_r = ((pixel >> 16) & 0xFF) as f32;
    let dst_g = ((pixel >> 8) & 0xFF) as f32;
    let dst_b = (pixel & 0xFF) as f32;

    let out_a = src_a + dst_a * (1.0 - src_a);
    let out_r = (src_r * src_a + dst_r * dst_a * (1.0 - src_a)) / out_a.max(1e-6);
    let out_g = (src_g * src_a + dst_g * dst_a * (1.0 - src_a)) / out_a.max(1e-6);
    let out_b = (src_b * src_a + dst_b * dst_a * (1.0 - src_a)) / out_a.max(1e-6);

    let ia = (out_a * 255.0).round().clamp(0.0, 255.0) as u32;
    let ir = (out_r).round().clamp(0.0, 255.0) as u32;
    let ig = (out_g).round().clamp(0.0, 255.0) as u32;
    let ib = (out_b).round().clamp(0.0, 255.0) as u32;

    Some((ia << 24) | (ir << 16) | (ig << 8) | ib)
}

/// Draw `text` into `frame` (ARGB32) at approximate baseline (x,y).
/// `baseline_y` is the baseline y coordinate in pixel space (not top-left).
/// `color` is ARGB u32 (eg. 0xFFFFFFFF for white opaque).
pub fn draw_text_ab_glyph(
    buffer: &mut impl ColorBuffer,
    fb_w: usize,
    fb_h: usize,
    font: &FontArc,
    size_px: f32,
    mut pen_x: f32,
    baseline_y: f32,
    text: &str,
    fill_color: u32,
) {
    let scale = PxScale::from(size_px);
    // scaled_font provides scaled metrics, h_advance, kern in pixel units
    let scaled_font = font.as_scaled(scale);

    let mut prev_gid: Option<GlyphId> = None;

    for ch in text.chars() {
        let gid = scaled_font.glyph_id(ch);

        // apply kerning (pixel-scaled)
        if let Some(prev) = prev_gid {
            pen_x += scaled_font.kern(prev, gid);
        }

        // create a positioned glyph at (pen_x, baseline_y)
        // GlyphId::with_scale_and_position(scale, point(x,y)) constructs a Glyph
        let glyph = gid.with_scale_and_position(scale, point(pen_x, baseline_y));

        // get an outline for rasterization (pixel coverage callback)
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds(); // bounds.min.x/min.y may be negative
            // draw provides (gx, gy, coverage) for each pixel in glyph bbox
            outlined.draw(|gx, gy, coverage| {
                // compute destination pixel coordinates
                let px = (bounds.min.x as i32).saturating_add(gx as i32);
                let py = (bounds.min.y as i32).saturating_add(gy as i32);

                if px < 0 || py < 0 {
                    return;
                }
                let px = px as usize;
                let py = py as usize;
                if px >= fb_w || py >= fb_h {
                    return;
                }

                if let Some(pixel) =
                    blend_into_pixel(buffer.get_pixel(px, py), fill_color, coverage)
                {
                    buffer.set_pixel(px, py, pixel);
                }
            });
        }

        // advance pen by glyph advance (pixel scaled)
        let advance = scaled_font.h_advance(gid);
        pen_x += advance;
        prev_gid = Some(gid);
    }
}

pub fn draw_text_ab_glyph_with_outline(
    buffer: &mut impl ColorBuffer,
    fb_w: usize,
    fb_h: usize,
    font: &FontArc,
    size_px: f32,
    mut pen_x: f32,
    baseline_y: f32,
    text: &str,
    fill_color: u32,
    outline_color: Option<u32>,
    outline_width: f32,
) {
    let scale = PxScale::from(size_px);
    let scaled_font = font.as_scaled(scale);
    let mut prev_gid: Option<GlyphId> = None;

    // First pass: draw outline if specified
    // if let Some(outline_col) = outline_color {
    //     if outline_width > 0.0 {
    //         let mut outline_pen_x = pen_x;
    //         let mut outline_prev_gid: Option<GlyphId> = None;
    //
    //         for ch in text.chars() {
    //             let gid = scaled_font.glyph_id(ch);
    //
    //             if let Some(prev) = outline_prev_gid {
    //                 outline_pen_x += scaled_font.kern(prev, gid);
    //             }
    //
    //             let glyph = gid.with_scale_and_position(scale, point(outline_pen_x, baseline_y));
    //
    //             if let Some(outlined) = font.outline_glyph(glyph) {
    //                 // Draw outline by sampling multiple offset positions
    //                 draw_glyph_outline(buffer, fb_w, fb_h, &outlined, outline_col, outline_width);
    //             }
    //
    //             let advance = scaled_font.h_advance(gid);
    //             outline_pen_x += advance;
    //             outline_prev_gid = Some(gid);
    //         }
    //     }
    // }

    // Second pass: draw main text
    for ch in text.chars() {
        let gid = scaled_font.glyph_id(ch);

        if let Some(prev) = prev_gid {
            pen_x += scaled_font.kern(prev, gid);
        }

        let glyph = gid.with_scale_and_position(scale, point(pen_x, baseline_y));

        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let px = (bounds.min.x as i32).saturating_add(gx as i32);
                let py = (bounds.min.y as i32).saturating_add(gy as i32);

                if px < 0 || py < 0 {
                    return;
                }
                let px = px as usize;
                let py = py as usize;
                if px >= fb_w || py >= fb_h {
                    return;
                }

                if let Some(pixel) =
                    blend_into_pixel(buffer.get_pixel(px, py), fill_color, coverage)
                {
                    buffer.set_pixel(px, py, pixel);
                }
            });
        }

        let advance = scaled_font.h_advance(gid);
        pen_x += advance;
        prev_gid = Some(gid);
    }
}

fn draw_glyph_outline(
    buffer: &mut impl ColorBuffer,
    fb_w: usize,
    fb_h: usize,
    outlined_glyph: &ab_glyph::OutlinedGlyph,
    outline_color: u32,
    outline_width: f32,
) {
    let bounds = outlined_glyph.px_bounds();
    let outline_radius = outline_width.ceil() as i32;

    // Create offset pattern for outline sampling
    let mut offsets = Vec::new();
    for dy in -outline_radius..=outline_radius {
        for dx in -outline_radius..=outline_radius {
            let distance = ((dx * dx + dy * dy) as f32).sqrt();
            if distance <= outline_width && distance > 0.0 {
                offsets.push((
                    dx as f32,
                    dy as f32,
                    1.0 - (distance / outline_width).min(1.0),
                ));
            }
        }
    }

    // For each pixel in the glyph bounds
    let min_x = (bounds.min.x.floor() as i32 - outline_radius).max(0) as usize;
    let max_x = (bounds.max.x.ceil() as i32 + outline_radius).min(fb_w as i32) as usize;
    let min_y = (bounds.min.y.floor() as i32 - outline_radius).max(0) as usize;
    let max_y = (bounds.max.y.ceil() as i32 + outline_radius).min(fb_h as i32) as usize;

    for py in min_y..max_y {
        for px in min_x..max_x {
            let mut max_outline_coverage = 0.0f32;

            // Sample the glyph at offset positions to create outline effect
            for &(dx, dy, weight) in &offsets {
                let sample_x = px as f32 + dx;
                let sample_y = py as f32 + dy;

                // Check if this sample position would have glyph coverage
                let glyph_x = sample_x - bounds.min.x;
                let glyph_y = sample_y - bounds.min.y;

                if glyph_x >= 0.0
                    && glyph_y >= 0.0
                    && glyph_x < bounds.width()
                    && glyph_y < bounds.height()
                {
                    // Sample the glyph coverage at this position
                    let mut sample_coverage = 0.0;
                    outlined_glyph.draw(|gx, gy, coverage| {
                        let test_px = bounds.min.x as i32 + gx as i32;
                        let test_py = bounds.min.y as i32 + gy as i32;

                        if (test_px as f32 - sample_x).abs() < 0.5
                            && (test_py as f32 - sample_y).abs() < 0.5
                        {
                            sample_coverage = coverage;
                        }
                    });

                    if sample_coverage > 0.0 {
                        max_outline_coverage = max_outline_coverage.max(sample_coverage * weight);
                    }
                }
            }

            if max_outline_coverage > 0.0 {
                // Check that we're not drawing over the main glyph area
                let mut main_coverage = 0.0;
                outlined_glyph.draw(|gx, gy, coverage| {
                    let test_px = bounds.min.x as i32 + gx as i32;
                    let test_py = bounds.min.y as i32 + gy as i32;

                    if test_px == px as i32 && test_py == py as i32 {
                        main_coverage = coverage;
                    }
                });

                // Only draw outline where main glyph coverage is low
                if main_coverage < 0.1 {
                    if let Some(pixel) = blend_into_pixel(
                        buffer.get_pixel(px, py),
                        outline_color,
                        max_outline_coverage * 0.8, // Reduce outline opacity slightly
                    ) {
                        buffer.set_pixel(px, py, pixel);
                    }
                }
            }
        }
    }
}
