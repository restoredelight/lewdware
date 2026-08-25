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
//! - `remove_stage` and the rename retire media and tags alongside the stage row, so a stage, the
//!   wallpaper that was only ever its scenery, and the tag that existed only for it go in one
//!   transaction.
//!
//! What none of them take is a *decision*. The stage's scenery, the name its tag should have, and
//! whether that tag is the editor's to rename are all facts about the pack, and they are worked out
//! here, inside the transaction, from the pack itself — not computed by the front end against
//! whatever it last fetched and sent along. See [`BehaviourAction::retiring`].

use rusqlite::Transaction;
use shared::behaviour::editor;
use shared::behaviour::{EndStrategy, EventCountCondition, EventKind, EventSchedule, Mitosis, Movement};
use tauri::State;

use super::setter;
use crate::pack::{BehaviourAction, BehaviourOutcome, Prepared, TagAction};
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
/// One undo entry rather than three: the stage, the files that were only ever its scenery, and the
/// tag that existed only for it go together, so one undo brings back the stage *with* its wallpaper
/// rather than a stage the author never created.
///
/// `also_remove_tag` is the only thing the caller decides, because it is the only part that is not
/// a fact — the author was shown what the tag is on and said to take it anyway. Left false, the tag
/// goes if and only if nothing turns out to claim it.
#[tauri::command]
pub async fn remove_stage(
    state: State<'_, AppState>,
    id: String,
    also_remove_tag: bool,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            let reading = id.clone();
            pack.behaviour_edit(BehaviourAction::planned(
                label,
                move |tx: &Transaction<'_>| {
                    Ok(Prepared {
                        value: (),
                        retiring: editor::stage_scenery(tx, &reading)?,
                        // Both of them: a stage owns an inclusion tag and an exclusion tag, and
                        // leaving either behind litters the pack with a tag nothing can reach.
                        tag_actions: editor::stage_owned_tags(tx, &reading)?
                            .into_iter()
                            .map(|owned| {
                                if also_remove_tag {
                                    TagAction::Delete { tag: owned.tag }
                                } else {
                                    TagAction::RetireIfUnclaimed { tag: owned.tag }
                                }
                            })
                            .collect(),
                    })
                },
                move |tx: &Transaction<'_>, ()| editor::remove_stage(tx, &id),
            ))
            .await
        })
        .await
}


// ── The stage's own fields ───────────────────────────────────────────────────

/// Renames the stage, and renames the tag it owns along with it.
///
/// One command because they are one author action: the editor names a stage's tag after the stage,
/// so letting the two land separately would leave a rename half-applied under undo.
///
/// The rename is worked out here because all three of its conditions are facts about the pack. Only
/// a tag the stage owns, and only one no other stage selects by — a rename is lossless everywhere
/// else it appears, since the lists hold tag *ids* and follow the row, but another stage reading
/// this name is another author decision and this one is bookkeeping. And the new name has to dedupe
/// against the tags the pack has at the moment of the write, not the ones the front end last saw.
#[tauri::command]
pub async fn set_stage_label(
    state: State<'_, AppState>,
    id: String,
    value: String,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            let (stage, renaming) = (id.clone(), value.clone());
            pack.behaviour_edit(BehaviourAction::planned(
                label,
                move |tx: &Transaction<'_>| {
                    // Both sides: a stage owns `stage-peak` to put files in and `not-stage-peak` to
                    // keep them out, and renaming only one leaves the pair disagreeing about which
                    // stage they belong to.
                    let mut renames = Vec::new();
                    for owned in editor::stage_owned_tags(tx, &stage)? {
                        if editor::stage_tag_shared(tx, &owned.tag, &stage)? {
                            continue;
                        }
                        let to = if owned.excluded {
                            editor::stage_exclude_tag_name(tx, &renaming, Some(&owned.tag))?
                        } else {
                            editor::stage_tag_name(tx, &renaming, Some(&owned.tag))?
                        };
                        if to != owned.tag {
                            renames.push(TagAction::Rename {
                                from: owned.tag,
                                to,
                            });
                        }
                    }
                    Ok(Prepared {
                        tag_actions: renames,
                        ..Prepared::default()
                    })
                },
                move |tx: &Transaction<'_>, ()| editor::set_stage_label(tx, &id, &value),
            ))
            .await
        })
        .await
}

/// Stops the stage restricting its content: it shows the whole pack again.
///
/// Its tag goes with the restriction — there is no selection left for a tag to be part of, and the
/// `owned` flag lives on the association row. The tag itself survives on the media carrying it,
/// which is the point: switching the restriction back on is a fresh decision, not an undo.
#[tauri::command]
pub async fn unrestrict_stage_content(
    state: State<'_, AppState>,
    id: String,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::new(label, move |tx: &Transaction<'_>| {
                editor::set_stage_content_tags(tx, &id, None, None)
            }))
            .await
        })
        .await
}

