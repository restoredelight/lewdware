//! Decoded YUV frame -> straight-alpha BGRA, on the decode thread. New code: the engine converts
//! YUV/NV12 to RGB in a wgpu shader (`lewdware/src/shaders/yuv.wgsl`, `nv12.wgsl`), which gpui
//! gives us no way to plug in, so here it is done by swscale on the CPU.
//!
//! Colour handling mirrors the engine's shader: BT.709 matrix, range from the stream (packed-alpha
//! videos are always full range). gpui samples its atlas as sRGB-encoded bytes, which is what
//! swscale produces, so there is no equivalent of the shader's `gamma_decode`.
//!
//! Packed alpha (`shared/src/encode/transcode.rs`): the coded frame is twice the display height,
//! colour in the top half and alpha stored as full-range luma in the bottom half. Chroma of the
//! bottom half is unused. The colour half goes through swscale by handing it the top `h` rows of
//! the planes; alpha is the bottom half's Y plane, copied (or scaled with a second gray8 swscale
//! context when downscaling) into the A byte. gpui's sprite shader wants straight (not
//! premultiplied) alpha, which is what this produces.

use std::ptr;

use anyhow::{Result, bail};
use ffmpeg_next::{ffi, frame::Video};

pub struct Converter {
    color: *mut ffi::SwsContext,
    alpha: *mut ffi::SwsContext,
    key: Key,
    alpha_scratch: Vec<u8>,
}

// The contexts are only ever used from the decode thread that owns the Converter.
unsafe impl Send for Converter {}

/// Bytes past the last row that swscale may write into.
const SLACK: usize = 64;

