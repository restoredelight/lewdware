use std::path::PathBuf;

use crate::{
    lua::{Coord, TextFont},
    text_font,
};

/// The widest and tallest an engine-chosen popup may be: a third of the screen's width, half its
/// height -- but never so little that the popup stops being legible, and never more than the
/// screen itself.
///
/// The ratios exist so that many popups fit at once without one dominating. That reasoning holds
/// against a whole monitor, and it holds against a [region](shared::monitor::MonitorRegion) too --
/// which is the same thing as far as everything here is concerned. What it does *not* survive is
/// a small area: dividing a 120px-tall strip by two counts the same restraint twice, and a 60px
/// popup is confetti rather than spectacle. Below the floors the cap becomes the area itself, so
/// the rule reads: at most a third by a half, but a small area may be filled entirely.
///
/// The floors bind only under 900x400 logical pixels -- smaller than any real monitor -- so they
/// change nothing for a popup sized against a whole screen. They exist for areas, which is a
/// shape the engine could not previously be given.
const MIN_POPUP_CAP_WIDTH: f64 = 300.0;
const MIN_POPUP_CAP_HEIGHT: f64 = 200.0;

/// The largest an engine-chosen popup may be on a screen (or area) of this size, in logical
/// pixels. See [`MIN_POPUP_CAP_WIDTH`].
fn popup_size_caps(monitor_width: u32, monitor_height: u32) -> (f64, f64) {
    let (width, height) = (monitor_width as f64, monitor_height as f64);

    (
        (width / 3.0).max(MIN_POPUP_CAP_WIDTH.min(width)),
        (height / 2.0).max(MIN_POPUP_CAP_HEIGHT.min(height)),
    )
}

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
            // Same cap, and the same reason, as an image's: text wrapped to a third of a narrow
            // area is wrapped twice over. See `popup_size_caps`.
            let max_width = popup_size_caps(monitor_width, monitor_height).0 as f32 - padding;
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

    let (cap_width, cap_height) = popup_size_caps(monitor_width, monitor_height);

    let max_width_scale = cap_width / width;
    let max_height_scale = cap_height / height;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4K panel at 2x, in the logical pixels the engine actually works in.
    const SCREEN: (u32, u32) = (1834, 1032);
    /// 1080p media, bigger than any cap here, so the caps are what the result shows.
    const MEDIA: (u32, u32) = (1920, 1080);

    fn default_size(monitor: (u32, u32)) -> (u32, u32) {
        calculate_media_popup_size(None, None, MEDIA.0, MEDIA.1, monitor.0, monitor.1)
    }

    /// The floors must be invisible on a real screen: the ratios, and nothing else, decide there.
    #[test]
    fn a_whole_screen_is_governed_by_the_ratios() {
        let (width, height) = popup_size_caps(SCREEN.0, SCREEN.1);
        assert_eq!(width, SCREEN.0 as f64 / 3.0);
        assert_eq!(height, SCREEN.1 as f64 / 2.0);
    }

    /// The smallest laptop panel worth worrying about, still comfortably above both floors.
    #[test]
    fn a_small_laptop_screen_is_also_governed_by_the_ratios() {
        let (width, height) = popup_size_caps(1366, 768);
        assert_eq!(width, 1366.0 / 3.0);
        assert_eq!(height, 768.0 / 2.0);
    }

    /// The case the floors exist for: half of a 120px strip is 60px, which no image survives.
    /// The strip may be filled instead.
    #[test]
    fn a_narrow_strip_may_be_filled_rather_than_halved() {
        let (_, cap_height) = popup_size_caps(1834, 120);
        assert_eq!(cap_height, 120.0);

        let (_, height) = default_size((1834, 120));
        assert_eq!(height, 120);
    }

    /// A small area gets the floors on both axes -- half of it, not a third by a half.
    #[test]
    fn a_small_area_uses_the_floors() {
        assert_eq!(popup_size_caps(600, 400), (300.0, 200.0));
    }

    /// Whatever the floors say, a popup can never be told it may exceed the area it must fit in.
    #[test]
    fn a_cap_never_exceeds_the_area() {
        for (width, height) in [(100u32, 100u32), (250, 80), (1, 1)] {
            let (cap_width, cap_height) = popup_size_caps(width, height);

            assert!(
                cap_width <= width as f64 && cap_height <= height as f64,
                "{width}x{height} produced caps {cap_width}x{cap_height}"
            );
        }
    }

    /// The floors raise a cap, never a popup: media smaller than the cap keeps its own size.
    #[test]
    fn small_media_is_never_upscaled_to_a_floor() {
        assert_eq!(
            calculate_media_popup_size(None, None, 64, 48, 400, 300),
            (64, 48)
        );
    }

    /// A cap is a ceiling, not a target: the aspect ratio still decides which axis binds.
    #[test]
    fn the_aspect_ratio_still_governs_within_the_caps() {
        let (width, height) = default_size(SCREEN);

        assert_eq!(width, SCREEN.0 / 3);
        assert_eq!(
            height,
            ((width as f64 / MEDIA.0 as f64) * MEDIA.1 as f64).round() as u32
        );
    }
}
