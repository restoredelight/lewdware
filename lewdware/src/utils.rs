use std::path::PathBuf;

use crate::{
    lua::{Coord, TextFont},
    text_font,
};

pub fn calculate_media_popup_size(
    width: Option<Coord>,
    height: Option<Coord>,
    media_width: u32,
    media_height: u32,
    monitor_width: u32,
    monitor_height: u32,
) -> (u32, u32) {
    let width = width.map(|width| width.to_pixels(monitor_width).max(0) as u32);
    let height = height.map(|height| height.to_pixels(monitor_height).max(0) as u32);

    match (width, height) {
        (None, None) => {
            default_media_popup_size(media_width, media_height, monitor_width, monitor_height)
        }
        (None, Some(height)) => (
            ((height as f64 / media_height as f64) * media_width as f64).round() as u32,
            height,
        ),
        (Some(width), None) => (
            width,
            ((width as f64 / media_width as f64) * media_height as f64).round() as u32,
        ),
        (Some(width), Some(height)) => (width, height),
    }
}

/// Resolve the size of a text popup. Unlike `calculate_media_popup_size`, text has no fixed
/// aspect ratio to scale, so an omitted width/height wraps the text to fit rather than scaling a
/// font size the caller explicitly chose.
///
/// `outline_width` (0 if the text has no outline) is added as padding on auto-computed
/// dimensions, since the outline is drawn by repainting the text offset by up to
/// `outline_width` pixels in every direction — without this, the stroke would be clipped by the
/// window bounds.
#[allow(clippy::too_many_arguments)]
pub fn calculate_text_popup_size(
    width: Option<Coord>,
    height: Option<Coord>,
    text: &str,
    font: TextFont,
    font_size: f32,
    outline_width: f32,
    monitor_width: u32,
    monitor_height: u32,
) -> (u32, u32) {
    let width = width.map(|width| width.to_pixels(monitor_width).max(0) as u32);
    let height = height.map(|height| height.to_pixels(monitor_height).max(0) as u32);

    let padding = outline_width * 2.0;

    match (width, height) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) => {
            let wrap_width = (width as f32 - padding).max(0.0);
            let size = text_font::measure(text, font, font_size, wrap_width);
            (width, (size.y + padding).ceil() as u32)
        }
        (None, height) => {
            let max_width = (monitor_width / 3) as f32 - padding;
            let natural = text_font::measure(text, font, font_size, f32::INFINITY);

            let (width, wrapped_height) = if natural.x <= max_width {
                (natural.x, natural.y)
            } else {
                let wrapped = text_font::measure(text, font, font_size, max_width);
                (max_width, wrapped.y)
            };
            let width = width + padding;

            (
                width.ceil() as u32,
                height.unwrap_or((wrapped_height + padding).ceil() as u32),
            )
        }
    }
}

fn default_media_popup_size(
    media_width: u32,
    media_height: u32,
    monitor_width: u32,
    monitor_height: u32,
) -> (u32, u32) {
    let width = media_width as f64;
    let height = media_height as f64;

    let max_width_scale = (monitor_width as f64 / 3.0) / width;
    let max_height_scale = (monitor_height as f64 / 2.0) / height;

    let scale = max_width_scale.min(max_height_scale).min(1.0);

    let width = (width * scale).round();
    let height = (height * scale).round();

    (width as u32, height as u32)
}

/// Pick a random coordinate (within `[0, total_size]`) at which a window of `window_size` fits
/// entirely on screen — or 0 if the window is bigger than the available space.
pub fn random_position(window_size: u32, total_size: u32) -> i32 {
    if window_size > total_size {
        0
    } else {
        rand::random_range(0i32..=(total_size - window_size) as i32)
    }
}

