//! Tags and artists, on one file or across a whole selection.

use tauri::State;

use crate::pack::{ArtistSummary, TagRow, TagSummary};
use crate::AppState;

#[tauri::command]
pub async fn get_all_tags(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    state
        .with_pack_or(vec![], async |pack| pack.get_all_tags().await)
        .await
}

#[tauri::command]
pub async fn get_file_tags(state: State<'_, AppState>, id: u64) -> Result<Vec<String>, String> {
    state
        .with_pack_or(vec![], async |pack| pack.get_tags(id).await)
        .await
}

#[tauri::command]
pub async fn add_tag_to_file(
    state: State<'_, AppState>,
    id: u64,
    tag: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.add_tag(id, tag).await)
        .await
}

#[tauri::command]
pub async fn remove_tag_from_file(
    state: State<'_, AppState>,
    id: u64,
    tag: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.remove_tag(id, tag).await)
        .await
}

#[tauri::command]
pub async fn create_and_add_tag(
    state: State<'_, AppState>,
    id: u64,
    tag: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.create_and_add_tag(id, tag).await)
        .await
}

#[tauri::command]
pub async fn get_tag_summaries(state: State<'_, AppState>) -> Result<Vec<TagSummary>, String> {
    state
        .with_pack_or(vec![], async |pack| pack.get_tag_summaries().await)
        .await
}

#[tauri::command]
pub async fn get_tag_rows(state: State<'_, AppState>) -> Result<Vec<TagRow>, String> {
    state
        .with_pack(async |pack| pack.get_tag_rows().await)
        .await
}

/// Every media slot pointing at `id`, named the way the author would recognize it.
#[tauri::command]
pub async fn get_media_usage(state: State<'_, AppState>, id: u64) -> Result<Vec<String>, String> {
    state
        .with_pack_or(vec![], async |pack| pack.get_media_usage(id).await)
        .await
}

#[tauri::command]
pub async fn rename_tag(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<(), String> {
    state
        .with_pack(async |pack| pack.rename_tag(from, to).await)
        .await
}

#[tauri::command]
pub async fn merge_tag(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<(), String> {
    state
        .with_pack(async |pack| pack.merge_tag(from, to).await)
        .await
}

#[tauri::command]
pub async fn delete_tag(state: State<'_, AppState>, tag: String) -> Result<(), String> {
    state
        .with_pack(async |pack| pack.delete_tag(tag).await)
        .await
}

#[tauri::command]
pub async fn add_tag_to_files(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    tag: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.add_tag_to_files(ids, tag).await)
        .await
}

#[tauri::command]
pub async fn remove_tag_from_files(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    tag: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.remove_tag_from_files(ids, tag).await)
        .await
}

// ── Artists ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_all_artists(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    state
        .with_pack_or(vec![], async |pack| pack.get_all_artists().await)
        .await
}

#[tauri::command]
pub async fn get_file_artists(state: State<'_, AppState>, id: u64) -> Result<Vec<String>, String> {
    state
        .with_pack_or(vec![], async |pack| pack.get_artists(id).await)
        .await
}

#[tauri::command]
pub async fn add_artist_to_file(
    state: State<'_, AppState>,
    id: u64,
    artist: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.add_artist(id, artist).await)
        .await
}

#[tauri::command]
pub async fn remove_artist_from_file(
    state: State<'_, AppState>,
    id: u64,
    artist: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.remove_artist(id, artist).await)
        .await
}

#[tauri::command]
pub async fn create_and_add_artist(
    state: State<'_, AppState>,
    id: u64,
    artist: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| {
            pack.create_and_add_artist(id, artist).await
        })
        .await
}

#[tauri::command]
pub async fn get_artist_summaries(
    state: State<'_, AppState>,
) -> Result<Vec<ArtistSummary>, String> {
    state
        .with_pack_or(vec![], async |pack| pack.get_artist_summaries().await)
        .await
}

#[tauri::command]
pub async fn rename_artist(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.rename_artist(from, to).await)
        .await
}

#[tauri::command]
pub async fn merge_artist(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.merge_artist(from, to).await)
        .await
}

#[tauri::command]
pub async fn delete_artist(state: State<'_, AppState>, artist: String) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.delete_artist(artist).await)
        .await
}

#[tauri::command]
pub async fn add_artist_to_files(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    artist: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.add_artist_to_files(ids, artist).await)
        .await
}

#[tauri::command]
pub async fn remove_artist_from_files(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    artist: String,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| {
            pack.remove_artist_from_files(ids, artist).await
        })
        .await
}
