//! The behaviour document, as typed queries and typed mutations.
//!
//! Two rules shape this surface, and both are reactions to what it replaced (a single
//! `edit_behaviour` taking dot-separated path strings and a JSON value — see
//! `design/editor-data-flow.md`):
//!
//! - **A query serves a view.** Each returns what one surface renders, so the Content tab does not
//!   read the timeline to count its captions and the media inspector does not hold the document to
//!   answer "is this file the wallpaper?".
//! - **A mutation is one author action**, addressed by the id of the thing it changes, and is one
//!   transaction and one undo entry. Nothing here takes a whole document, and nothing here returns
//!   one: the front end fetches what it renders.
//!
//! `label` is the author's word for what they did — it becomes the undo entry, so it is UI copy and
//! belongs to the front end. It addresses nothing.

use shared::behaviour::editor::{
    self, AudioChanges, MediaSlots, PoolKind, PopupChanges, TextItemRow, WebLinkRow,
};
use shared::behaviour::{AudioMedia, ContentGroup, PopupMedia, Stage, TextItem, Transition, WebLink};
use rusqlite::Transaction;
use tauri::State;

use crate::pack::{BehaviourAction, BehaviourOutcome, TagAction};
use crate::AppState;

/// What the Content tab's badges and the Experience tab's header need, without reading either
/// surface's contents.
#[derive(serde::Serialize)]
pub struct BehaviourSummary {
    captions: usize,
    prompts: usize,
    notifications: usize,
    web_links: usize,
    content_groups: usize,
    /// Whether the pack has a timeline section at all.
    has_timeline: bool,
    /// Whether it plays: a timeline the author switched off is present but disabled.
    timeline_enabled: bool,
    /// The timeline's mode-name override, if any.
    timeline_label: Option<String>,
}

#[tauri::command]
pub async fn get_behaviour_summary(state: State<'_, AppState>) -> Result<BehaviourSummary, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(|conn: &rusqlite::Connection| {
                let timeline = editor::timeline(conn)?;
                Ok(BehaviourSummary {
                    captions: editor::text_pool(conn, PoolKind::Caption)?.len(),
                    prompts: editor::text_pool(conn, PoolKind::Prompt)?.len(),
                    notifications: editor::text_pool(conn, PoolKind::Notification)?.len(),
                    web_links: editor::web_links(conn)?.len(),
                    content_groups: editor::content_groups(conn)?.len(),
                    has_timeline: timeline.is_some(),
                    timeline_enabled: timeline.as_ref().is_some_and(|view| view.enabled),
                    timeline_label: timeline.and_then(|view| view.label),
                })
            })
            .await
        })
        .await
}

// ── Text pools ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_text_pool(
    state: State<'_, AppState>,
    kind: PoolKind,
) -> Result<Vec<TextItemRow>, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(move |conn: &rusqlite::Connection| editor::text_pool(conn, kind))
                .await
        })
        .await
}

#[tauri::command]
pub async fn add_text_item(
    state: State<'_, AppState>,
    kind: PoolKind,
    item: TextItem,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::add_text_item(tx, kind, &item)?;
                Ok(true)
            }))
            .await
        })
        .await
}

#[tauri::command]
pub async fn update_text_item(
    state: State<'_, AppState>,
    id: i64,
    item: TextItem,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::update_text_item(tx, id, &item)
            }))
            .await
        })
        .await
}

#[tauri::command]
pub async fn remove_text_item(
    state: State<'_, AppState>,
    id: i64,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::remove_text_item(tx, id)
            }))
            .await
        })
        .await
}

#[tauri::command]
pub async fn reorder_text_items(
    state: State<'_, AppState>,
    kind: PoolKind,
    ids: Vec<i64>,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::reorder_text_items(tx, kind, &ids)
            }))
            .await
        })
        .await
}

// ── Web links ────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_web_links(state: State<'_, AppState>) -> Result<Vec<WebLinkRow>, String> {
    state
        .with_pack(async |pack| pack.behaviour_query(editor::web_links).await)
        .await
}

#[tauri::command]
pub async fn add_web_link(
    state: State<'_, AppState>,
    link: WebLink,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::add_web_link(tx, &link)?;
                Ok(true)
            }))
            .await
        })
        .await
}

#[tauri::command]
pub async fn update_web_link(
    state: State<'_, AppState>,
    id: i64,
    link: WebLink,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::update_web_link(tx, id, &link)
            }))
            .await
        })
        .await
}

#[tauri::command]
pub async fn remove_web_link(
    state: State<'_, AppState>,
    id: i64,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::remove_web_link(tx, id)
            }))
            .await
        })
        .await
}

// ── Content groups ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_content_groups(state: State<'_, AppState>) -> Result<Vec<ContentGroup>, String> {
    state
        .with_pack(async |pack| pack.behaviour_query(editor::content_groups).await)
        .await
}

#[tauri::command]
pub async fn add_content_group(
    state: State<'_, AppState>,
    group: ContentGroup,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::add_content_group(tx, &group)?;
                Ok(true)
            }))
            .await
        })
        .await
}

#[tauri::command]
pub async fn update_content_group(
    state: State<'_, AppState>,
    id: String,
    group: ContentGroup,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::update_content_group(tx, &id, &group)
            }))
            .await
        })
        .await
}

#[tauri::command]
pub async fn remove_content_group(
    state: State<'_, AppState>,
    id: String,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::remove_content_group(tx, &id)
            }))
            .await
        })
        .await
}

// ── Media slots ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_media_slots(state: State<'_, AppState>) -> Result<MediaSlots, String> {
    state
        .with_pack(async |pack| pack.behaviour_query(editor::media_slots).await)
        .await
}

