use serde::{Deserialize, Serialize};
use shared::pack::RecommendedMode;
use shared::user_config::Mode;
use tauri::{AppHandle, Emitter};

use crate::dto::{ModeGroupDto, ModeIdDto, PackMetadataDto};
use crate::modes::{build_mode_groups, load_pack, save_to_disk};
use crate::state::State;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PickPackResult {
    pub pack_path: String,
    pub pack_metadata: PackMetadataDto,
    pub mode_groups: Vec<ModeGroupDto>,
    pub first_mode: Option<ModeIdDto>,
}

#[tauri::command]
pub async fn pick_pack(
    app_handle: AppHandle,
    state: State<'_>,
) -> Result<Option<PickPackResult>, String> {
    use tauri_plugin_dialog::DialogExt;

    let path = app_handle
        .dialog()
        .file()
        .add_filter("Pack", &["lwpack"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok());

    let Some(path) = path else {
        return Ok(None);
    };
    app_handle
        .emit("picker:pack-selected", ())
        .map_err(|e| e.to_string())?;

    let loaded = tokio::task::spawn_blocking({
        let path = path.clone();
        move || load_pack(path)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    // Preselect a mode for the newly-picked pack, same "new pack resets mode selection"
    // precedent for both cases: an embedded pack mode wins if present; otherwise nudge toward
    // whichever built-in default mode the pack author recommends (falling back to Sandbox --
    // see `behaviour-design/default-mode.md`: "a plain content pack recommends Sandbox").
    let first_mode = loaded
        .modes
        .first()
        .map(|m| ModeIdDto::Pack { id: m.id })
        .or(Some(match loaded.recommended_mode {
            Some(RecommendedMode::Experience) => ModeIdDto::Experience,
            _ => ModeIdDto::Sandbox,
        }));

    let pack_path_str = path.to_string_lossy().into_owned();
    let pack_metadata = PackMetadataDto::from(&loaded.metadata);
    *state.pack.lock().unwrap() = Some(loaded);

    let mut config = state.config.lock().unwrap();
    config.pack_path = Some(path);
    if let Some(ref m) = first_mode {
        config.mode = m.clone().into();
    }

    let groups = build_mode_groups(&state);
    let uploaded = state.uploaded.lock().unwrap();
    save_to_disk(&config, &uploaded).map_err(|e| e.to_string())?;

    Ok(Some(PickPackResult {
        pack_path: pack_path_str,
        pack_metadata,
        mode_groups: groups,
        first_mode,
    }))
}

#[tauri::command]
pub fn get_pack_metadata(state: State<'_>) -> Option<PackMetadataDto> {
    state
        .pack
        .lock()
        .unwrap()
        .as_ref()
        .map(|pack| PackMetadataDto::from(&pack.metadata))
}

#[tauri::command]
pub fn remove_pack(state: State<'_>) -> Result<(), String> {
    *state.pack.lock().unwrap() = None;
    let mut config = state.config.lock().unwrap();
    config.pack_path = None;
    if matches!(config.mode, Mode::Pack { .. }) {
        config.mode = Mode::default();
    }
    let uploaded = state.uploaded.lock().unwrap();
    save_to_disk(&config, &uploaded).map_err(|e| e.to_string())
}
