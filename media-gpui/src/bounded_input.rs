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
    sync::Arc,
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
    file: Arc<File>,
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
    open_bounded_file(Arc::new(File::open(path)?), offset, length)
}

pub fn open_bounded_file(
    file: Arc<File>,
    offset: u64,
    length: u64,
) -> anyhow::Result<BoundedInput> {
    anyhow::ensure!(
        length > 0
            && offset
                .checked_add(length)
                .is_some_and(|end| end <= file.metadata().map(|m| m.len()).unwrap_or(0)),
        "Invalid media byte range"
    );
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
            anyhow::bail!("avformat_open_input failed: {}", ffmpeg::Error::from(ret));
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
