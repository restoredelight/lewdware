use std::{fs, path::PathBuf, time::Duration};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct AppConfig {
    pub pack_path: Option<PathBuf>,
    pub tags: Option<Vec<String>>,
    pub popup_frequency: Duration,
    pub max_popup_duration: Option<Duration>,
    pub close_button: bool,
    pub max_videos: usize,
    pub video_audio: bool,
    pub audio: bool,
    pub open_links: bool,
    pub link_frequency: Duration,
    pub notifications: bool,
    pub notification_frequency: Duration,
    pub prompts: bool,
    pub prompt_frequency: Duration,
    pub moving_windows: bool,
    pub moving_window_chance: u32,
    pub panic_button: egui::Key,
    pub panic_modifiers: egui::Modifiers,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            pack_path: None,
            tags: None,
            popup_frequency: Duration::from_millis(500),
            max_popup_duration: Some(Duration::from_secs(60)),
            close_button: true,
            max_videos: 50,
            video_audio: true,
            audio: true,
            open_links: true,
            link_frequency: Duration::from_secs(10),
            notifications: true,
            notification_frequency: Duration::from_secs(2),
            prompts: true,
            prompt_frequency: Duration::from_secs(60),
            moving_windows: false,
            moving_window_chance: 5,
            panic_button: egui::Key::Escape,
            panic_modifiers: egui::Modifiers::NONE,
        }
    }
}

pub fn load_config() -> Result<AppConfig> {
    let path = config_path()?;

    Ok(fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

pub fn save_config(config: &AppConfig) -> Result<()> {
    let path = config_path()?;

    fs::write(path, serde_json::to_string(config)?)?;

    Ok(())
}

fn config_path() -> Result<PathBuf> {
    let mut config_path =
        dirs::config_dir().ok_or_else(|| anyhow!("Could not find a valid config dir for this OS"))?;

    config_path.push("lewdware");

    fs::create_dir_all(&config_path)?;

    config_path.push("config.json");

    Ok(config_path)
}