/// A row length rounded up to 64 bytes, the widest SIMD store swscale makes.
fn padded(row: usize) -> usize {
    row.div_ceil(64) * 64
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Key {
    src_format: i32,
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    full_range: bool,
    packed_alpha: bool,
}

impl Converter {
    pub fn new() -> Self {
        Self {
            color: ptr::null_mut(),
            alpha: ptr::null_mut(),
            key: Key {
                src_format: -1,
                src_w: 0,
                src_h: 0,
                dst_w: 0,
                dst_h: 0,
                full_range: false,
                packed_alpha: false,
            },
            alpha_scratch: Vec::new(),
        }
    }

    /// Convert `frame` to BGRA at `dst_w` x `dst_h` (the display size, for packed alpha the
    /// colour half's). Returns a tightly packed buffer, `dst_w * 4` bytes per row.
    pub fn convert(
        &mut self,
        frame: &Video,
        packed_alpha: bool,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<Vec<u8>> {
        let raw = unsafe { &*frame.as_ptr() };
        let mut src_format = raw.format;
        let src_w = raw.width as u32;
        let coded_h = raw.height as u32;
        let src_h = if packed_alpha { coded_h / 2 } else { coded_h };

        // The deprecated "J" formats are full-range variants; swscale wants the plain format
        // plus an explicit range.
        let mut full_range = packed_alpha || raw.color_range == ffi::AVColorRange::AVCOL_RANGE_JPEG;
        if src_format == ffi::AVPixelFormat::AV_PIX_FMT_YUVJ420P as i32 {
            src_format = ffi::AVPixelFormat::AV_PIX_FMT_YUV420P as i32;
            full_range = true;
        }

        let planar_420 = src_format == ffi::AVPixelFormat::AV_PIX_FMT_YUV420P as i32;
        if packed_alpha && !planar_420 {
            bail!("packed alpha needs yuv420p, got format {src_format}");
        }

        let key = Key {
            src_format,
            src_w,
            src_h,
            dst_w,
            dst_h,
            full_range,
            packed_alpha,
        };
        if key != self.key {
            self.rebuild(key)?;
        }

        // swscale's SIMD paths write past the end of a row (8 bytes over a 333px BGRA row, seen
        // under valgrind), so a tight buffer has its last row run off the allocation and corrupt
        // the heap. Give it padded rows and slack at the end, then pack the rows tight.
        let tight = dst_w as usize * 4;
        let stride = padded(tight);
        let mut out = vec![0u8; stride * dst_h as usize + SLACK];
        let dst_stride = [stride as i32, 0, 0, 0];
        let dst_data = [
            out.as_mut_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        ];

        unsafe {
            let src_data = [raw.data[0], raw.data[1], raw.data[2], raw.data[3]];
            let src_stride = [
                raw.linesize[0],
                raw.linesize[1],
                raw.linesize[2],
                raw.linesize[3],
            ];
            // Top `src_h` rows only: for packed alpha that's exactly the colour half.
            let rows = ffi::sws_scale(
                self.color,
                src_data.as_ptr() as *const *const u8,
                src_stride.as_ptr(),
                0,
                src_h as i32,
                dst_data.as_ptr(),
                dst_stride.as_ptr(),
            );
            if rows <= 0 {
                bail!("sws_scale failed: {rows}");
            }

            if packed_alpha {
                let y_stride = raw.linesize[0] as usize;
                let alpha_top = raw.data[0].add(y_stride * src_h as usize);

                if dst_w == src_w && dst_h == src_h {
                    for row in 0..dst_h as usize {
                        let src = std::slice::from_raw_parts(
                            alpha_top.add(row * y_stride),
                            dst_w as usize,
                        );
                        let dst = &mut out[row * stride..][..tight];
                        for (px, &a) in dst.chunks_exact_mut(4).zip(src) {
                            px[3] = a;
                        }
                    }
                } else {
                    let a_stride = padded(dst_w as usize);
                    self.alpha_scratch
                        .resize(a_stride * dst_h as usize + SLACK, 0);
                    let a_src = [
                        alpha_top as *const u8,
                        ptr::null(),
                        ptr::null(),
                        ptr::null(),
                    ];
                    let a_src_stride = [y_stride as i32, 0, 0, 0];
                    let a_dst = [
                        self.alpha_scratch.as_mut_ptr(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    ];
                    let a_dst_stride = [a_stride as i32, 0, 0, 0];
                    ffi::sws_scale(
                        self.alpha,
                        a_src.as_ptr(),
                        a_src_stride.as_ptr(),
                        0,
                        src_h as i32,
                        a_dst.as_ptr(),
                        a_dst_stride.as_ptr(),
                    );
                    for row in 0..dst_h as usize {
                        let alpha = &self.alpha_scratch[row * a_stride..][..dst_w as usize];
                        let dst = &mut out[row * stride..][..tight];
                        for (px, &a) in dst.chunks_exact_mut(4).zip(alpha) {
                            px[3] = a;
                        }
                    }
                }
            }
        }

        if stride != tight {
            for row in 1..dst_h as usize {
                out.copy_within(row * stride..row * stride + tight, row * tight);
            }
        }
        out.truncate(tight * dst_h as usize);
        Ok(out)
    }

    fn rebuild(&mut self, key: Key) -> Result<()> {
        self.free();
        unsafe {
            let src_fmt: ffi::AVPixelFormat = std::mem::transmute(key.src_format);
            // Bilinear: the same filter the engine's sampler and gpui's sprite sampler use. Only
            // matters when downscaling (`--scale ffmpeg`).
            let flags = ffi::SwsFlags::SWS_BILINEAR as i32;
            self.color = ffi::sws_getContext(
                key.src_w as i32,
                key.src_h as i32,
                src_fmt,
                key.dst_w as i32,
                key.dst_h as i32,
                ffi::AVPixelFormat::AV_PIX_FMT_BGRA,
                flags,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            if self.color.is_null() {
                bail!("sws_getContext failed for {key:?}");
            }

            // BT.709 in, matching the engine's shader; full-range RGB out.
            let bt709 = ffi::sws_getCoefficients(ffi::SWS_CS_ITU709 as i32);
            ffi::sws_setColorspaceDetails(
                self.color,
                bt709,
                key.full_range as i32,
                bt709,
                1,
                0,
                1 << 16,
                1 << 16,
            );

            if key.packed_alpha && (key.dst_w != key.src_w || key.dst_h != key.src_h) {
                self.alpha = ffi::sws_getContext(
                    key.src_w as i32,
                    key.src_h as i32,
                    ffi::AVPixelFormat::AV_PIX_FMT_GRAY8,
                    key.dst_w as i32,
                    key.dst_h as i32,
                    ffi::AVPixelFormat::AV_PIX_FMT_GRAY8,
                    flags,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                if self.alpha.is_null() {
                    bail!("sws_getContext (alpha) failed");
                }
                // Alpha is full range in and out: no expansion.
                let coeffs = ffi::sws_getCoefficients(ffi::SWS_CS_DEFAULT as i32);
                ffi::sws_setColorspaceDetails(
                    self.alpha,
                    coeffs,
                    1,
                    coeffs,
                    1,
                    0,
                    1 << 16,
                    1 << 16,
                );
            }
        }
        self.key = key;
        Ok(())
    }

    fn free(&mut self) {
        unsafe {
            if !self.color.is_null() {
                ffi::sws_freeContext(self.color);
                self.color = ptr::null_mut();
            }
            if !self.alpha.is_null() {
                ffi::sws_freeContext(self.alpha);
                self.alpha = ptr::null_mut();
            }
        }
    }
}

impl Drop for Converter {
    fn drop(&mut self) {
        self.free();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffmpeg_next::format::Pixel;

    fn yuv_frame(width: u32, height: u32, y: u8, u: u8, v: u8) -> Video {
        let mut frame = Video::new(Pixel::YUV420P, width, height);
        frame.data_mut(0).fill(y);
        frame.data_mut(1).fill(u);
        frame.data_mut(2).fill(v);
        frame
    }

    #[test]
    fn limited_range_red_is_opaque_bgra() {
        // BT.709 limited-range red. RenderImage expects BGRA even though its container is
        // named RgbaImage; a swapped channel would turn this blue in the GPUI preview.
        let frame = yuv_frame(16, 16, 63, 102, 240);
        let pixels = Converter::new().convert(&frame, false, 16, 16).unwrap();
        for pixel in pixels.chunks_exact(4) {
            assert!(pixel[0] < 5 && pixel[1] < 5 && pixel[2] > 245, "{pixel:?}");
            assert_eq!(pixel[3], 255);
        }
    }

    #[test]
    fn packed_alpha_uses_bottom_luma_with_its_stride() {
        let mut frame = yuv_frame(16, 32, 255, 128, 128);
        let stride = frame.stride(0);
        for row in 0..16 {
            frame.data_mut(0)[(16 + row) * stride..(16 + row) * stride + 16].fill((row * 17) as u8);
        }
        let pixels = Converter::new().convert(&frame, true, 16, 16).unwrap();
        for (row, pixels) in pixels.chunks_exact(16 * 4).enumerate() {
            for pixel in pixels.chunks_exact(4) {
                assert_eq!(pixel[3], (row * 17) as u8);
                assert!(pixel[..3].iter().all(|&channel| channel >= 254));
            }
        }
    }

    #[test]
    fn resized_alpha_keeps_full_range_and_converter_can_change_sizes() {
        let mut converter = Converter::new();
        for alpha in [0, 64, 128, 255] {
            let mut frame = yuv_frame(16, 32, 255, 128, 128);
            let stride = frame.stride(0);
            frame.data_mut(0)[16 * stride..32 * stride].fill(alpha);
            for size in [8, 16, 8] {
                let pixels = converter.convert(&frame, true, size, size).unwrap();
                assert_eq!(pixels.len(), (size * size * 4) as usize);
                assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == alpha));
            }
        }
    }

    /// Odd and unaligned sizes, opaque and packed-alpha, same size and scaled. swscale's SIMD
    /// writes run past a row's end; this is the shape that corrupted the heap (run it under
    /// valgrind to see the writes, see `convert`).
    #[test]
    fn awkward_sizes_stay_inside_the_output() {
        let mut converter = Converter::new();
        for (w, h) in [(333, 250), (17, 9), (641, 361), (1, 1), (999, 563)] {
            for packed in [false, true] {
                let frame = yuv_frame(w, if packed { h * 2 } else { h }, 128, 128, 128);
                for (dw, dh) in [(w, h), ((w / 2).max(1), (h / 2).max(1)), (w + 3, h + 1)] {
                    let pixels = converter.convert(&frame, packed, dw, dh).unwrap();
                    assert_eq!(pixels.len(), (dw * dh * 4) as usize);
                }
            }
        }
    }
}
