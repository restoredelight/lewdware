//! The media files in a pack: listing them, removing them, and their per-file attributes.

use shared::behaviour::Behaviour;
use tauri::State;

use crate::pack::MediaFile;
use crate::AppState;

#[tauri::command]
pub async fn get_files(state: State<'_, AppState>) -> Result<Vec<MediaFile>, String> {
    state
        .with_pack_or(vec![], async |pack| pack.get_files().await)
        .await
}

#[tauri::command]
pub async fn remove_files(
    state: State<'_, AppState>,
    ids: Vec<u64>,
) -> Result<Option<Behaviour>, String> {
    state
        .with_pack_or(None, async |pack| pack.remove_files(ids).await.map(Some))
        .await
}

/// Renames a media file. Nothing else moves with it: the behaviour document's media slots hold
/// ids, so a rename cannot invalidate one — which is why this returns nothing where it once had
/// to hand back a rewritten document (`design/behaviour-storage.md`, step 1).
#[tauri::command]
pub async fn set_file_title(
    state: State<'_, AppState>,
    id: u64,
    name: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.set_title(id, name).await)
        .await
}

#[tauri::command]
pub async fn set_file_source_url(
    state: State<'_, AppState>,
    id: u64,
    url: Option<String>,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.set_source_url(id, url).await)
        .await
}