// ── Per-file attributes ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_popup_attributes(
    state: State<'_, AppState>,
    ids: Vec<u64>,
) -> Result<Vec<(u64, PopupMedia)>, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(move |conn: &rusqlite::Connection| editor::popup_attributes(conn, &ids))
                .await
        })
        .await
}

#[tauri::command]
pub async fn get_audio_attributes(
    state: State<'_, AppState>,
    ids: Vec<u64>,
) -> Result<Vec<(u64, AudioMedia)>, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(move |conn: &rusqlite::Connection| editor::audio_attributes(conn, &ids))
                .await
        })
        .await
}

#[tauri::command]
pub async fn set_popup_attributes(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    changes: PopupChanges,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::set_popup_attributes(tx, &ids, &changes)
            }))
            .await
        })
        .await
}

#[tauri::command]
pub async fn set_audio_attributes(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    changes: AudioChanges,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::set_audio_attributes(tx, &ids, &changes)
            }))
            .await
        })
        .await
}

// ── The timeline ─────────────────────────────────────────────────────────────

/// The timeline as the editor shows it: present even while switched off.
#[derive(serde::Serialize)]
pub struct TimelineDto {
    stages: Vec<Stage>,
    transitions: Vec<Transition>,
    enabled: bool,
    label: Option<String>,
}

#[tauri::command]
pub async fn get_timeline(state: State<'_, AppState>) -> Result<Option<TimelineDto>, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(|conn: &rusqlite::Connection| {
                Ok(editor::timeline(conn)?.map(|view| TimelineDto {
                    stages: view.timeline.stages,
                    transitions: view.timeline.transitions,
                    enabled: view.enabled,
                    label: view.label,
                }))
            })
            .await
        })
        .await
}

/// Switches the timeline on or off, keeping its stages either way.
///
/// Nothing is retired: switching off drops every stage *on purpose*, and a cleanup driven by "what
/// stopped being referenced?" would delete every stage wallpaper the moment the author toggled it.
#[tauri::command]
pub async fn set_timeline_enabled(
    state: State<'_, AppState>,
    enabled: bool,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::set_experience_enabled(tx, enabled)
            }))
            .await
        })
        .await
}

#[tauri::command]
pub async fn set_timeline_label(
    state: State<'_, AppState>,
    value: Option<String>,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                let before = editor::timeline(tx)?.and_then(|view| view.label);
                if before == value {
                    return Ok(false);
                }
                editor::set_experience_label(tx, value.as_deref())?;
                Ok(true)
            }))
            .await
        })
        .await
}

/// Adds a stage after `after`, copying `source`'s settings if given.
#[tauri::command]
pub async fn add_stage(
    state: State<'_, AppState>,
    after: Option<String>,
    source: Option<String>,
    name: String,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::add_stage(tx, after.as_deref(), source.as_deref(), &name)?;
                Ok(true)
            }))
            .await
        })
        .await
}

#[tauri::command]
pub async fn duplicate_stage(
    state: State<'_, AppState>,
    id: String,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::duplicate_stage(tx, &id)?;
                Ok(true)
            }))
            .await
        })
        .await
}

#[tauri::command]
pub async fn move_stage(
    state: State<'_, AppState>,
    id: String,
    to: usize,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::move_stage(tx, &id, to)
            }))
            .await
        })
        .await
}

/// Removes a stage, retiring the media and the tag that existed only for it.
///
/// `retiring` and `tag_actions` are what makes this one undo entry rather than three: the stage,
/// its wallpaper and its owned tag go together, so one undo brings back the stage *with* its
/// wallpaper rather than a stage the author never created.
#[tauri::command]
pub async fn remove_stage(
    state: State<'_, AppState>,
    id: String,
    retiring: Vec<u64>,
    tag_actions: Vec<TagAction>,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(
                BehaviourAction::new(label, move |tx: &Transaction<'_>| editor::remove_stage(tx, &id))
                    .retiring(retiring)
                    .with_tag_actions(tag_actions),
            )
            .await
        })
        .await
}

/// One stage's new settings, as [`update_stages`] takes them.
#[derive(serde::Deserialize)]
pub struct StageUpdate {
    id: String,
    stage: Stage,
}

/// Replaces the settings of one or more stages.
///
/// A list rather than a single stage because one author action can legitimately touch several. The
/// case that forces it: taking a file out of one timeline stage, when a stage that shared the tag
/// would lose the file too — the editor gives that other stage a fresh tag of its own so its
/// membership survives. That is one thing the author did, so it is one transaction and one undo
/// entry, not one per stage the rewrite happened to reach.
///
/// `tag_actions` carries the tag half of the same action — renaming a stage renames the tag it
/// owns, and switching its tag restriction on seeds that tag onto the media the stage was showing a
/// moment before. Both have to be in this transaction or undo comes apart.
#[tauri::command]
pub async fn update_stages(
    state: State<'_, AppState>,
    updates: Vec<StageUpdate>,
    retiring: Vec<u64>,
    tag_actions: Vec<TagAction>,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(
                BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                    let mut changed = false;
                    for update in &updates {
                        changed |= editor::update_stage(tx, &update.id, &update.stage)?;
                    }
                    Ok(changed)
                })
                .retiring(retiring)
                .with_tag_actions(tag_actions),
            )
            .await
        })
        .await
}

#[tauri::command]
pub async fn update_transition(
    state: State<'_, AppState>,
    id: String,
    transition: Transition,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::update_transition(tx, &id, &transition)
            }))
            .await
        })
        .await
}
