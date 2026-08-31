//! One entry of a text pool: a caption, a prompt, or a notification.

use shared::behaviour::editor;
use shared::behaviour::{Caption, Notification, Prompt};
use tauri::State;

use super::setter;
use crate::pack::{BehaviourAction, BehaviourOutcome};
use crate::AppState;

setter!(
    set_caption_text(id: i64, text: String) |tx| editor::set_caption_text(tx, id, &text)
);

setter!(
    set_prompt_text(id: i64, text: String) |tx| editor::set_prompt_text(tx, id, &text)
);

setter!(
    /// The notification's body, as opposed to its optional summary title.
    set_notification_body(id: i64, text: String) |tx| {
        editor::set_notification_body(tx, id, &text)
    }
);

setter!(
    /// The notification's title. A blank one is the absence of a title, not a title of "".
    set_notification_summary(id: i64, summary: Option<String>) |tx| {
        editor::set_notification_summary(tx, id, summary.as_deref())
    }
);

setter!(
    /// A prompt's answer deadline. `None` asks the mode to derive one from the prompt's length.
    set_prompt_timeout(id: i64, seconds: Option<f64>) |tx| {
        editor::set_prompt_timeout(tx, id, seconds)
    }
);

#[tauri::command]
pub async fn add_caption(
    state: State<'_, AppState>,
    caption: Caption,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(
                label,
                move |tx: &rusqlite::Transaction<'_>| editor::add_caption(tx, &caption).map(|_| ()),
            ))
            .await
        })
        .await
}

#[tauri::command]
pub async fn add_prompt(
    state: State<'_, AppState>,
    prompt: Prompt,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(
                label,
                move |tx: &rusqlite::Transaction<'_>| editor::add_prompt(tx, &prompt).map(|_| ()),
            ))
            .await
        })
        .await
}

#[tauri::command]
pub async fn add_notification(
    state: State<'_, AppState>,
    notification: Notification,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(
                label,
                move |tx: &rusqlite::Transaction<'_>| {
                    editor::add_notification(tx, &notification).map(|_| ())
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
