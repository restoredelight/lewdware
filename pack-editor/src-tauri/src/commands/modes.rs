//! Modes embedded in a pack: listing them, adding one from disk, and removing one.

use std::{io::Cursor, path::PathBuf};

use serde::Serialize;
use shared::mode;
use tauri::{AppHandle, Emitter, State};

use crate::AppState;

#[derive(Serialize, Clone, Debug)]
pub struct EmbeddedMode {
    pub id: u64,
    pub stable_id: String,
    pub name: String,
    pub author: Option<String>,
    pub version: Option<String>,
    pub option_count: usize,
    pub size: u64,
}

pub fn embedded_mode(id: u64, bytes: &[u8]) -> Result<EmbeddedMode, String> {
    let (header, metadata) =
        mode::read_mode_metadata(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let option_count = metadata.all_options().len();
    Ok(EmbeddedMode {
        id,
        stable_id: header.id.to_string(),
        name: metadata.name,
        author: metadata.author,
        version: metadata.version,
        option_count,
        size: bytes.len() as u64,
    })
}

#[tauri::command]
pub async fn get_modes(state: State<'_, AppState>) -> Result<Vec<EmbeddedMode>, String> {
    let lock = state.pack.lock().await;
    let Some(pack) = lock.as_ref() else {
        return Ok(vec![]);
    };
    pack.get_modes()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|(id, bytes)| embedded_mode(id, &bytes))
        .collect()
}

#[tauri::command]
pub async fn add_mode_dialog(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<EmbeddedMode>, String> {
    use tauri_plugin_dialog::DialogExt;
    let app_c = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        app_c
            .dialog()
            .file()
            .set_title("Add custom mode")
            .add_filter("Lewdware Mode", &["lwmode"])
            .blocking_pick_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path: PathBuf = picked.into_path().map_err(|error| error.to_string())?;
    app.emit("picker:mode-selected", ())
        .map_err(|error| error.to_string())?;
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| error.to_string())?;
    let candidate = embedded_mode(0, &bytes)?;

    let lock = state.pack.lock().await;
    let Some(pack) = lock.as_ref() else {
        return Err("No pack is open".into());
    };
    for (id, existing) in pack.get_modes().await.map_err(|error| error.to_string())? {
        let current = embedded_mode(id, &existing)?;
        if current.stable_id == candidate.stable_id {
            return Err(format!(
                "“{}” is already included in this pack",
                current.name
            ));
        }
    }
    let id = pack
        .add_mode(bytes)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Some(EmbeddedMode { id, ..candidate }))
}

#[tauri::command]
pub async fn remove_mode(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.remove_mode(id).await)
        .await
}
