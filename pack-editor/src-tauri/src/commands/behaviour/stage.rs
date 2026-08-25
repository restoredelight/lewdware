//! One timeline stage: its own fields, and the structural edits that move it in the timeline.
//!
//! Three commands here take more than a field's worth of argument, and each earns it:
//!
//! - `set_stage_content_tags` writes the selection *and* which of those tags the editor owns,
//!   because ownership is recorded on the association row — a stage cannot own a tag it does not
//!   select by, and the two set separately would allow a moment where it does.
//! - `set_stage_movement` and `set_stage_mitosis` carry the whole sub-structure, because one click
//!   creates both of its values. As separate commands that would be several undo entries for one
//!   action, and would need a grouping mechanism to undo the granularity it just bought.
//! - `remove_stage` and the rename carry `retiring` and `tag_actions`, so a stage, the wallpaper
//!   that was only ever its scenery, and the tag that existed only for it go in one transaction.

use rusqlite::Transaction;
use shared::behaviour::editor;
use shared::behaviour::{EndStrategy, EventCountCondition, EventKind, EventSchedule, Mitosis, Movement};
use tauri::State;

use super::setter;
use crate::pack::{BehaviourAction, BehaviourOutcome, TagAction};
use crate::AppState;

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


// ── The stage's own fields ───────────────────────────────────────────────────

/// Renames the stage, and renames the tag it owns along with it.
///
/// One command because they are one author action: the editor names a stage's tag after the stage,
/// so letting the two land separately would leave a rename half-applied under undo. `tag_actions`
/// is empty for a stage that owns nothing.
#[tauri::command]
pub async fn set_stage_label(
    state: State<'_, AppState>,
    id: String,
    value: String,
    tag_actions: Vec<TagAction>,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(
                BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                    editor::set_stage_label(tx, &id, &value)
                })
                .with_tag_actions(tag_actions),
            )
            .await
        })
        .await
}

/// Which media the stage selects, and which of those tags the editor maintains the name of.
///
/// `tags` of `None` is a stage that restricts nothing — and therefore owns nothing, since there is
/// no selection for a tag to be part of. `tag_actions` carries the seeding that switching a
/// restriction on needs: the stage's new tag goes onto the media it was showing a moment before, so
/// behaviour is preserved rather than the stage going from everything to nothing in one checkbox.
#[tauri::command]
pub async fn set_stage_content_tags(
    state: State<'_, AppState>,
    id: String,
    tags: Option<Vec<String>>,
    owned_tag: Option<String>,
    tag_actions: Vec<TagAction>,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(
                BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                    editor::set_stage_content_tags(tx, &id, tags.as_deref(), owned_tag.as_deref())
                })
                .with_tag_actions(tag_actions),
            )
            .await
        })
        .await
}

/// The same, across several stages at once.
///
/// The one genuinely multi-entity action: taking a file out of one stage gives any stage that
/// shared its tag a fresh tag of its own, so the file stays where it was. That is one thing the
/// author did, so it is one transaction and one undo entry.
#[tauri::command]
pub async fn set_stage_content_tags_many(
    state: State<'_, AppState>,
    updates: Vec<StageTagUpdate>,
    tag_actions: Vec<TagAction>,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(
                BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                    let mut changed = false;
                    for update in &updates {
                        changed |= editor::set_stage_content_tags(
                            tx,
                            &update.id,
                            update.tags.as_deref(),
                            update.owned_tag.as_deref(),
                        )?;
                    }
                    Ok(changed)
                })
                .with_tag_actions(tag_actions),
            )
            .await
        })
        .await
}

/// One stage's selection, as [`set_stage_content_tags_many`] takes it.
#[derive(serde::Deserialize)]
pub struct StageTagUpdate {
    id: String,
    tags: Option<Vec<String>>,
    owned_tag: Option<String>,
}

/// Whether the stage picks a fresh background track from its own tags when it begins.
///
/// Switching this on clears any named track — the two are alternatives. `retiring` names the track
/// being let go of, so a sound that was only ever this stage's scenery leaves with it.
#[tauri::command]
pub async fn set_stage_audio_random(
    state: State<'_, AppState>,
    id: String,
    random: bool,
    retiring: Vec<u64>,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(
                BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                    editor::set_stage_audio_random(tx, &id, random)
                })
                .retiring(retiring),
            )
            .await
        })
        .await
}

setter!(
    /// One event kind's schedule, or `None` to stop the stage scheduling that kind at all.
    set_stage_event(id: String, kind: EventKind, schedule: Option<EventSchedule>) |tx| {
        editor::set_stage_event(tx, &id, kind, schedule.as_ref())
    }
);

setter!(
    set_stage_entry_notification(id: String, text: Option<String>) |tx| {
        editor::set_stage_entry_notification(tx, &id, text.as_deref())
    }
);

setter!(
    set_stage_entry_popup_burst(id: String, count: Option<u32>) |tx| {
        editor::set_stage_entry_popup_burst(tx, &id, count)
    }
);

setter!(
    set_stage_prompt_timeouts_enabled(id: String, enabled: bool) |tx| {
        editor::set_stage_prompt_timeouts_enabled(tx, &id, enabled)
    }
);

setter!(
    /// Scales every prompt's deadline while this stage is playing.
    set_stage_prompt_timeout_multiplier(id: String, multiplier: f64) |tx| {
        editor::set_stage_prompt_timeout_multiplier(tx, &id, multiplier)
    }
);

setter!(
    set_stage_prompt_popup_burst(id: String, count: Option<u32>) |tx| {
        editor::set_stage_prompt_popup_burst(tx, &id, count)
    }
);

setter!(
    /// Switches window movement on or off, with the speeds it starts at.
    set_stage_movement(id: String, movement: Option<Movement>) |tx| {
        editor::set_stage_movement(tx, &id, movement.as_ref())
    }
);

setter!(
    /// One or both speeds. The unnamed one keeps its value.
    set_stage_movement_speed(id: String, minimum: Option<f64>, maximum: Option<f64>) |tx| {
        editor::set_stage_movement_speed(tx, &id, minimum, maximum)
    }
);

setter!(
    /// Switches mitosis on or off, with the values it starts at.
    set_stage_mitosis(id: String, mitosis: Option<Mitosis>) |tx| {
        editor::set_stage_mitosis(tx, &id, mitosis.as_ref())
    }
);

setter!(
    /// One or both mitosis values. The unnamed one keeps its value.
    set_stage_mitosis_values(id: String, chance: Option<f64>, count: Option<u32>) |tx| {
        editor::set_stage_mitosis_values(tx, &id, chance, count)
    }
);

setter!(
    set_stage_end_duration(id: String, seconds: Option<f64>) |tx| {
        editor::set_stage_end_duration(tx, &id, seconds)
    }
);

setter!(
    set_stage_end_event_count(id: String, condition: Option<EventCountCondition>) |tx| {
        editor::set_stage_end_event_count(tx, &id, condition.as_ref())
    }
);

setter!(
    set_stage_end_strategy(id: String, strategy: EndStrategy) |tx| {
        editor::set_stage_end_strategy(tx, &id, strategy)
    }
);

setter!(
    /// Adds one tag to a restricted stage's selection.
    add_stage_tag(id: String, tag: String) |tx| editor::add_stage_tag(tx, &id, &tag)
);

setter!(
    /// Removes one tag, and the stage's ownership of it if it owned it.
    remove_stage_tag(id: String, tag: String) |tx| editor::remove_stage_tag(tx, &id, &tag)
);
