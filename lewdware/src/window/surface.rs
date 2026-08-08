use std::sync::Arc;

use winit::window::Window;

use crate::window::theme::{BorderRing, ring_band};

pub enum Surface {
    Wgpu {
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
    },
    Softbuffer {
        _context: softbuffer::Context<Arc<Window>>,
        surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    },
    /// A texture standing in for a swapchain, so a frame can be rendered and read back without
    /// a window. Test-only; see [`RenderTarget::offscreen`](crate::window::RenderTarget).
    #[cfg(test)]
    Offscreen {
        texture: wgpu::Texture,
        format: wgpu::TextureFormat,
    },
}

impl Surface {
    pub fn is_gpu(&self) -> bool {
        #[cfg(test)]
        if matches!(self, Self::Offscreen { .. }) {
            return true;
        }

        matches!(self, Self::Wgpu { .. })
    }
}

/// A destination being composited into, for one frame.
///
/// Deliberately just a pixel slice and its dimensions rather than the softbuffer type: the
/// window's back buffer derefs to exactly this, and so does a plain `Vec` in a test. The pixel
/// arithmetic lives in the free functions below, so none of it needs a window either.
pub struct Buffer<'a> {
    pixels: &'a mut [u32],
    width: u32,
    height: u32,
}

impl<'a> Buffer<'a> {
    pub fn new(pixels: &'a mut [u32], width: u32, height: u32) -> Self {
        debug_assert_eq!(
            pixels.len(),
            (width * height) as usize,
            "buffer dimensions do not match its backing slice"
        );
        Self {
            pixels,
            width,
            height,
        }
    }

    pub fn copy_from_pixmap(&mut self, source: &tiny_skia::Pixmap, x: u32, y: u32) {
        blit_pixmap(self.pixels, self.width, source, x, y);
    }

    pub fn copy_from_u32_buf(&mut self, src: &[u32], src_width: u32, x: u32, y: u32) {
        blit_u32(self.pixels, self.width, src, src_width, x, y);
    }

    pub fn draw_border(&mut self, rings: &[BorderRing], thickness: u32) {
        stroke_border(self.pixels, self.width, self.height, rings, thickness);
    }
}

/// Softbuffer's `u32` is the *value* `0x00RRGGBB` (R in bits 16-23, G in 8-15, B in 0-7) — on a
/// little-endian machine that's byte order `[B, G, R, 0]`, not RGBA. So RGBA sources have to be
/// repacked per pixel rather than memcpy'd, or R and B end up swapped. Alpha is dropped: the
/// software path composites into an opaque window buffer.
fn pack_rgba(px: &[u8]) -> u32 {
    (px[0] as u32) << 16 | (px[1] as u32) << 8 | (px[2] as u32)
}

/// Write one run of RGBA bytes starting at `pixel_offset` (in pixels, not bytes).
fn blit_rgba_run(dst: &mut [u32], pixel_offset: usize, data: &[u8]) {
    for (i, px) in data.chunks_exact(4).enumerate() {
        dst[pixel_offset + i] = pack_rgba(px);
    }
}

/// Blit an RGBA pixmap into `dst` with its top-left corner at `(x, y)`.
fn blit_pixmap(dst: &mut [u32], dst_width: u32, source: &tiny_skia::Pixmap, x: u32, y: u32) {
    let offset = (y * dst_width) as usize;
    let src_data = source.data();

    // Contiguous when the source spans the full destination width — one run instead of per-row.
    if x == 0 && dst_width == source.width() {
        blit_rgba_run(dst, offset, src_data);
    } else {
        for (i, row) in src_data
            .chunks_exact(source.width() as usize * 4)
            .enumerate()
        {
            let index = offset + (dst_width * i as u32 + x) as usize;
            blit_rgba_run(dst, index, row);
        }
    }
}

/// Blit pixels already in softbuffer's layout into `dst` with their top-left corner at `(x, y)`.
fn blit_u32(dst: &mut [u32], dst_width: u32, src: &[u32], src_width: u32, x: u32, y: u32) {
    let offset = (y * dst_width) as usize;

    for (i, row) in src.chunks_exact(src_width as usize).enumerate() {
        let index = offset + (dst_width * i as u32 + x) as usize;
        dst[index..(index + row.len())].copy_from_slice(row);
    }
}

/// Stroke a themed border, `thickness` physical pixels deep, around the edge of `dst`.
///
/// `rings` are one logical pixel each, outermost first, and are distributed across `thickness` by
/// [`ring_band`] — so the whole reserved border is painted rather than just its outer pixel. That
/// matters above 1x, where the layout reserves `round(border_width * scale_factor)` pixels: only
/// the outermost used to be filled, leaving the rest of the reserved band unpainted between the
/// border and the content.
fn stroke_border(dst: &mut [u32], width: u32, height: u32, rings: &[BorderRing], thickness: u32) {
    if rings.is_empty() || thickness == 0 {
        return;
    }

    let w = width as usize;
    let h = height as usize;

    for (index, ring) in rings.iter().enumerate() {
        let (start, end) = ring_band(index, rings.len(), thickness);
        let top_left = pack_color(ring.top_left());
        let bottom_right = pack_color(ring.bottom_right());

        for inset in start as usize..end as usize {
            // Stop once the rings have met in the middle, which a thick border on a small window
            // can do.
            if inset * 2 >= w.min(h) {
                break;
            }

            for x in inset..w - inset {
                dst[inset * w + x] = top_left;
                dst[(h - 1 - inset) * w + x] = bottom_right;
            }
            for y in inset..h - inset {
                dst[y * w + inset] = top_left;
                dst[y * w + (w - 1 - inset)] = bottom_right;
            }
        }
    }
}

