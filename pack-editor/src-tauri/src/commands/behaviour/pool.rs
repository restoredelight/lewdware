//! One entry of a text pool: a caption, a prompt, or a notification.

use shared::behaviour::editor::{self, PoolKind};
use shared::behaviour::TextItem;
use tauri::State;

use super::setter;
use crate::pack::{BehaviourAction, BehaviourOutcome};
use crate::AppState;

setter!(
    set_text_item_text(id: i64, text: String) |tx| editor::set_text_item_text(tx, id, &text)
);

setter!(
    /// The notification's title. A blank one is the absence of a title, not a title of "".
    set_text_item_summary(id: i64, summary: Option<String>) |tx| {
        editor::set_text_item_summary(tx, id, summary.as_deref())
    }
);

setter!(
    /// A prompt's answer deadline. `None` asks the mode to derive one from the prompt's length.
    set_text_item_timeout(id: i64, seconds: Option<f64>) |tx| {
        editor::set_text_item_timeout(tx, id, seconds)
    }
);

setter!(
    set_text_item_tags(id: i64, tags: Vec<String>) |tx| editor::set_text_item_tags(tx, id, &tags)
);

#[tauri::command]
pub async fn add_text_item(
    state: State<'_, AppState>,
    kind: PoolKind,
    item: TextItem,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(
                label,
                move |tx: &rusqlite::Transaction<'_>| {
                    editor::add_text_item(tx, kind, &item)?;
                    Ok(true)
                },
            ))
            .await
        })
        .await
}

setter!(
    remove_text_item(id: i64) |tx| editor::remove_text_item(tx, id)
);

setter!(
    add_text_item_tag(id: i64, tag: String) |tx| editor::add_text_item_tag(tx, id, &tag)
);

setter!(
    remove_text_item_tag(id: i64, tag: String) |tx| editor::remove_text_item_tag(tx, id, &tag)
);
