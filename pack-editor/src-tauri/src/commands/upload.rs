//! Bringing new media into the pack, from a file picker, a folder, or a drop.

use std::path::PathBuf;

use shared::encode::HardwareEncoder;
use tauri::{AppHandle, State};

use crate::{encode, AppState};

#[tauri::command]
pub async fn add_files_dialog(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    use tauri_plugin_dialog::DialogExt;
    let app_c = app.clone();
    let files = tokio::task::spawn_blocking(move || {
        app_c
            .dialog()
            .file()
            .set_title("Select files")
            .blocking_pick_files()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(handles) = files else { return Ok(()) };
    let paths: Vec<PathBuf> = handles
        .into_iter()
        .filter_map(|h| h.into_path().ok())
        .collect();

    if paths.is_empty() {
        return Ok(());
    }

    let pack_state = state.pack.clone();
    let encoder = state
        .hardware_encoder
        .get()
        .cloned()
        .unwrap_or(HardwareEncoder::SoftwareFallback);
    let upload_lock = state.upload_lock.clone();
    state.cancel_uploads.send_replace(false);
    let cancel = state.cancel_uploads.subscribe();
    tauri::async_runtime::spawn(encode::process_files(
        pack_state,
        paths,
        app,
        encoder,
        upload_lock,
        cancel,
    ));
    Ok(())
}

#[tauri::command]
pub async fn add_folder_dialog(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    use tauri_plugin_dialog::DialogExt;
    let app_c = app.clone();
    let folder = tokio::task::spawn_blocking(move || {
        app_c
            .dialog()
            .file()
            .set_title("Select folder")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(folder) = folder else { return Ok(()) };
    let folder: PathBuf = folder.into_path().map_err(|e| e.to_string())?;

    let paths = tokio::task::spawn_blocking(move || encode::explore_folder(&folder))
        .await
        .map_err(|e| e.to_string())?;

    if paths.is_empty() {
        return Ok(());
    }

    let pack_state = state.pack.clone();
    let encoder = state
        .hardware_encoder
        .get()
        .cloned()
        .unwrap_or(HardwareEncoder::SoftwareFallback);
    let upload_lock = state.upload_lock.clone();
    state.cancel_uploads.send_replace(false);
    let cancel = state.cancel_uploads.subscribe();
    tauri::async_runtime::spawn(encode::process_files(
        pack_state,
        paths,
        app,
        encoder,
        upload_lock,
        cancel,
    ));
    Ok(())
}

#[tauri::command]
pub async fn add_paths(
    state: State<'_, AppState>,
    app: AppHandle,
    paths: Vec<PathBuf>,
) -> Result<(), String> {
    let paths = tokio::task::spawn_blocking(move || {
        let mut result = Vec::new();
        for path in paths {
            if path.is_dir() {
                result.extend(encode::explore_folder(&path));
            } else if !encode::is_junk_path(&path) && encode::is_media_path(&path).unwrap_or(false)
            {
                result.push(path);
            }
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?;

    if paths.is_empty() {
        return Ok(());
    }

    let pack_state = state.pack.clone();
    let encoder = state
        .hardware_encoder
        .get()
        .cloned()
        .unwrap_or(HardwareEncoder::SoftwareFallback);
    let upload_lock = state.upload_lock.clone();
    state.cancel_uploads.send_replace(false);
    let cancel = state.cancel_uploads.subscribe();
    tauri::async_runtime::spawn(encode::process_files(
        pack_state,
        paths,
        app,
        encoder,
        upload_lock,
        cancel,
    ));
    Ok(())
}

#[tauri::command]
pub async fn cancel_upload(state: State<'_, AppState>) -> Result<(), String> {
    state.cancel_uploads.send_replace(true);
    Ok(())
}
