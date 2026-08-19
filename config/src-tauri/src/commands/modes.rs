use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use shared::mode::StoredValue;
use shared::user_config::Mode;
use tauri::{AppHandle, Emitter};

use crate::dto::{ModeGroupDto, ModeOptionsDto};
use crate::modes::{build_mode_groups, get_mode_options_for, load_mode_file, save_to_disk};
use crate::state::State;

#[tauri::command]
pub fn get_mode_groups(state: State<'_>) -> Vec<ModeGroupDto> {
    build_mode_groups(&state)
}

#[tauri::command]
pub fn get_mode_options(state: State<'_>) -> ModeOptionsDto {
    let config = state.config.lock().unwrap();
    get_mode_options_for(&config, &state)
}

/// Stores an option value as the frontend sent it. No schema lookup: `StoredValue` holds the
/// JSON shapes as-is, and which `OptionType` the value belongs to is decided on the way back
/// out, by `resolve_options` against the mode's schema. The frontend could not make that
/// call correctly anyway -- JavaScript has one number type.
#[tauri::command]
pub fn set_mode_option(state: State<'_>, key: String, value: JsonValue) -> Result<(), String> {
    let mut config = state.config.lock().unwrap();
    let mode = config.mode.clone();

    let typed_value: StoredValue =
        serde_json::from_value(value).map_err(|_| "invalid option value".to_string())?;

    if matches!(mode, Mode::Experience) {
        let pack_id = state
            .pack
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.id)
            .ok_or_else(|| "no pack loaded".to_string())?;
        config
            .experience_options
            .entry(pack_id)
            .or_default()
            .insert(key, typed_value);
    } else {
        config
            .mode_options
            .entry(mode)
            .or_default()
            .insert(key, typed_value);
    }
    let uploaded = state.uploaded.lock().unwrap();
    save_to_disk(&config, &uploaded).map_err(|e| e.to_string())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UploadModeResult {
    pub mode_groups: Vec<ModeGroupDto>,
}

#[tauri::command]
pub async fn upload_mode(
    app_handle: AppHandle,
    state: State<'_>,
) -> Result<Option<UploadModeResult>, String> {
    use tauri_plugin_dialog::DialogExt;

    let path = app_handle
        .dialog()
        .file()
        .add_filter("Mode", &["lwmode"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok());

    let Some(path) = path else {
        return Ok(None);
    };
    app_handle
        .emit("picker:mode-selected", ())
        .map_err(|e| e.to_string())?;

    // Note the guard is released before `build_mode_groups`, which locks `uploaded` itself --
    // holding it across the call self-deadlocks (`Mutex` is not reentrant) and hangs the UI.
    let already_uploaded = state
        .uploaded
        .lock()
        .unwrap()
        .iter()
        .any(|u| u.path == path);
    if already_uploaded {
        return Ok(Some(UploadModeResult {
            mode_groups: build_mode_groups(&state),
        }));
    }

    let entry = tokio::task::spawn_blocking({
        let path = path.clone();
        move || load_mode_file(path)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    state.uploaded.lock().unwrap().push(entry);

    {
        let mut config = state.config.lock().unwrap();
        config.uploaded_modes.push(path);
        let uploaded = state.uploaded.lock().unwrap();
        save_to_disk(&config, &uploaded).map_err(|e| e.to_string())?;
    }

    Ok(Some(UploadModeResult {
        mode_groups: build_mode_groups(&state),
    }))
}

#[tauri::command]
pub fn remove_uploaded_mode(state: State<'_>, path: String) -> Result<Vec<ModeGroupDto>, String> {
    let path = PathBuf::from(&path);

    state.uploaded.lock().unwrap().retain(|u| u.path != path);

    {
        let mut config = state.config.lock().unwrap();
        config.uploaded_modes.retain(|p| p != &path);
        if let Mode::File { path: ref mp, .. } = config.mode.clone() {
            if mp == &path {
                config.mode = Mode::default();
            }
        }
        let uploaded = state.uploaded.lock().unwrap();
        save_to_disk(&config, &uploaded).map_err(|e| e.to_string())?;
    }

    Ok(build_mode_groups(&state))
}
