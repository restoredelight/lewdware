//! The pack's own name, author and version.

use tauri::State;

use super::MetadataDto;
use crate::AppState;
use shared::pack::RecommendedMode;

#[tauri::command]
pub async fn get_pack_metadata(state: State<'_, AppState>) -> Result<MetadataDto, String> {
    state
        .with_pack(async |pack| Ok(pack.metadata().into()))
        .await
}

#[tauri::command]
pub async fn set_pack_name(
    state: State<'_, AppState>,
    value: String,
    label: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.set_metadata_name(value, label).await)
        .await
}

#[tauri::command]
pub async fn set_pack_creator(
    state: State<'_, AppState>,
    value: Option<String>,
    label: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| {
            pack.set_metadata_creator(value, label).await
        })
        .await
}

#[tauri::command]
pub async fn set_pack_description(
    state: State<'_, AppState>,
    value: Option<String>,
    label: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| {
            pack.set_metadata_description(value, label).await
        })
        .await
}

#[tauri::command]
pub async fn set_pack_version(
    state: State<'_, AppState>,
    value: Option<String>,
    label: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| {
            pack.set_metadata_version(value, label).await
        })
        .await
}

#[tauri::command]
pub async fn set_pack_recommended_mode(
    state: State<'_, AppState>,
    value: Option<RecommendedMode>,
    label: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| {
            pack.set_metadata_recommended_mode(value, label).await
        })
        .await
}

#[tauri::command]
pub async fn mark_pack_unsaved(state: State<'_, AppState>) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.mark_unsaved().await)
        .await
}
