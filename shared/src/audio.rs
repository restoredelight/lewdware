//! The audio-output-device protocol between the engine and the config app.

use serde::{Deserialize, Serialize};

pub const LIST_AUDIO_DEVICES_FLAG: &str = "--list-audio-devices";
pub const TEST_AUDIO_FLAG: &str = "--test-audio";
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
