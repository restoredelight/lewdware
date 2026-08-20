mod commands;
mod editor_db;
mod encode;
mod history;
mod import;
mod media_server;
mod pack;
mod thumbnail;

use std::sync::{Arc, OnceLock};

use pack::MediaPack;
use serde::Serialize;
use tauri::{Emitter, Manager};
use tokio::sync::{watch, Mutex, RwLock};

use shared::encode::HardwareEncoder;

use commands::*;

pub type PackState = Arc<Mutex<Option<Arc<MediaPack>>>>;

pub struct AppState {
    pub pack: PackState,
    pub media_server: OnceLock<MediaServerInfo>,
    pub hardware_encoder: OnceLock<HardwareEncoder>,
    pub upload_lock: Arc<RwLock<()>>,
    pub cancel_uploads: watch::Sender<bool>,
}

impl AppState {
    /// Runs `f` against the open pack, holding the state lock for as long as it takes -- the
    /// shape of nearly every command. Errors when no pack is open.
    async fn with_pack<T>(
        &self,
        f: impl AsyncFnOnce(&MediaPack) -> anyhow::Result<T>,
    ) -> Result<T, String> {
        let lock = self.pack.lock().await;
        let pack = lock.as_ref().ok_or("No pack open")?;
        f(pack).await.map_err(|e| e.to_string())
    }

    /// Like [`Self::with_pack`], but answers `default` when no pack is open rather than erroring
    /// -- for the queries the frontend makes before one is loaded.
    async fn with_pack_or<T>(
        &self,
        default: T,
        f: impl AsyncFnOnce(&MediaPack) -> anyhow::Result<T>,
    ) -> Result<T, String> {
        let lock = self.pack.lock().await;
        match lock.as_ref() {
            Some(pack) => f(pack).await.map_err(|e| e.to_string()),
            None => Ok(default),
        }
    }

    fn new() -> Self {
        Self {
            pack: Arc::new(Mutex::new(None)),
            media_server: OnceLock::new(),
            hardware_encoder: OnceLock::new(),
            upload_lock: Arc::new(RwLock::new(())),
            cancel_uploads: watch::channel(false).0,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct MediaServerInfo {
    pub port: u16,
    pub token: String,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _log_guard = shared::logging::init("pack-editor").expect("Couldn't start logs");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin({
            #[cfg(target_os = "windows")]
            {
                tauri_plugin_prevent_default::Builder::new()
                    .platform(
                        tauri_plugin_prevent_default::PlatformOptions::new()
                            .browser_accelerator_keys(false)
                            .default_context_menus(false)
                            .default_script_dialogs(false),
                    )
                    .build()
            }
            #[cfg(not(target_os = "windows"))]
            {
                tauri_plugin_prevent_default::init()
            }
        })
        .manage(AppState::new())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;
                window.set_icon(icon)?;
            }

            let state = app.state::<AppState>();

            if let Ok(resource_dir) = app.path().resource_dir() {
                let ffmpeg_name = if cfg!(target_os = "windows") {
                    "lewdware-ffmpeg.exe"
                } else {
                    "lewdware-ffmpeg"
                };
                let ffprobe_name = if cfg!(target_os = "windows") {
                    "lewdware-ffprobe.exe"
                } else {
                    "lewdware-ffprobe"
                };
                let avifenc_name = if cfg!(target_os = "windows") {
                    "lewdware-avifenc.exe"
                } else {
                    "lewdware-avifenc"
                };
                let ffmpeg = resource_dir.join("binaries").join(ffmpeg_name);
                let ffprobe = resource_dir.join("binaries").join(ffprobe_name);
                let avifenc = resource_dir.join("binaries").join(avifenc_name);
                if ffmpeg.exists() && ffprobe.exists() && avifenc.exists() {
                    shared::encode::init_binary_paths(ffmpeg, ffprobe, avifenc);
                }
            }
            let _ = state
                .hardware_encoder
                .set(HardwareEncoder::detect_and_test());

            let pack = state.pack.clone();
            let token = uuid::Uuid::new_v4().simple().to_string();
            let (tx, rx) = std::sync::mpsc::channel();
            tauri::async_runtime::spawn(async move {
                match media_server::start(pack, token.clone()).await {
                    Ok(port) => {
                        tx.send(MediaServerInfo { port, token }).ok();
                    }
                    Err(e) => tracing::error!("media server failed to start: {e}"),
                }
            });
            if let Ok(server) = rx.recv() {
                state.media_server.set(server).ok();
            }
            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        win.emit("close-requested", ()).ok();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            lifecycle::new_pack,
            lifecycle::open_pack_dialog,
            lifecycle::open_recent_pack,
            recents::get_recent_packs,
            recents::remove_recent_pack,
            lifecycle::import_edgeware_pack_dialog,
            lifecycle::save_pack,
            lifecycle::save_pack_as_dialog,
            lifecycle::discard_changes,
            lifecycle::discard_pack,
            lifecycle::close_pack,
            lifecycle::confirm_close,
            lifecycle::is_pack_saved,
            lifecycle::get_history_status,
            lifecycle::undo,
            lifecycle::redo,
            files::get_files,
            files::remove_files,
            modes::get_modes,
            modes::add_mode_dialog,
            modes::remove_mode,
            files::set_file_title,
            files::set_file_source_url,
            labels::get_all_tags,
            labels::get_file_tags,
            labels::add_tag_to_file,
            labels::remove_tag_from_file,
            labels::create_and_add_tag,
            labels::get_tag_summaries,
            labels::rename_tag,
            labels::merge_tag,
            labels::delete_tag,
            labels::add_tag_to_files,
            labels::remove_tag_from_files,
            labels::get_all_artists,
            labels::get_file_artists,
            labels::add_artist_to_file,
            labels::remove_artist_from_file,
            labels::create_and_add_artist,
            labels::get_artist_summaries,
            labels::rename_artist,
            labels::merge_artist,
            labels::delete_artist,
            labels::add_artist_to_files,
            labels::remove_artist_from_files,
            metadata::get_pack_metadata,
            metadata::set_pack_metadata,
            metadata::save_pack_metadata,
            metadata::mark_pack_unsaved,
            behaviour::get_behaviour,
            behaviour::edit_behaviour,
            slots::fill_media_slot_dialog,
            slots::set_media_slot,
            slots::clear_media_slot,
            slots::set_shown_as_popup,
            slots::set_popup_audio,
            upload::add_files_dialog,
            upload::add_folder_dialog,
            upload::add_paths,
            upload::cancel_upload,
            server::get_media_server,
            update::check_for_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
