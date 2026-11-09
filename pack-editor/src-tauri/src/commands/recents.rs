//! The recently-opened list, as commands. The list itself lives in `pack_editor_core::recents`,
//! shared with the gpui editor.

use pack_editor_core::recents;
pub use pack_editor_core::recents::RecentPack;
use tauri::State;

use crate::AppState;

fn text(error: anyhow::Error) -> String {
    error.to_string()
}

pub fn remember_pack(path: &std::path::Path, name: String) -> Result<(), String> {
    recents::remember_pack(path, name).map_err(text)
}

pub fn remember_draft(id: uuid::Uuid, name: String) -> Result<(), String> {
    recents::remember_draft(id, name).map_err(text)
}

pub fn forget_draft(id: uuid::Uuid) -> Result<(), String> {
    recents::forget_draft(id).map_err(text)
}

#[tauri::command]
pub fn get_recent_packs() -> Vec<RecentPack> {
    recents::list()
}

#[tauri::command]
pub async fn remove_recent_pack(
    state: State<'_, AppState>,
    path: Option<String>,
    draft_id: Option<String>,
) -> Result<(), String> {
    let draft_id = draft_id
        .map(|id| uuid::Uuid::parse_str(&id).map_err(|error| error.to_string()))
        .transpose()?;
    if let Some(id) = draft_id {
        let lock = state.pack.lock().await;
        if lock.as_ref().is_some_and(|pack| pack.id() == id) {
            return Err("Cannot remove the draft while it is open".into());
        }
    }
    recents::remove(path.as_deref(), draft_id).map_err(text)
}
