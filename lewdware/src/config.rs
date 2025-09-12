use std::{fs, time::Duration};

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub tags: Option<Vec<String>>,
    pub spawn_interval: Duration,
    pub window_duration: Option<Duration>,
    pub close_button: bool,
    pub max_videos: usize,
    pub video_audio: bool,
    pub open_links: bool,
    pub prompts: bool,
    pub notifications: bool,
    pub moving_window_chance: f64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tags: None,
            spawn_interval: Duration::from_millis(500),
            window_duration: Some(Duration::from_secs(60)),
            close_button: true,
            max_videos: 50,
            video_audio: true,
            open_links: true,
            prompts: true,
            notifications: true,
            moving_window_chance: 1.0 / 20.0,
        }
    }
}

pub fn load_config(path: &str) -> AppConfig {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
