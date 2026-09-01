//! Fine-grained edits to captions, prompts, notifications, and web links.

use shared::behaviour::editor;
use shared::behaviour::{Caption, Notification, Prompt, WebLink};
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
    set_notification_body(id: i64, text: String) |tx| {
        editor::set_notification_body(tx, id, &text)
    }
);
setter!(
    set_notification_summary(id: i64, summary: Option<String>) |tx| {
        editor::set_notification_summary(tx, id, summary.as_deref())
    }
);
setter!(
    set_prompt_timeout(id: i64, seconds: Option<f64>) |tx| {
        editor::set_prompt_timeout(tx, id, seconds)
    }
);
setter!(
    set_web_link_url(id: i64, url: String) |tx| editor::set_web_link_url(tx, id, &url)
);

macro_rules! add_entry_command {
    ($name:ident, $argument:ident: $ty:ty, $add:path) => {
        #[tauri::command]
        pub async fn $name(
            state: State<'_, AppState>,
            $argument: $ty,
            label: String,
        ) -> Result<BehaviourOutcome, String> {
            state
                .with_pack(async |pack| {
                    pack.behaviour_edit(BehaviourAction::new(
                        label,
                        move |tx: &rusqlite::Transaction<'_>| $add(tx, &$argument).map(|_| ()),
                    ))
                    .await
                })
                .await
        }
    };
}

add_entry_command!(add_caption, caption: Caption, editor::add_caption);
add_entry_command!(add_prompt, prompt: Prompt, editor::add_prompt);
add_entry_command!(
    add_notification,
    notification: Notification,
    editor::add_notification
);
add_entry_command!(add_web_link, link: WebLink, editor::add_web_link);

setter!(remove_caption(id: i64) |tx| editor::remove_caption(tx, id));
setter!(remove_prompt(id: i64) |tx| editor::remove_prompt(tx, id));
setter!(remove_notification(id: i64) |tx| editor::remove_notification(tx, id));
setter!(remove_web_link(id: i64) |tx| editor::remove_web_link(tx, id));

setter!(
    add_caption_tag(id: i64, tag: String) |tx| editor::add_caption_tag(tx, id, &tag)
);
setter!(
    remove_caption_tag(id: i64, tag: String) |tx| editor::remove_caption_tag(tx, id, &tag)
);
setter!(
    add_prompt_tag(id: i64, tag: String) |tx| editor::add_prompt_tag(tx, id, &tag)
);
setter!(
    remove_prompt_tag(id: i64, tag: String) |tx| editor::remove_prompt_tag(tx, id, &tag)
);
setter!(
    add_notification_tag(id: i64, tag: String) |tx| editor::add_notification_tag(tx, id, &tag)
);
setter!(
    remove_notification_tag(id: i64, tag: String) |tx| {
        editor::remove_notification_tag(tx, id, &tag)
    }
);
setter!(
    add_web_link_tag(id: i64, tag: String) |tx| editor::add_web_link_tag(tx, id, &tag)
);
setter!(
    remove_web_link_tag(id: i64, tag: String) |tx| editor::remove_web_link_tag(tx, id, &tag)
);