/// Starts the stage restricting its content, giving it a tag of its own to restrict by.
///
/// Switching this on is the cliff: the stage goes from *everything* to *nothing* in one checkbox,
/// because an empty inclusion list selects no media. So the new tag is seeded onto the media the
/// stage is showing right now — which, for a stage that restricted nothing, is the whole pack.
/// Behaviour is preserved exactly, and only then do the per-file toggles have a tag to remove. See
/// `behaviour-design/default-mode-v2.md`, "Turning a stage's tags on is the cliff".
///
/// The name comes from the stage's label, deduped against the tags the pack has *at this moment* —
/// which is why it is chosen here. A name picked from a stale tag list would either collide with a
/// real classification, quietly adopting whatever it is already on, or fail the uniqueness check
/// and take the author's checkbox with it.
///
/// **A stage that is already restricted is left alone.** The checkbox computes its next value from
/// what it last rendered, so a double click sends this twice; without the check the second call
/// would mint `stage-peak-2`, put it on every file, replace the selection with it — orphaning the
/// first tag on the whole pack — and spend a second undo entry. The same guard covers a genuinely
/// stale request, which would otherwise overwrite tags the author has since chosen by hand.
#[tauri::command]
pub async fn restrict_stage_content(
    state: State<'_, AppState>,
    id: String,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            let (naming, writing) = (id.clone(), id);
            pack.behaviour_edit(BehaviourAction::planned(
                label,
                move |tx: &Transaction<'_>| {
                    let Some(label) = editor::read_one_stage_label(tx, &naming)? else {
                        anyhow::bail!("no stage {naming}");
                    };
                    // Already restricting: nothing to name, and nothing to seed.
                    if editor::stage_restricts_content(tx, &naming)? {
                        return Ok(Prepared::default());
                    }
                    let tag = editor::stage_tag_name(tx, &label, None)?;
                    Ok(Prepared {
                        // `media: None` is "every file in the pack", resolved against the pack
                        // rather than by shipping every id across the IPC boundary to describe one
                        // checkbox.
                        tag_actions: vec![TagAction::Apply {
                            tag: tag.clone(),
                            media: None,
                        }],
                        value: Some(tag),
                        retiring: Vec::new(),
                    })
                },
                move |tx: &Transaction<'_>, tag: Option<String>| match tag {
                    Some(tag) => editor::set_stage_content_tags(
                        tx,
                        &writing,
                        Some(std::slice::from_ref(&tag)),
                        Some(&tag),
                    ),
                    None => Ok(false),
                },
            ))
            .await
        })
        .await
}

setter!(
    /// Adds a tag to the list that keeps media *out* of this stage.
    ///
    /// Exclusion wins over inclusion, so this is how a stage says "everything except…" — and the
    /// only way to take one file out of a stage without editing the author's own vocabulary.
    add_stage_exclude_tag(id: String, tag: String) |tx| {
        editor::add_stage_exclude_tag(tx, &id, &tag)
    }
);

setter!(
    /// Stops the stage excluding by a tag. Every file it was keeping out comes back.
    remove_stage_exclude_tag(id: String, tag: String) |tx| {
        editor::remove_stage_exclude_tag(tx, &id, &tag)
    }
);

/// Puts one file into a stage, or takes it out — the "Appears in" strip.
///
/// The whole toggle is one command because it is one thing the author clicked, and because every
/// part of what it comes to is a fact about the pack rather than about the screen that asked.
/// Either direction may need the stage to be given a tag of its own first — an inclusion tag to
/// join by, an exclusion tag to be kept out by — and minting it, selecting by it and putting it on
/// the file are one transaction, so they cannot come apart under undo.
///
/// Nothing about it is destructive. Both directions are additive on a tag the editor owns, so the
/// author's own vocabulary is never edited to move one file — see [`editor::plan_stage_membership`].
#[tauri::command]
pub async fn set_stage_membership(
    state: State<'_, AppState>,
    media: u64,
    stage: String,
    member: bool,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_edit(BehaviourAction::planned(
                label,
                move |tx: &Transaction<'_>| {
                    let plan = editor::plan_stage_membership(tx, media, &stage, member)?;
                    Ok(Prepared {
                        tag_actions: membership_tag_actions(media, &plan),
                        value: plan,
                        retiring: Vec::new(),
                    })
                },
                |tx: &Transaction<'_>, plan: editor::MembershipPlan| {
                    editor::apply_membership_creation(tx, &plan)
                },
            ))
            .await
        })
        .await
}

/// Whether the stage picks a fresh background track from its own tags when it begins.
///
/// Switching this on clears any named track — the two are alternatives. The track being let go of
/// is retired with it, so a sound that was only ever this stage's scenery leaves with it rather
/// than staying behind marked out of popups and referenced by nothing.
#[tauri::command]
pub async fn set_stage_audio_random(
    state: State<'_, AppState>,
    id: String,
    random: bool,
    label: String,
) -> Result<BehaviourOutcome, String> {
    state
        .with_pack(async |pack| {
            let stage = id.clone();
            pack.behaviour_edit(BehaviourAction::planned(
                label,
                // The track this stage names *now*, not the one the front end last saw it naming.
                move |tx: &Transaction<'_>| {
                    Ok(Prepared {
                        retiring: editor::stage_audio(tx, &stage)?.into_iter().collect(),
                        ..Prepared::default()
                    })
                },
                move |tx: &Transaction<'_>, ()| editor::set_stage_audio_random(tx, &id, random),
            ))
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

/// The tag work one membership toggle comes to: the rescue tags on, the stage's tags off.
///
/// Named so the tests can build the same action the command does — what they exercise is the phase
/// ordering, and a hand-written pair of tag actions would exercise nothing.
pub(crate) fn membership_tag_actions(
    media: u64,
    plan: &editor::MembershipPlan,
) -> Vec<TagAction> {
    plan.apply
        .iter()
        .map(|tag| TagAction::Apply {
            tag: tag.clone(),
            media: Some(vec![media]),
        })
        .chain(plan.remove.iter().map(|tag| TagAction::Remove {
            tag: tag.clone(),
            media: vec![media],
        }))
        .collect()
}
