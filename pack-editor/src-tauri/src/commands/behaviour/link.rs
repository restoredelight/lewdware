//! One web link the pack can open.

use shared::behaviour::editor;
use shared::behaviour::WebLink;
use tauri::State;

use super::setter;
use crate::pack::{BehaviourAction, BehaviourOutcome};
use crate::AppState;

setter!(
    set_web_link_url(id: i64, url: String) |tx| editor::set_web_link_url(tx, id, &url)
);

setter!(
    /// The suffixes appended at random when the link is opened. One field, because adding and
    /// removing one both rewrite the list.
    set_web_link_args(id: i64, args: Vec<String>) |tx| editor::set_web_link_args(tx, id, &args)
);

setter!(
    set_web_link_tags(id: i64, tags: Vec<String>) |tx| editor::set_web_link_tags(tx, id, &tags)
);

#[tauri::command]
pub async fn add_web_link(
    state: State<'_, AppState>,
    link: WebLink,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(
                label,
                move |tx: &rusqlite::Transaction<'_>| {
                    editor::add_web_link(tx, &link)?;
                    Ok(true)
                },
            ))
            .await
        })
        .await
}

setter!(
    remove_web_link(id: i64) |tx| editor::remove_web_link(tx, id)
);

setter!(
    add_web_link_tag(id: i64, tag: String) |tx| editor::add_web_link_tag(tx, id, &tag)
);

setter!(
    remove_web_link_tag(id: i64, tag: String) |tx| editor::remove_web_link_tag(tx, id, &tag)
);

setter!(
    /// Appends one suffix.
    add_web_link_arg(id: i64, value: String) |tx| editor::add_web_link_arg(tx, id, &value)
);

setter!(
    /// Removes the suffix at `position` — by position, because the same suffix may appear twice.
    remove_web_link_arg(id: i64, position: usize) |tx| {
        editor::remove_web_link_arg(tx, id, position)
    }
);
