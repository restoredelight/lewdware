use std::io::Cursor;
use std::sync::Mutex;

use shared::mode;
use shared::user_config;
use tauri::Manager;

mod autostart;
mod commands;
mod dto;
mod modes;
mod state;

use commands::process::forward_supervisor_status;
use modes::{load_mode_file, load_pack};
use state::{AppState, UploadedModeEntry};

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

    let _log_guard = shared::logging::init("config").expect("Couldn't start logs");

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
            commands::config::get_config,
            commands::config::save_config,
            commands::theme::get_theme_catalogue,
            commands::hardware::get_monitors,
            commands::hardware::get_audio_devices,
            commands::hardware::test_audio_device,
            commands::modes::get_mode_groups,
            commands::modes::get_mode_options,
            commands::modes::set_mode_option,
            commands::pack::get_pack_metadata,
            commands::pack::pick_pack,
            commands::pack::remove_pack,
            commands::modes::upload_mode,
            commands::modes::remove_uploaded_mode,
            commands::process::launch_lewdware,
            commands::process::stop_lewdware,
            commands::process::lewdware_running,
            commands::schedule::get_schedule_status,
            commands::schedule::set_schedule_enabled,
            commands::schedule::reload_supervisor_schedule,
            commands::diagnostics::open_logs,
            commands::diagnostics::get_diagnostics,
            commands::updates::check_for_update,
            commands::hardware::input_monitoring_granted,
            commands::hardware::request_input_monitoring,
            commands::hardware::open_input_monitoring_settings,
            commands::wallpaper::wallpaper_support,
            commands::wallpaper::wallpaper_restore_preview,
            commands::wallpaper::pick_restore_image,
            commands::wallpaper::default_restore_image,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests;
