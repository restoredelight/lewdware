use std::sync::Arc;

use tiny_skia::{Color, Paint, PathBuilder, PixmapMut, Rect, Stroke, Transform};
use winit::window::Window;

pub enum Surface<'a> {
    Wgpu {
        surface: wgpu::Surface<'a>,
        surface_config: wgpu::SurfaceConfiguration,
    },
    Softbuffer {
        _context: softbuffer::Context<Arc<Window>>,
        surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    },
}

impl<'a> Surface<'a> {
    pub fn is_gpu(&self) -> bool {
        matches!(self, Self::Wgpu { .. })
    }
}

pub enum Buffer<'a> {
    Pixmap(PixmapMut<'a>),
    Softbuffer(softbuffer::Buffer<'a, Arc<Window>, Arc<Window>>),
}

impl<'a> Buffer<'a> {
    pub fn copy_from_slice(&mut self, offset: usize, data: &[u8]) {
        match self {
            Buffer::Pixmap(pixmap) => {
                let dest = pixmap.data_mut();
                dest[offset..offset + data.len()].copy_from_slice(data);
            }
            Buffer::Softbuffer(buffer) => {
                let dest = bytemuck::cast_slice_mut(buffer);
                dest[offset..offset + data.len()].copy_from_slice(data);
            }
        }
    }

    pub fn copy_from_pixmap(&mut self, source: &tiny_skia::Pixmap, x: u32, y: u32) {
        let dst_width = self.width();
        let offset = (y * dst_width) as usize;
        let src_data = source.data();

        if x == 0 && dst_width == source.width() {
            self.copy_from_slice(offset, src_data);
        } else {
            for (i, row) in src_data
                .chunks_exact(source.width() as usize * 4)
                .enumerate()
            {
                let index = offset + (dst_width * i as u32 + x) as usize;

                self.copy_from_slice(index * 4, row);
            }
        }
    }

    pub fn copy_from_u32_buf(&mut self, src: &[u32], width: u32, x: u32, y: u32) {
        let offset = (y * self.width()) as usize;
        let dst_width = self.width();

        match self {
            Buffer::Pixmap(pixmap) => {
                let data = pixmap.data_mut();
                let src_bytes = bytemuck::cast_slice::<u32, u8>(src);
                let row_bytes = width as usize * 4;
                for (i, row) in src_bytes.chunks_exact(row_bytes).enumerate() {
                    let index = offset + (dst_width * i as u32 + x) as usize;
                    let byte_index = index * 4;
                    data[byte_index..(byte_index + row.len())].copy_from_slice(row);
                }
            }
            Buffer::Softbuffer(buffer) => {
                for (i, row) in src.chunks_exact(width as usize).enumerate() {
                    let index = offset + (dst_width * i as u32 + x) as usize;

                    buffer[index..(index + row.len())].copy_from_slice(row);
                }
            }
        }
    }

    pub fn draw_border(&mut self) {
        match self {
            Buffer::Pixmap(pixmap) => {
                let border = PathBuilder::from_rect(
                    Rect::from_xywh(0.0, 0.0, pixmap.width() as f32, pixmap.height() as f32)
                        .unwrap(),
                );
                let mut paint = Paint::default();
                paint.set_color(Color::BLACK);
                pixmap.stroke_path(
                    &border,
                    &paint,
                    &Stroke::default(),
                    Transform::default(),
                    None,
                );
            }
            Buffer::Softbuffer(buffer) => {
                let black = Color::BLACK.to_color_u8();
                let color = ((black.alpha() as u32) << 24)
                    | ((black.red() as u32) << 16)
                    | ((black.green() as u32) << 8)
                    | (black.blue() as u32);
                let width = buffer.width().get() as usize;
                let height = buffer.height().get() as usize;

                for i in 0..width {
                    buffer[i] = color;
                    buffer[width * (height - 1) + i] = color;
                }

                for i in 0..height {
                    buffer[i * width] = color;
                    buffer[i * width + (width - 1)] = color;
                }
            }
        }
    }

    fn width(&self) -> u32 {
        match self {
            Buffer::Pixmap(p) => p.width(),
            Buffer::Softbuffer(s) => s.width().get(),
        }
    }
}
