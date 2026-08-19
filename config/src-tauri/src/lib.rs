use std::io::Cursor;
use std::sync::Mutex;

use shared::mode;
use shared::user_config;
use tauri::Manager;

mod commands;
mod dto;
mod modes;
mod state;

pub use commands::*;
pub use dto::*;
pub use state::*;

use modes::{load_mode_file, load_pack};
use state::UploadedModeEntry;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let sandbox_mode_bytes = include_bytes!("../../../default-modes/sandbox/build/Sandbox.lwmode");
    let sandbox_mode = mode::read_mode_metadata(&mut Cursor::new(sandbox_mode_bytes))
        .expect("failed to load embedded Sandbox mode")
        .1;

    let experience_mode_bytes =
        include_bytes!("../../../default-modes/experience/build/Sequence.lwmode");
    let experience_mode = mode::read_mode_metadata(&mut Cursor::new(experience_mode_bytes))
        .expect("failed to load embedded Experience mode")
        .1;

    let _log_guard = shared::logging::init("config");

    let config = user_config::load_config().unwrap_or_default();

    let pack = config.pack_path.as_ref().and_then(|p| {
        load_pack(p.clone())
            .inspect_err(|e| tracing::error!("failed to load pack: {e}"))
            .ok()
    });

    let uploaded: Vec<UploadedModeEntry> = config
        .uploaded_modes
        .iter()
        .filter_map(|p| {
            load_mode_file(p.clone())
                .inspect_err(|e| tracing::error!("failed to load mode {}: {e}", p.display()))
                .ok()
        })
        .collect();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            config: Mutex::new(config),
            pack: Mutex::new(pack),
            uploaded: Mutex::new(uploaded),
            sandbox_mode,
            experience_mode,
        })
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;
                window.set_icon(icon)?;
            }
            tauri::async_runtime::spawn(forward_supervisor_status(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_theme_catalogue,
            get_monitors,
            get_audio_devices,
            test_audio_device,
            get_mode_groups,
            get_mode_options,
            set_mode_option,
            pick_pack,
            remove_pack,
            upload_mode,
            remove_uploaded_mode,
            launch_lewdware,
            stop_lewdware,
            lewdware_running,
            get_schedule_status,
            set_schedule_enabled,
            reload_supervisor_schedule,
            open_logs,
            get_diagnostics,
            check_for_update,
            input_monitoring_granted,
            request_input_monitoring,
            open_input_monitoring_settings,
            wallpaper_support,
            wallpaper_restore_preview,
            pick_restore_image,
            default_restore_image,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests;
