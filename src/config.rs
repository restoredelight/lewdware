use std::{fs, time::Duration};

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub tags: Option<Vec<String>>,
    pub spawn_interval: Duration,
    pub window_duration: Option<Duration>,
    pub close_button: bool,
    pub max_videos: usize,
    pub open_links: bool,
    pub prompts: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tags: None,
            spawn_interval: Duration::from_millis(100),
            window_duration: None,
            close_button: true,
            max_videos: 20,
            open_links: true,
            prompts: false,
        }
    }
}

pub fn load_config(path: &str) -> AppConfig {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
