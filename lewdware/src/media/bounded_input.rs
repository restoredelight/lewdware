//! Opens an ffmpeg [`Input`](ffmpeg::format::context::Input) bound to a byte range
//! `[offset, offset + length)` within a larger file, without copying that range out to a
//! standalone temp file first.
//!
//! Pack files bundle many videos/audio clips back-to-back with a SQLite index recording each
//! one's offset and length. Previously we extracted each clip to its own temp file before handing
//! a path to ffmpeg; this reads directly out of the pack file instead, via a custom
//! `AVIOContext` — the same technique as ffmpeg's own `avio_read_callback.c` example, extended
//! with a `seek` callback (containers like mp4 need to seek around their own box structure, e.g.
//! to reach the moov atom) and bounded to the given window instead of the whole file.

use std::{
    ffi::c_void,
    fs::File,
    io, mem,
    ops::{Deref, DerefMut},
    os::raw::c_int,
    path::Path,
    ptr,
};

use ffmpeg_next::{self as ffmpeg, Error as FfmpegError, ffi};

/// Size of the buffer ffmpeg reads through. Larger than the 4096 bytes ffmpeg's own example
/// uses since we're backed by real file I/O (one `pread` per underfilled buffer) rather than a
/// plain memory copy.
const AVIO_BUFFER_SIZE: usize = 32 * 1024;

/// An `ffmpeg::format::context::Input` opened against a bounded region of a file. Derefs to
/// `Input`, so it's a drop-in replacement for the value `ffmpeg::format::input(path)` returns.
pub struct BoundedInput {
    input: mem::ManuallyDrop<ffmpeg::format::context::Input>,
    avio_ctx: *mut ffi::AVIOContext,
    opaque: *mut BoundedFile,
}

// Safety: mirrors `ffmpeg_next::format::context::Input`'s own `unsafe impl Send` (the underlying
// AVFormatContext has no thread affinity), plus `BoundedFile`'s `File` and offsets, which are
// likewise `Send`. `BoundedInput` is only ever accessed from one thread at a time.
unsafe impl Send for BoundedInput {}

impl Deref for BoundedInput {
    type Target = ffmpeg::format::context::Input;

    fn deref(&self) -> &Self::Target {
        &self.input
    }
}

impl DerefMut for BoundedInput {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.input
    }
}

impl Drop for BoundedInput {
    fn drop(&mut self) {
        unsafe {
            // Closes the AVFormatContext. Because we set AVFMT_FLAG_CUSTOM_IO below, this does
            // NOT touch `avio_ctx` (ffmpeg would otherwise call avio_close() on it, which
            // assumes `pb->opaque` is a URLContext -- ours is our own BoundedFile, so that would
            // be undefined behaviour). We free avio_ctx ourselves right after, same order as
            // ffmpeg's avio_read_callback.c example.
            mem::ManuallyDrop::drop(&mut self.input);

            if !self.avio_ctx.is_null() {
                ffi::av_freep(&mut (*self.avio_ctx).buffer as *mut _ as *mut c_void);
                ffi::avio_context_free(&mut self.avio_ctx);
            }

            drop(Box::from_raw(self.opaque));
        }
    }
}

/// Per-open state backing the read/seek callbacks. One instance per `BoundedInput`; each opens
/// its own `File` handle, so no locking/sharing is needed even when several bounded inputs read
/// the same pack file concurrently (e.g. a video's own stream plus its separately-decoded audio
/// track).
struct BoundedFile {
    file: File,
    offset: u64,
    length: u64,
    /// Current position, relative to `offset`, in `[0, length]`.
    pos: u64,
}

#[cfg(unix)]
fn pread(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buf, offset)
}

#[cfg(windows)]
fn pread(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buf, offset)
}

unsafe extern "C" fn read_packet(opaque: *mut c_void, buf: *mut u8, buf_size: c_int) -> c_int {
    let state = unsafe { &mut *opaque.cast::<BoundedFile>() };

    if buf_size < 0 || buf.is_null() {
        return c_int::from(FfmpegError::Other {
            errno: libc::EINVAL,
        });
    }

    let remaining = state.length.saturating_sub(state.pos);
    if remaining == 0 {
        return c_int::from(FfmpegError::Eof);
    }

    let to_read = (buf_size as u64).min(remaining) as usize;
    // Safety: ffmpeg guarantees `buf` is valid for `buf_size` writable bytes, and `to_read` <=
    // `buf_size`.
    let dest = unsafe { std::slice::from_raw_parts_mut(buf, to_read) };

    match pread(&state.file, dest, state.offset + state.pos) {
        Ok(0) => c_int::from(FfmpegError::Eof),
        Ok(n) => {
            state.pos += n as u64;
            n as c_int
        }
        Err(err) => {
            tracing::error!("Bounded pack read failed: {err}");
            c_int::from(FfmpegError::Other {
                errno: err.raw_os_error().unwrap_or(libc::EIO),
            })
        }
    }
}

