use std::{fs::File, sync::Mutex, time::Duration};

use serde::{Deserialize, Serialize};
use shared::{
    read_pack::read_pack_metadata,
    user_config::{self, load_config, AppConfig, Key},
};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
struct Config {
    pack_path: Option<String>,
    tags: Option<Vec<String>>,
    popup_frequency: f64,
    max_popup_duration: Option<f64>,
    close_button: bool,
    max_videos: usize,
    video_audio: bool,
    audio: bool,
    open_links: bool,
    link_frequency: f64,
    notifications: bool,
    notification_frequency: f64,
    prompts: bool,
    prompt_frequency: f64,
    moving_windows: bool,
    moving_window_chance: u32,
    panic_button: Key,
}

impl From<AppConfig> for Config {
    fn from(value: AppConfig) -> Self {
        Config {
            pack_path: value.pack_path.map(|x| x.to_str().unwrap().to_string()),
            tags: value.tags,
            popup_frequency: value.popup_frequency.as_secs_f64(),
            max_popup_duration: value.max_popup_duration.map(|x| x.as_secs_f64()),
            close_button: value.close_button,
            max_videos: value.max_videos,
            video_audio: value.video_audio,
            audio: value.audio,
            open_links: value.open_links,
            link_frequency: value.link_frequency.as_secs_f64(),
            notifications: value.notifications,
            notification_frequency: value.notification_frequency.as_secs_f64(),
            prompts: value.prompts,
            prompt_frequency: value.prompt_frequency.as_secs_f64(),
            moving_windows: value.moving_windows,
            moving_window_chance: value.moving_window_chance,
            panic_button: value.panic_button,
        }
    }
}

impl From<Config> for AppConfig {
    fn from(value: Config) -> Self {
        AppConfig {
            pack_path: value.pack_path.map(|x| x.into()),
            tags: value.tags,
            popup_frequency: Duration::from_secs_f64(value.popup_frequency),
            max_popup_duration: value.max_popup_duration.map(Duration::from_secs_f64),
            close_button: value.close_button,
            max_videos: value.max_videos,
            video_audio: value.video_audio,
            audio: value.audio,
            open_links: value.open_links,
            link_frequency: Duration::from_secs_f64(value.link_frequency),
            notifications: value.notifications,
            notification_frequency: Duration::from_secs_f64(value.notification_frequency),
            prompts: value.prompts,
            prompt_frequency: Duration::from_secs_f64(value.prompt_frequency),
            moving_windows: value.moving_windows,
            moving_window_chance: value.moving_window_chance,
            panic_button: value.panic_button,
        }
    }
}

pub struct AppState {
    config: Mutex<Config>,
}

pub type State<'a> = tauri::State<'a, AppState>;

#[tauri::command]
fn save_config(state: State<'_>, config: Config, force: Option<bool>) -> Result<(), String> {
    let force = force.unwrap_or(false);

    let mut current_config = if force {
        state.config.lock().unwrap()
    } else {
        match state.config.try_lock() {
            Ok(config) => config,
            Err(_) => return Ok(()),
        }
    };

    println!("{:?}", config);

    if *current_config != config {
        user_config::save_config(&config.clone().into()).map_err(|err| err.to_string())?;

        *current_config = config;
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackInfo {
    name: String,
    creator: Option<String>,
    description: Option<String>,
    version: Option<String>,
}

#[tauri::command]
fn get_config(state: State<'_>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn load_info(path: String) -> Result<PackInfo, String> {
    let file = File::open(path).map_err(|err| err.to_string())?;

    let (_, metadata) = read_pack_metadata(file).map_err(|err| err.to_string())?;

    Ok(PackInfo {
        name: metadata.name,
        creator: metadata.creator,
        description: metadata.description,
        version: metadata.version,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = load_config().unwrap().into();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            config: Mutex::new(config),
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_config, save_config, load_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
