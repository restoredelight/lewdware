use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use tiny_skia::{PathBuilder, Pixmap, PixmapMut, Rect, Paint, Color, Stroke, Transform};
use winit::window::Window;

use crate::{video::VideoFrame, window::video_renderer::VideoRenderer};

pub enum Surface<'a> {
    Wgpu {
        surface: wgpu::Surface<'a>,
        surface_config: wgpu::SurfaceConfiguration,
        frame_buffer: Vec<u8>,
        texture: wgpu::Texture,
        bind_group: wgpu::BindGroup,
        opacity_buffer: wgpu::Buffer,
        window_bind_group: wgpu::BindGroup,
        video_renderer: Option<VideoRenderer>,
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

    pub fn buffer(&mut self) -> Result<Buffer<'_>> {
        match self {
            Surface::Wgpu {
                frame_buffer,
                surface_config,
                ..
            } => {
                let dest = PixmapMut::from_bytes(
                    frame_buffer,
                    surface_config.width,
                    surface_config.height,
                )
                .context("Invalid pixmap size")?;

                Ok(Buffer::Pixmap(dest))
            }
            Surface::Softbuffer { _context, surface } => {
                let buffer = surface.buffer_mut().map_err(|err| anyhow!("{err}"))?;

                Ok(Buffer::Softbuffer(buffer))
            }
        }
    }
}

pub enum Buffer<'a> {
    Pixmap(PixmapMut<'a>),
    Softbuffer(softbuffer::Buffer<'a, Arc<Window>, Arc<Window>>),
}

impl<'a> Buffer<'a> {
    fn copy_from_slice(&mut self, start: usize, src: &[u8]) {
        match self {
            Buffer::Pixmap(pixmap) => {
                let start = start * 4;
                pixmap.data_mut()[start..(start + src.len())].copy_from_slice(src);
            }
            Buffer::Softbuffer(buffer) => {
                for (index, pixel) in src.chunks_exact(4).enumerate() {
                    let r = pixel[0] as u32;
                    let g = pixel[1] as u32;
                    let b = pixel[2] as u32;
                    let a = pixel[3] as u32;

                    buffer[start + index] = (a << 24) | (r << 16) | (g << 8) | b;
                }
            }
        }
    }

    pub fn copy_from_pixmap(&mut self, source: &Pixmap, x: u32, y: u32) {
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

                self.copy_from_slice(index, row);
            }
        }
    }

    pub fn copy_from_frame(&mut self, frame: &VideoFrame, x: u32, y: u32) {
        let frame_width = frame.frame.width() as usize;
        let frame_height = frame.frame.height() as usize;
        let line_size = frame.frame.stride(0); // Bytes per row
        let data = frame.frame.data(0);

        let copy_width = frame_width.min(self.width().saturating_sub(x) as usize);
        let copy_height = frame_height.min(self.height().saturating_sub(y) as usize);

        let dst_width = self.width();
        let offset = (y * dst_width) as usize;

        for row_index in 0..copy_height {
            let src_start = row_index * line_size;
            let src_end = src_start + copy_width * 4;

            let index = offset + (dst_width * row_index as u32 + x) as usize;

            self.copy_from_slice(index, &data[src_start..src_end]);
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
            Buffer::Pixmap(pixmap) => pixmap.width(),
            Buffer::Softbuffer(buffer) => buffer.width().get(),
        }
    }

    fn height(&self) -> u32 {
        match self {
            Buffer::Pixmap(pixmap) => pixmap.height(),
            Buffer::Softbuffer(buffer) => buffer.height().get(),
        }
    }
}