// Silence the "Secure coding is automatically enabled for restorable state" warning by explicitly
// opting in. winit doesn't do this itself, so we inject the method into its app delegate class.
//
// Must be called after EventLoop::build() (which creates the NSApplication and sets its delegate)
// and before run_app() (when the method is first queried).
#[cfg(target_vendor = "apple")]
pub fn opt_in_secure_restorable_state() {
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject, Bool, Sel},
        sel,
    };
    use std::ffi::c_char;

    unsafe extern "C" {
        fn class_addMethod(
            cls: *const AnyClass,
            name: Sel,
            imp: unsafe extern "C" fn(),
            types: *const c_char,
        ) -> bool;
    }

    unsafe extern "C" fn returns_yes(_: *mut AnyObject, _: Sel) -> Bool {
        Bool::YES
    }

    unsafe {
        let app_cls = AnyClass::get(c"NSApplication").expect("NSApplication");
        let app: *mut AnyObject = msg_send![app_cls, sharedApplication];
        let delegate: *mut AnyObject = msg_send![app, delegate];
        if delegate.is_null() {
            return;
        }
        class_addMethod(
            (*delegate).class(),
            sel!(applicationSupportsSecureRestorableState:),
            std::mem::transmute::<
                unsafe extern "C" fn(*mut AnyObject, Sel) -> Bool,
                unsafe extern "C" fn(),
            >(returns_yes),
            c"c@:".as_ptr(),
        );
    }
}

// lewdware opens lots of file descriptors on Unix systems (Windows, media files, GPU stuff),
// so increase the file descriptor limit so we don't crash with lots of windows open.
#[cfg(unix)]
pub fn raise_fd_limit() {
    unsafe {
        let mut rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) == 0 {
            let target_limit = 65535;
            if rlim.rlim_cur < target_limit {
                rlim.rlim_cur = std::cmp::min(target_limit, rlim.rlim_max);
                if libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) != 0 {
                    tracing::error!("Failed to raise file descriptor limit");
                }
            }
        }
    }
}

#[cfg(not(unix))]
pub fn raise_fd_limit() {}

/// Reports a fatal startup error: logs it (as before), forwards it to the supervisor over the
/// engine-link connection (see `supervisor_link`) so it's visible wherever sessions are
/// reported, and fires a desktop notification directly, since the engine-link connection might
/// not be up yet for a very early failure (and historically this notification predates the
/// supervisor entirely).
pub fn report_fatal_startup_error(err: impl std::fmt::Display) {
    let message = err.to_string();
    tracing::error!("{message}");

    crate::supervisor_link::report(shared::ipc::EngineToSupervisor::FailedToStart {
        message: message.clone(),
    });

    if let Err(err) = notify_rust::Notification::new()
        .summary("Lewdware failed to start")
        .body(&message)
        .show()
    {
        tracing::warn!("Failed to show startup-failure notification: {err}");
    }
}

/// The directory lewdware extracts media (images/video/audio, the pack's SQLite index) into
/// while running. A subdirectory of the regular system temp dir (`$TMPDIR`/`/tmp`), *not*
/// `$XDG_RUNTIME_DIR` (used for the lock file) — the runtime dir is conventionally sized for
/// small runtime objects like sockets, and is typically much smaller than `/tmp` (on this
/// machine: ~1.6GB vs. ~6GB), which matters since these extracted files can be large (whole
/// video/audio files, a pack's full SQLite index). Kept in its own subdirectory so leftovers
/// from a previous run can be identified and swept away, see [`prepare_temp_dir`].
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("lewdware-tmp")
}

/// Clears out [`temp_dir`] and recreates it. Only safe to call while holding the single-instance
/// lock: if a previous session crashed or was force-killed, its `NamedTempFile`s never got a
/// chance to run their `Drop` cleanup. Sweeping the directory on the next startup, once we know
/// no other instance is running, reclaims that space instead of letting it grow across sessions.
pub fn prepare_temp_dir() -> std::io::Result<PathBuf> {
    let dir = temp_dir();

    if let Err(err) = std::fs::remove_dir_all(&dir)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("Failed to clear stale temp dir {}: {err}", dir.display());
    }

    std::fs::create_dir_all(&dir)?;

    Ok(dir)
}
