//! One content group: a named, described set of tags the player can switch off.

use shared::behaviour::editor;
use shared::behaviour::ContentGroup;
use tauri::State;

use super::setter;
use crate::pack::{BehaviourAction, BehaviourOutcome};
use crate::AppState;

setter!(
    set_content_group_label(id: String, value: String) |tx| {
        editor::set_content_group_label(tx, &id, &value)
    }
);

setter!(
    /// A blank description is the absence of one.
    set_content_group_description(id: String, description: Option<String>) |tx| {
        editor::set_content_group_description(tx, &id, description.as_deref())
    }
);

setter!(
    set_content_group_enabled_by_default(id: String, enabled: bool) |tx| {
        editor::set_content_group_enabled_by_default(tx, &id, enabled)
    }
);

setter!(
    set_content_group_tags(id: String, tags: Vec<String>) |tx| {
        editor::set_content_group_tags(tx, &id, &tags)
    }
);

#[tauri::command]
pub async fn add_content_group(
    state: State<'_, AppState>,
    group: ContentGroup,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(
                label,
                move |tx: &rusqlite::Transaction<'_>| {
                    editor::add_content_group(tx, &group)?;
                    Ok(true)
                },
            ))
            .await
        })
        .await
}

setter!(
    remove_content_group(id: String) |tx| editor::remove_content_group(tx, &id)
);
