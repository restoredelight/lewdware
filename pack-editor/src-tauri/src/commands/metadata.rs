//! The pack's own name, author and version.

use tauri::State;

use super::MetadataDto;
use crate::AppState;

#[tauri::command]
pub async fn get_pack_metadata(state: State<'_, AppState>) -> Result<MetadataDto, String> {
    state
        .with_pack(async |pack| Ok(pack.metadata().into()))
        .await
}

#[tauri::command]
pub async fn set_pack_metadata(
    state: State<'_, AppState>,
    dto: MetadataDto,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.set_metadata(&dto.into()).await)
        .await
}

#[tauri::command]
pub async fn save_pack_metadata(state: State<'_, AppState>) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.save_metadata().await)
        .await
}

#[tauri::command]
pub async fn mark_pack_unsaved(state: State<'_, AppState>) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.mark_unsaved().await)
        .await
}
