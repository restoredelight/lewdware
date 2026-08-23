mod harness;
mod media_query;
mod mode_config;
mod mode_support;
mod modes_attributes;
mod modes_audio;
mod modes_chrome;
mod modes_content;
mod modes_experience;
mod modes_notifications;
mod modes_popups;
mod modes_prompts;
mod modes_spawn_tags;
mod modes_splash;
mod modes_timeline;
mod modes_wallpaper;
mod modes_web;
mod runtime;

use shared::user_config::Permissions;

use super::*;

/// Makes sure `utils::temp_dir()` exists, which anything extracting media from a pack
/// (`MediaPack::extract`) writes into without creating it first.
///
/// In the real engine that directory is made once at startup by `utils::prepare_temp_dir`
/// (see `main.rs`); no test goes through `main`, so without this a test only passes when
/// some *earlier* run of the actual app happened to leave the directory behind. That made
/// the media tests pass against a developer's `/tmp` and fail on any clean one -- a fresh CI
/// runner, or a `TMPDIR` pointed somewhere new.
///
/// Deliberately `create_dir_all` rather than `prepare_temp_dir`: the latter clears the
/// directory first, and is documented as safe only while holding the single-instance lock.
/// Cargo runs these tests in parallel threads, so a sweep here would delete media out from
/// under a test running concurrently. Creating is idempotent and safe to race.
pub(super) fn ensure_temp_dir() {
    // Ignoring the error is deliberate: if the directory can't be created, the test that
    // needs it fails on its own with a clearer message than a panic here would give.
    let _ = std::fs::create_dir_all(crate::utils::temp_dir());
}

/// Every capability allowed.
///
/// Deliberately not `Capabilities::default()`: `open_link` now ships *denied* (see
/// `Capabilities::default`), and a test about what a mode does with a link would otherwise
/// quietly turn into a test of a denied permission. Tests that care about denial say so
/// through `Harness::with_capabilities`.
pub(super) fn all_capabilities() -> Permissions {
    Permissions {
        set_wallpaper: true,
        open_links: true,
        send_notifications: true,
    }
}
