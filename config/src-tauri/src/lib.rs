use std::{env, fs::File, sync::Mutex, time::Duration};

use serde::{Deserialize, Serialize};
use shared::{
    read_pack::read_pack_metadata,
    user_config::{self, load_config, AppConfig, Key},
};
use tauri::AppHandle;

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
    disabled_monitors: Vec<String>
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
            disabled_monitors: value.disabled_monitors,
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
            disabled_monitors: value.disabled_monitors,
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

#[derive(Serialize, Deserialize)]
pub struct Monitor {
    id: String,
    name: String,
    primary: bool,
    disabled: bool,
}

#[tauri::command]
async fn get_monitors(app_handle: AppHandle, state: State<'_>) -> Result<Vec<Monitor>, String> {
    let primary_monitor_name = app_handle
        .primary_monitor()
        .map_err(|err| err.to_string())?
        .and_then(|monitor_handle| monitor_handle.name().cloned());

    let config = state.config.lock().unwrap();

    let mut monitors: Vec<_> = app_handle
        .available_monitors()
        .map_err(|err| err.to_string())?
        .iter()
        .filter_map(|monitor_handle| {
            let id = monitor_handle.name()?.to_string();

            let primary = Some(&id) == primary_monitor_name.as_ref();

            let disabled = config.disabled_monitors.contains(&id);

            let size = monitor_handle.size();
            let name = format!("{id} ({}x{})", size.width, size.height);

            Some(Monitor { id, name, primary, disabled })
        })
        .collect();

    // Make sure the primary monitor is always first
    if let Some(primary_position) = monitors.iter().position(|monitor| monitor.primary) {
        monitors.swap(0, primary_position);
    }

    Ok(monitors)
}

#[tauri::command]
fn is_wayland() -> bool {
    if cfg!(target_os = "linux") {
        match env::var("XDG_SESSION_TYPE") {
            Ok(session_type) => session_type.to_lowercase() == "wayland",
            Err(_) => false,
        }
    } else {
        false
    }
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
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            load_info,
            get_monitors,
            is_wayland,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
