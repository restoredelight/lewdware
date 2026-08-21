//! The audio-output-device protocol between the engine and the config app.

use serde::{Deserialize, Serialize};

/// Makes the engine print its audio output devices as JSON and exit.
pub const LIST_AUDIO_DEVICES_COMMAND: &str = "list-audio-devices";
/// Makes the engine play the test chime on the device named by the following argument and exit.
pub const TEST_AUDIO_COMMAND: &str = "test-audio";
pub const TEST_AUDIO_DEFAULT: &str = "default";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestAudioResult {
    pub fell_back: bool,
}

/// An audio output device
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}
