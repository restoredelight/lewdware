//! Reading and patching the behaviour document the Content and Experience tabs edit.

use shared::behaviour::{Behaviour, Patch};
use tauri::State;

use crate::pack::{BehaviourEdit, TagAction};
use crate::AppState;

#[tauri::command]
pub async fn get_behaviour(state: State<'_, AppState>) -> Result<Behaviour, String> {
    state
        .with_pack(async |pack| pack.get_behaviour().await)
        .await
}

/// Applies one author action to the behaviour document and hands back the stored result.
///
/// Takes patches rather than a document: the backend is the only writer of behaviour, so the
/// front end describes what it changed instead of sending the copy it holds. `retiring` names
/// media the action deliberately lets go of, and `tag_actions` the tag edits that belong to the
/// same author action, so that dropping a stage and dropping the wallpaper and the tag that
/// existed only for it are one transaction and one undo entry. See [`MediaPack::edit_behaviour`].
#[tauri::command]
pub async fn edit_behaviour(
    state: State<'_, AppState>,
    patches: Vec<Patch>,
    label: String,
    retiring: Vec<u64>,
    tag_actions: Vec<TagAction>,
) -> Result<BehaviourEdit, String> {
    state
        .with_pack(async |pack| {
            pack.edit_behaviour(patches, label, retiring, tag_actions)
                .await
        })
        .await
}
