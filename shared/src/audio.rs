//! The audio-output-device protocol between the engine and the config app.
//!
//! Only the wire types and the flags live here. Enumerating devices needs cpal, and cpal lives in
//! the engine alone -- deliberately, for the same reason [`crate::monitor`] keeps the monitor probe
//! there: see [`LIST_AUDIO_DEVICES_FLAG`].

use serde::{Deserialize, Serialize};

/// The argument that makes the engine print its output-device list as JSON and exit.
///
/// The config app can't work these out for itself, and must not try. A device is identified by
/// `cpal::DeviceId`, whose string form is `host:backend-id` -- so the identity depends on *which
/// host cpal picked*, and cpal picks per process from the features it was built with (on Linux:
/// PipeWire, then PulseAudio, then ALSA). A list enumerated by the config binary could therefore
/// be ALSA ids while the engine goes on to resolve PulseAudio ones, and every saved selection
/// would silently fall back to the default device. Asking the engine means the process that
/// enumerates is the process that opens, so a saved id matches by construction.
pub const LIST_AUDIO_DEVICES_FLAG: &str = "--list-audio-devices";

/// The argument that makes the engine play a short test tone and exit, so "test audio" exercises
/// the real playback path rather than a second one that could work when it doesn't.
///
/// Takes the device id to play on, or [`TEST_AUDIO_DEFAULT`].
pub const TEST_AUDIO_FLAG: &str = "--test-audio";

/// The `--test-audio` argument meaning "whatever the system default is" -- the same thing an unset
/// [`crate::user_config::AppConfig::audio_device`] means.
pub const TEST_AUDIO_DEFAULT: &str = "default";

/// What came of a `--test-audio` run, printed as JSON once the tone has finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestAudioResult {
    /// True when the requested device could not be used and the tone played on the system default
    /// instead.
    ///
    /// Worth reporting rather than swallowing: the fallback is silent by design during a session,
    /// but a user pressing "test" and hearing the wrong speakers with no explanation would read it
    /// as the setting being broken.
    pub fell_back: bool,
}

/// One selectable audio output, as the engine sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    /// `cpal::DeviceId`'s `Display` form (`host:backend-id`), which cpal documents as stable
    /// across runs and reboots where the platform allows it, and parses back via `FromStr`. This
    /// is what gets stored in `AppConfig::audio_device`.
    pub id: String,
    /// The device's human-readable name, for the picker. On the PulseAudio host this is the sink
    /// *description* ("soundcore Space One"), not the raw sink name -- which is the whole reason
    /// the PulseAudio backend is worth having on Linux.
    pub name: String,
    /// Whether this is the host's current default output. Shown against "System default" so the
    /// user can see what that currently resolves to.
    pub is_default: bool,
}