/// Pack a colour for the border.
///
/// Unlike [`pack_rgba`] this keeps alpha in the top byte. Softbuffer ignores it either way, but the
/// hardcoded border this replaced wrote `0xAARRGGBB`, and matching it bit for bit keeps the
/// existing pixel assertions meaningful.
fn pack_color(color: crate::lua::Color) -> u32 {
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (channel(color.a) << 24) | (channel(color.r) << 16) | (channel(color.g) << 8) | channel(color.b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::{IntSize, Pixmap};

    const BLACK: u32 = 0xFF00_0000;

    /// A pixmap whose every pixel is the given RGBA bytes.
    fn pixmap(width: u32, height: u32, rgba: [u8; 4]) -> Pixmap {
        let data: Vec<u8> = rgba
            .iter()
            .copied()
            .cycle()
            .take((width * height * 4) as usize)
            .collect();
        Pixmap::from_vec(data, IntSize::from_wh(width, height).unwrap()).unwrap()
    }

    #[test]
    fn rgba_is_repacked_not_memcpied() {
        // R=0x11 G=0x22 B=0x33 must land as 0x00112233, not byte-copied as 0x44332211.
        let mut dst = vec![0u32; 1];
        blit_rgba_run(&mut dst, 0, &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(dst[0], 0x0011_2233);
    }

    #[test]
    fn full_width_pixmap_takes_the_contiguous_path() {
        let mut dst = vec![0u32; 3 * 2];
        blit_pixmap(&mut dst, 3, &pixmap(3, 2, [0xAA, 0xBB, 0xCC, 0xFF]), 0, 0);
        assert_eq!(dst, vec![0x00AA_BBCC; 6]);
    }

    #[test]
    fn offset_pixmap_lands_at_its_origin() {
        // 4x4 destination, 2x2 source at (1, 1): only that square is touched.
        let mut dst = vec![0u32; 4 * 4];
        blit_pixmap(&mut dst, 4, &pixmap(2, 2, [0x10, 0x20, 0x30, 0xFF]), 1, 1);

        let v = 0x0010_2030;
        #[rustfmt::skip]
        let expected = vec![
            0, 0, 0, 0,
            0, v, v, 0,
            0, v, v, 0,
            0, 0, 0, 0,
        ];
        assert_eq!(dst, expected);
    }

    /// A source narrower than the destination must not run off the end of its row: regression
    /// guard for the row-stride arithmetic in the non-contiguous path.
    #[test]
    fn narrow_pixmap_at_x_zero_does_not_bleed_across_rows() {
        let mut dst = vec![0u32; 4 * 2];
        blit_pixmap(&mut dst, 4, &pixmap(2, 2, [0xFF, 0xFF, 0xFF, 0xFF]), 0, 0);

        let v = 0x00FF_FFFF;
        assert_eq!(dst, vec![v, v, 0, 0, v, v, 0, 0]);
    }

    #[test]
    fn u32_blit_lands_at_its_origin() {
        let mut dst = vec![0u32; 4 * 3];
        blit_u32(&mut dst, 4, &[1, 2, 3, 4], 2, 1, 1);

        #[rustfmt::skip]
        let expected = vec![
            0, 0, 0, 0,
            0, 1, 2, 0,
            0, 3, 4, 0,
        ];
        assert_eq!(dst, expected);
    }

    const BLACK_RING: [BorderRing; 1] = [BorderRing::Uniform(crate::lua::Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    })];

    #[test]
    fn border_covers_the_perimeter_and_nothing_else() {
        let mut dst = vec![0u32; 4 * 3];
        stroke_border(&mut dst, 4, 3, &BLACK_RING, 1);

        #[rustfmt::skip]
        let expected = vec![
            BLACK, BLACK, BLACK, BLACK,
            BLACK, 0,     0,     BLACK,
            BLACK, BLACK, BLACK, BLACK,
        ];
        assert_eq!(dst, expected);
    }

    #[test]
    fn border_on_a_minimal_buffer_does_not_panic() {
        let mut dst = vec![0u32; 1];
        stroke_border(&mut dst, 1, 1, &BLACK_RING, 1);
        assert_eq!(dst, vec![BLACK]);
    }

    /// A single logical ring scaled up covers the whole physical thickness — the HiDPI case the
    /// old hardcoded one-pixel stroke left partly unpainted.
    #[test]
    fn one_ring_fills_the_whole_physical_thickness() {
        let mut dst = vec![0u32; 6 * 6];
        stroke_border(&mut dst, 6, 6, &BLACK_RING, 2);

        #[rustfmt::skip]
        let expected = vec![
            BLACK, BLACK, BLACK, BLACK, BLACK, BLACK,
            BLACK, BLACK, BLACK, BLACK, BLACK, BLACK,
            BLACK, BLACK, 0,     0,     BLACK, BLACK,
            BLACK, BLACK, 0,     0,     BLACK, BLACK,
            BLACK, BLACK, BLACK, BLACK, BLACK, BLACK,
            BLACK, BLACK, BLACK, BLACK, BLACK, BLACK,
        ];
        assert_eq!(dst, expected);
    }

    /// Rings thicker than the window they surround must stop when they meet rather than index
    /// past the buffer.
    #[test]
    fn a_border_thicker_than_the_window_does_not_panic() {
        let mut dst = vec![0u32; 3 * 3];
        stroke_border(&mut dst, 3, 3, &BLACK_RING, 8);
        assert!(dst.iter().all(|&p| p == BLACK));
    }
}