unsafe extern "C" fn seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    let state = unsafe { &mut *opaque.cast::<BoundedFile>() };

    if whence & ffi::AVSEEK_SIZE != 0 {
        return state.length as i64;
    }

    let base_whence = whence & !ffi::AVSEEK_FORCE;

    let base = match base_whence {
        libc::SEEK_SET => 0i64,
        libc::SEEK_CUR => state.pos as i64,
        libc::SEEK_END => state.length as i64,
        _ => return -1,
    };

    let new_pos = match base.checked_add(offset) {
        Some(pos) if pos >= 0 && pos as u64 <= state.length => pos,
        _ => return -1,
    };

    state.pos = new_pos as u64;
    new_pos
}

/// Opens `path`, exposing only the bytes in `[offset, offset + length)` to ffmpeg as if it were
/// the whole file.
pub fn open_bounded(path: &Path, offset: u64, length: u64) -> anyhow::Result<BoundedInput> {
    let file = File::open(path)?;
    let opaque = Box::into_raw(Box::new(BoundedFile {
        file,
        offset,
        length,
        pos: 0,
    }));

    unsafe {
        let buffer = ffi::av_malloc(AVIO_BUFFER_SIZE) as *mut u8;
        if buffer.is_null() {
            drop(Box::from_raw(opaque));
            anyhow::bail!("av_malloc failed to allocate the AVIOContext buffer");
        }

        let avio_ctx = ffi::avio_alloc_context(
            buffer,
            AVIO_BUFFER_SIZE as c_int,
            0, // read-only
            opaque as *mut c_void,
            Some(read_packet),
            None,
            Some(seek),
        );

        if avio_ctx.is_null() {
            ffi::av_free(buffer as *mut c_void);
            drop(Box::from_raw(opaque));
            anyhow::bail!("avio_alloc_context failed");
        }

        // Free `avio_ctx` (and its buffer, and our opaque state) the same way `Drop` does. Only
        // used on the error paths below, before a `BoundedInput` exists to own that cleanup.
        let free_avio_ctx = |mut avio_ctx: *mut ffi::AVIOContext| {
            ffi::av_freep(&mut (*avio_ctx).buffer as *mut _ as *mut c_void);
            ffi::avio_context_free(&mut avio_ctx);
            drop(Box::from_raw(opaque));
        };

        let mut fmt_ctx = ffi::avformat_alloc_context();
        if fmt_ctx.is_null() {
            free_avio_ctx(avio_ctx);
            anyhow::bail!("avformat_alloc_context failed");
        }

        (*fmt_ctx).pb = avio_ctx;
        (*fmt_ctx).flags |= ffi::AVFMT_FLAG_CUSTOM_IO;

        let ret =
            ffi::avformat_open_input(&mut fmt_ctx, ptr::null(), ptr::null_mut(), ptr::null_mut());
        if ret < 0 {
            // On failure ffmpeg frees the AVFormatContext we supplied (and sets fmt_ctx to
            // NULL), but leaves `pb` alone because of AVFMT_FLAG_CUSTOM_IO, so avio_ctx is still
            // ours to free.
            free_avio_ctx(avio_ctx);
            anyhow::bail!(
                "avformat_open_input failed: {}",
                ffmpeg::Error::from(ret)
            );
        }

        let ret = ffi::avformat_find_stream_info(fmt_ctx, ptr::null_mut());
        if ret < 0 {
            ffi::avformat_close_input(&mut fmt_ctx);
            free_avio_ctx(avio_ctx);
            anyhow::bail!(
                "avformat_find_stream_info failed: {}",
                ffmpeg::Error::from(ret)
            );
        }

        Ok(BoundedInput {
            input: mem::ManuallyDrop::new(ffmpeg::format::context::Input::wrap(fmt_ctx)),
            avio_ctx,
            opaque,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    const TEST_CLIP: &[u8] = include_bytes!("test_fixtures/test_clip.mp4");

    /// Writes `TEST_CLIP` into a file surrounded by unrelated padding bytes (simulating other
    /// media packed before/after it in a real pack file), and returns the path plus the
    /// offset/length window `TEST_CLIP` occupies within it.
    fn write_padded_clip() -> (tempfile::NamedTempFile, u64, u64) {
        let prefix = b"not a video, just padding before it in the pack file..";
        let suffix = b"and some unrelated bytes after it too";

        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(prefix).unwrap();
        file.write_all(TEST_CLIP).unwrap();
        file.write_all(suffix).unwrap();
        file.flush().unwrap();

        (file, prefix.len() as u64, TEST_CLIP.len() as u64)
    }

    fn stream_summary(input: &ffmpeg::format::context::Input) -> Vec<(i32, u32, u32)> {
        input
            .streams()
            .map(|s| {
                let params = s.parameters();
                (
                    unsafe { (*params.as_ptr()).codec_type } as i32,
                    unsafe { (*params.as_ptr()).width as u32 },
                    unsafe { (*params.as_ptr()).height as u32 },
                )
            })
            .collect()
    }

    #[test]
    fn opens_and_matches_direct_open() {
        ffmpeg::init().unwrap();

        let (padded, offset, length) = write_padded_clip();
        let bounded = open_bounded(padded.path(), offset, length).unwrap();

        let mut direct_file = tempfile::NamedTempFile::new().unwrap();
        direct_file.write_all(TEST_CLIP).unwrap();
        direct_file.flush().unwrap();
        let direct = ffmpeg::format::input(&direct_file.path()).unwrap();

        assert_eq!(bounded.nb_streams(), direct.nb_streams());
        assert!(bounded.nb_streams() >= 2, "expected a video and audio stream");

        assert_eq!(stream_summary(&bounded), stream_summary(&direct));

        // Duration should match within ffmpeg's own rounding (AV_TIME_BASE units).
        assert!((bounded.duration() - direct.duration()).abs() < 10_000);
    }

    #[test]
    fn decodes_every_frame_and_supports_seeking() {
        ffmpeg::init().unwrap();

        let (padded, offset, length) = write_padded_clip();
        let mut bounded = open_bounded(padded.path(), offset, length).unwrap();

        let video_stream_index = bounded
            .streams()
            .best(ffmpeg::media::Type::Video)
            .unwrap()
            .index();

        let mut decoded_frames = 0;

        for (stream, packet) in bounded.packets() {
            if stream.index() != video_stream_index {
                continue;
            }

            let params = stream.parameters();
            let context = ffmpeg::codec::Context::from_parameters(params).unwrap();
            let mut decoder = context.decoder().video().unwrap();
            let mut frame = ffmpeg::util::frame::Video::empty();

            decoder.send_packet(&packet).unwrap();
            while decoder.receive_frame(&mut frame).is_ok() {
                decoded_frames += 1;
            }
        }

        // Not asserting an exact count (depends on GOP structure), just that we actually got
        // real decodable data out of the bounded region, not garbage/empty reads.
        assert!(decoded_frames > 0, "expected to decode at least one frame");

        // Exercise the seek callback (containers like mp4 do this internally too, but drive it
        // directly here the same way video.rs/audio.rs loop videos: seek back to the start).
        bounded.seek(0, ..0).unwrap();

        let mut frames_after_seek = 0;
        for (stream, _packet) in bounded.packets() {
            if stream.index() == video_stream_index {
                frames_after_seek += 1;
            }
            if frames_after_seek > 0 {
                break;
            }
        }
        assert!(frames_after_seek > 0, "expected packets after seeking back to start");
    }

    #[test]
    fn seek_flags() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"hello world").unwrap();
        file.flush().unwrap();

        let opaque = Box::into_raw(Box::new(BoundedFile {
            file: file.reopen().unwrap(),
            offset: 0,
            length: 11,
            pos: 5,
        }));

        unsafe {
            // Test that AVSEEK_SIZE returns the length
            assert_eq!(seek(opaque as *mut c_void, 0, ffi::AVSEEK_SIZE), 11);
            assert_eq!(seek(opaque as *mut c_void, 0, ffi::AVSEEK_SIZE | ffi::AVSEEK_FORCE), 11);

            // Test standard SEEK_SET
            assert_eq!(seek(opaque as *mut c_void, 2, libc::SEEK_SET), 2);
            assert_eq!((*opaque).pos, 2);

            // Test SEEK_SET with AVSEEK_FORCE
            assert_eq!(seek(opaque as *mut c_void, 4, libc::SEEK_SET | ffi::AVSEEK_FORCE), 4);
            assert_eq!((*opaque).pos, 4);

            // Clean up
            drop(Box::from_raw(opaque));
        }
    }
}
