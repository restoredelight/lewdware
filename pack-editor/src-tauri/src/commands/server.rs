//! How the frontend reaches the local media server that streams a pack's files.

use tauri::State;

use crate::{AppState, MediaServerInfo};

#[tauri::command]
pub fn get_media_server(state: State<'_, AppState>) -> Result<MediaServerInfo, String> {
    state.media_server.get().cloned().ok_or_else(|| {
        "Media previews are unavailable because the local server failed to start".into()
    })
}
