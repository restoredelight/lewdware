//! Recorded edits for the timeline, stages and transitions, as functions over a connection. See
//! `super` for the contract.
//!
//! Most of these are one call into `shared::behaviour::editor` inside one history entry. The few
//! that are more -- removing a stage, renaming one, switching its tag restriction on, putting one
//! file in or out of it, letting its audio go -- work out what else the action comes to inside the
//! same transaction (a [`BehaviourAction::planned`]). Their builders (`*_action`) are public so the
//! Tauri editor's commands run exactly the same action; see
//! `pack-editor/src-tauri/src/commands/behaviour/stage.rs` for why each is one action.
//!
//! The undo labels are the ones the Tauri editor used, so both editors' history reads the same.

use super::*;

use rusqlite::Transaction;
use serde::Serialize;
use shared::behaviour::editor;
use shared::behaviour::{
    Easing, EndStrategy, EventCountCondition, EventKind, EventSchedule, Interval, MediaSlot,
    Mitosis, Movement, TransitionCategory,
};

use crate::pack::behaviour::{fill_media_slot_tx, slot_label};
use crate::pack::media::delete_unreferenced_scenery;
use crate::pack::{BehaviourAction, BehaviourOutcome, Prepared, TagAction};

/// Runs one behaviour action on `connection`: one transaction, one undo entry.
fn run<P: Send + 'static>(
    connection: &Connection,
    action: BehaviourAction<P>,
) -> Result<BehaviourOutcome> {
    Ok(crate::pack::behaviour_edit_on(connection, action)?.0)
}

/// A field setter: one write, no tags, nothing retired.
fn set(
    connection: &Connection,
    label: &str,
    edit: impl FnOnce(&Transaction<'_>) -> Result<()> + Send + 'static,
) -> Result<()> {
    run(connection, BehaviourAction::new(label, edit)).map(drop)
}

// ── The timeline ────────────────────────────────────────────────────────────────────────────

/// The name a new stage gets: the first "Stage N" (from the stage count up) not already taken.
pub fn next_stage_name(connection: &Connection) -> Result<String> {
    let labels: Vec<String> = editor::stages(connection)?
        .into_iter()
        .map(|stage| stage.label)
        .collect();
    let mut n = labels.len() + 1;
    loop {
        let name = format!("Stage {n}");
        if !labels.contains(&name) {
            return Ok(name);
        }
        n += 1;
    }
}

/// Switches the timeline on or off, keeping its stages either way.
///
/// Switching on a timeline with no stages also gives it its first, in the same entry: an empty
/// timeline is a state the mode can't play. Nothing is retired switching off: dropping every stage
/// is on purpose, and retiring their wallpapers would lose them for good.
pub fn set_timeline_enabled(connection: &Connection, enabled: bool) -> Result<()> {
    let label = if enabled {
        "Enable timeline"
    } else {
        "Disable timeline"
    };
    set(connection, label, move |tx| {
        editor::set_timeline_enabled(tx, enabled)?;
        if enabled && editor::stages(tx)?.is_empty() {
            editor::add_stage(tx, None, None, "Stage 1")?;
        }
        Ok(())
    })
}

/// The mode-name override. Blank (after trimming) clears it, and the mode keeps its own name
/// ("Sequence").
pub fn set_timeline_label(connection: &Connection, label: Option<&str>) -> Result<()> {
    let label = label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string);
    set(connection, "Edit mode name", move |tx| {
        editor::set_timeline_label(tx, label.as_deref())
    })
}

/// Every tag an author can pick for a stage, by name: everything but Lewdware's own.
pub fn author_tags(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare("SELECT name FROM tags ORDER BY name")?;
    let tags = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(tags
        .into_iter()
        .filter(|tag| !shared::tags::is_managed(tag))
        .collect())
}

// ── Stages as a list ────────────────────────────────────────────────────────────────────────

/// Adds a stage after `after` (at the end for `None`), copying `source`'s settings if given, and
/// returns its id. Named with [`next_stage_name`] when `name` is `None`.
pub fn add_stage(
    connection: &Connection,
    after: Option<&str>,
    source: Option<&str>,
    name: Option<&str>,
) -> Result<String> {
    history::record(connection, "Add stage", 0, &[], |tx| {
        let name = match name {
            Some(name) => name.to_string(),
            None => next_stage_name(tx)?,
        };
        editor::add_stage(tx, after, source, &name)
    })
}

/// Copies a stage in beside itself, returning the copy's id.
pub fn duplicate_stage(connection: &Connection, id: &str) -> Result<String> {
    history::record(connection, "Duplicate stage", 0, &[], |tx| {
        editor::duplicate_stage(tx, id)
    })
}

/// Moves a stage to index `to`, clamped to the timeline.
pub fn move_stage(connection: &Connection, id: &str, to: usize) -> Result<()> {
    let id = id.to_string();
    set(connection, "Move stage", move |tx| {
        editor::move_stage(tx, &id, to)
    })
}

/// Removing a stage, the scenery that was only ever its own and the tags it owns, as one action.
/// `also_remove_tag` deletes the owned tags outright; otherwise each goes only if nothing claims it.
pub fn remove_stage_action(
    id: String,
    also_remove_tag: bool,
    label: String,
) -> BehaviourAction<()> {
    let reading = id.clone();
    BehaviourAction::planned(
        label,
        move |tx: &Transaction<'_>| {
            Ok(Prepared {
                value: (),
                retiring: editor::stage_scenery(tx, &reading)?,
                // Both of them: a stage owns an inclusion tag and an exclusion tag, and leaving
                // either behind litters the pack with a tag nothing can reach.
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
    )
}

pub fn remove_stage(connection: &Connection, id: &str, also_remove_tag: bool) -> Result<()> {
    run(
        connection,
        remove_stage_action(id.to_string(), also_remove_tag, "Remove stage".to_string()),
    )
    .map(drop)
}

/// What removing a stage would take with it, for the confirmation to say out loud: the stage's own
/// inclusion tag and everything else that holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StageRemovalImpact {
    pub tag: Option<String>,
    /// Files carrying the tag.
    pub media: u64,
    /// Other stages selecting by it, by label.
    pub stages: Vec<String>,
    /// Content entries and groups naming it.
    pub content: u64,
}

impl StageRemovalImpact {
    /// Whether anything holds the tag, so removing it too is a real choice.
    pub fn claimed(&self) -> bool {
        self.media > 0 || !self.stages.is_empty() || self.content > 0
    }
}

/// A read: see [`StageRemovalImpact`].
pub fn stage_removal_impact(connection: &Connection, id: &str) -> Result<StageRemovalImpact> {
    let tag = editor::stage_owned_tags(connection, id)?
        .into_iter()
        .find(|owned| !owned.excluded)
        .map(|owned| owned.tag);
    let Some(tag) = tag else {
        return Ok(StageRemovalImpact::default());
    };
    let media = connection.query_row(
        "SELECT COUNT(*) FROM media_tags mt JOIN tags t ON t.id = mt.tag_id WHERE t.name = ?1",
        [&tag],
        |row| row.get(0),
    )?;
    let stages = connection
        .prepare(
            "SELECT s.label FROM behaviour_stage_tag st JOIN tags t ON t.id = st.tag_id
             JOIN behaviour_stage s ON s.id = st.stage_id
             WHERE t.name = ?1 AND s.id <> ?2 ORDER BY s.position",
        )?
        .query_map([tag.as_str(), id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let content = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM behaviour_content_group_tag x JOIN tags t ON t.id = x.tag_id
                 WHERE t.name = ?1)
              + (SELECT COUNT(*) FROM behaviour_content_entry_tag x JOIN tags t ON t.id = x.tag_id
                 WHERE t.name = ?1)",
        [&tag],
        |row| row.get(0),
    )?;
    Ok(StageRemovalImpact {
        tag: Some(tag),
        media,
        stages,
        content,
    })
}

// ── One stage's own fields ──────────────────────────────────────────────────────────────────

/// Renaming a stage, and the tags it owns along with it (unless another stage selects by them).
pub fn set_stage_label_action(id: String, value: String, label: String) -> BehaviourAction<()> {
    let (stage, renaming) = (id.clone(), value.clone());
    BehaviourAction::planned(
        label,
        move |tx: &Transaction<'_>| {
            // Both sides: a stage owns `stage-peak` to put files in and `not-stage-peak` to keep
            // them out, and renaming only one leaves the pair disagreeing about their stage.
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
    )
}

/// Renames a stage. A blank name is refused rather than stored.
pub fn set_stage_label(connection: &Connection, id: &str, value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        bail!("A stage needs a name");
    }
    run(
        connection,
        set_stage_label_action(
            id.to_string(),
            value.to_string(),
            "Rename stage".to_string(),
        ),
    )
    .map(drop)
}

/// Starting a stage restricting its content, with a fresh tag of its own seeded onto every file so
/// it shows exactly what it showed a moment before. A stage that already restricts is left alone.
pub fn restrict_stage_content_action(id: String, label: String) -> BehaviourAction<Option<String>> {
    let (naming, writing) = (id.clone(), id);
    BehaviourAction::planned(
        label,
        move |tx: &Transaction<'_>| {
            let Some(label) = editor::read_one_stage_label(tx, &naming)? else {
                bail!("no stage {naming}");
            };
            // Already restricting: nothing to name, and nothing to seed.
            if editor::stage_restricts_content(tx, &naming)? {
                return Ok(Prepared::default());
            }
            let tag = editor::stage_tag_name(tx, &label, None)?;
            Ok(Prepared {
                // `None` is every file in the pack.
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
            None => Ok(()),
        },
    )
}

/// Switches a stage's tag restriction on or off. Off shows the whole pack again; the tag survives
/// on the files carrying it. On touches every file in the pack (~100ms at 10,000 files).
pub fn set_stage_restricts_content(connection: &Connection, id: &str, on: bool) -> Result<()> {
    if on {
        run(
            connection,
            restrict_stage_content_action(id.to_string(), "Restrict stage content".to_string()),
        )
        .map(drop)
    } else {
        let id = id.to_string();
        set(connection, "Change stage content", move |tx| {
            editor::set_stage_content_tags(tx, &id, None, None)
        })
    }
}

/// Adds (`present`) or removes one tag on the stage's inclusion list, or on its exclusion list when
/// `excluded`. Adding to the inclusion list needs the stage to restrict its content already.
pub fn set_stage_tag(
    connection: &Connection,
    id: &str,
    tag: &str,
    excluded: bool,
    present: bool,
) -> Result<()> {
    let tag = tag.trim().to_string();
    if tag.is_empty() {
        bail!("A tag needs a name");
    }
    if present {
        reject_managed_tag(&tag)?;
    }
    let id = id.to_string();
    let label = if present { "Add tag" } else { "Remove tag" };
    set(connection, label, move |tx| match (excluded, present) {
        (false, true) => editor::add_stage_tag(tx, &id, &tag),
        (false, false) => editor::remove_stage_tag(tx, &id, &tag),
        (true, true) => editor::add_stage_exclude_tag(tx, &id, &tag),
        (true, false) => editor::remove_stage_exclude_tag(tx, &id, &tag),
    })
}

/// Letting go of a stage's named track as it switches to (or away from) picking one at random.
pub fn set_stage_audio_random_action(
    id: String,
    random: bool,
    label: String,
) -> BehaviourAction<()> {
    let stage = id.clone();
    BehaviourAction::planned(
        label,
        // The track this stage names *now*.
        move |tx: &Transaction<'_>| {
            Ok(Prepared {
                retiring: editor::stage_audio(tx, &stage)?.into_iter().collect(),
                ..Prepared::default()
            })
        },
        move |tx: &Transaction<'_>, ()| editor::set_stage_audio_random(tx, &id, random),
    )
}

/// What a stage does about background audio when it begins, short of naming a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageAudioMode {
    /// Leaves whatever is playing.
    Keep,
    /// Picks a fresh track from the stage's tags.
    Random,
}

/// Switches the stage between keeping the current audio and picking a random track. Either way a
/// track it named is let go of (and leaves the pack if it was only ever this stage's scenery).
/// Naming a specific track is [`set_stage_slot`] with [`MediaSlot::StageAudio`].
pub fn set_stage_audio_mode(connection: &Connection, id: &str, mode: StageAudioMode) -> Result<()> {
    run(
        connection,
        set_stage_audio_random_action(
            id.to_string(),
            mode == StageAudioMode::Random,
            "Change stage audio".to_string(),
        ),
    )
    .map(drop)
}

/// One event kind's schedule, or `None` to stop the stage scheduling it.
pub fn set_stage_event(
    connection: &Connection,
    id: &str,
    kind: EventKind,
    schedule: Option<EventSchedule>,
) -> Result<()> {
    if let Some(schedule) = &schedule {
        validate_schedule(schedule)?;
    }
    let id = id.to_string();
    set(connection, "Change stage events", move |tx| {
        editor::set_stage_event(tx, &id, kind, schedule.as_ref())
    })
}

fn validate_schedule(schedule: &EventSchedule) -> Result<()> {
    let positive = |seconds: f64| seconds.is_finite() && seconds > 0.0;
    let intervals_ok = match schedule.interval {
        Interval::Fixed { seconds } => positive(seconds),
        Interval::Random {
            minimum_seconds,
            maximum_seconds,
        } => positive(minimum_seconds) && positive(maximum_seconds),
    };
    if !intervals_ok {
        bail!("An interval has to be more than zero seconds");
    }
    if schedule
        .initial_delay_seconds
        .is_some_and(|delay| !delay.is_finite() || delay < 0.0)
    {
        bail!("A delay can't be negative");
    }
    if schedule.max_concurrent == Some(0) {
        bail!("At least one has to be allowed at a time");
    }
    Ok(())
}

/// The desktop notification sent on entry. Blank sends none.
pub fn set_stage_entry_notification(
    connection: &Connection,
    id: &str,
    text: Option<&str>,
) -> Result<()> {
    let (id, text) = (id.to_string(), text.map(str::to_string));
    set(connection, "Edit stage notification", move |tx| {
        editor::set_stage_entry_notification(tx, &id, text.as_deref())
    })
}

/// Popups spawned once on entry (at least one), or `None` for none.
pub fn set_stage_entry_popup_burst(
    connection: &Connection,
    id: &str,
    count: Option<u32>,
) -> Result<()> {
    let label = match count {
        Some(_) => "Edit stage entry popups",
        None => "Toggle stage entry popups",
    };
    let count = count.map(|count| count.max(1));
    let id = id.to_string();
    set(connection, label, move |tx| {
        editor::set_stage_entry_popup_burst(tx, &id, count)
    })
}

pub fn set_stage_prompt_timeouts_enabled(
    connection: &Connection,
    id: &str,
    enabled: bool,
) -> Result<()> {
    let id = id.to_string();
    set(connection, "Change prompt deadlines", move |tx| {
        editor::set_stage_prompt_timeouts_enabled(tx, &id, enabled)
    })
}

/// Scales every prompt's deadline while the stage plays. At least 0.1.
pub fn set_stage_prompt_timeout_multiplier(
    connection: &Connection,
    id: &str,
    multiplier: f64,
) -> Result<()> {
    if !multiplier.is_finite() || multiplier < 0.1 {
        bail!("The time allowance has to be at least 0.1");
    }
    let id = id.to_string();
    set(connection, "Edit prompt time allowance", move |tx| {
        editor::set_stage_prompt_timeout_multiplier(tx, &id, multiplier)
    })
}

/// Popups spawned for a wrong answer or an expired prompt (at least one), or `None` for none.
pub fn set_stage_prompt_popup_burst(
    connection: &Connection,
    id: &str,
    count: Option<u32>,
) -> Result<()> {
    let label = match count {
        Some(_) => "Edit prompt popups",
        None => "Toggle prompt popups",
    };
    let count = count.map(|count| count.max(1));
    let id = id.to_string();
    set(connection, label, move |tx| {
        editor::set_stage_prompt_popup_burst(tx, &id, count)
    })
}

/// The speeds window movement starts at when it's switched on.
pub const DEFAULT_MOVEMENT: Movement = Movement {
    minimum_speed: Some(50.0),
    maximum_speed: Some(150.0),
};

/// The values mitosis starts at when it's switched on.
pub const DEFAULT_MITOSIS: Mitosis = Mitosis {
    chance: Some(0.5),
    count: Some(2),
};

/// Switches window movement on (at [`DEFAULT_MOVEMENT`]) or off.
pub fn set_stage_movement(connection: &Connection, id: &str, on: bool) -> Result<()> {
    let id = id.to_string();
    set(connection, "Toggle window movement", move |tx| {
        editor::set_stage_movement(tx, &id, on.then_some(&DEFAULT_MOVEMENT))
    })
}

/// One or both speeds; the unnamed one keeps its value.
pub fn set_stage_movement_speed(
    connection: &Connection,
    id: &str,
    minimum: Option<f64>,
    maximum: Option<f64>,
) -> Result<()> {
    if [minimum, maximum]
        .into_iter()
        .flatten()
        .any(|speed| !speed.is_finite() || speed < 0.0)
    {
        bail!("A speed can't be negative");
    }
    let id = id.to_string();
    set(connection, "Edit movement speed", move |tx| {
        editor::set_stage_movement_speed(tx, &id, minimum, maximum)
    })
}

/// Switches mitosis on (at [`DEFAULT_MITOSIS`]) or off.
pub fn set_stage_mitosis(connection: &Connection, id: &str, on: bool) -> Result<()> {
    let id = id.to_string();
    set(connection, "Toggle mitosis", move |tx| {
        editor::set_stage_mitosis(tx, &id, on.then_some(&DEFAULT_MITOSIS))
    })
}

/// The chance (0 to 1) and/or the number of copies (at least one); the unnamed one keeps its value.
pub fn set_stage_mitosis_values(
    connection: &Connection,
    id: &str,
    chance: Option<f64>,
    count: Option<u32>,
) -> Result<()> {
    if chance.is_some_and(|chance| !(0.0..=1.0).contains(&chance)) {
        bail!("The chance has to be between 0 and 1");
    }
    let count = count.map(|count| count.max(1));
    let id = id.to_string();
    set(connection, "Edit mitosis", move |tx| {
        editor::set_stage_mitosis_values(tx, &id, chance, count)
    })
}

/// How long the stage lasts, or `None` to end on its event condition alone. Refused for the last
/// stage, which has no end.
pub fn set_stage_end_duration(
    connection: &Connection,
    id: &str,
    seconds: Option<f64>,
) -> Result<()> {
    if seconds.is_some_and(|seconds| !seconds.is_finite() || seconds < 0.0) {
        bail!("A duration can't be negative");
    }
    let id = id.to_string();
    set(connection, "Edit stage duration", move |tx| {
        editor::set_stage_end_duration(tx, &id, seconds)
    })
}

/// The duration a stage's end condition gets by default (`shared::behaviour::editor`'s, which
/// isn't exported).
pub const DEFAULT_STAGE_SECONDS: f64 = 300.0;

/// The event count a stage's end condition starts at when one is chosen.
pub const DEFAULT_END_EVENT_COUNT: u32 = 10;

fn current_end_event(tx: &Transaction<'_>, id: &str) -> Result<Option<EventCountCondition>> {
    Ok(editor::stages(tx)?
        .into_iter()
        .find(|stage| stage.id == id)
        .and_then(|stage| stage.end)
        .and_then(|end| end.event_count))
}

/// Which event's count can also end the stage, or `None` for no event condition. Changing the kind
/// keeps the count and scope already set.
pub fn set_stage_end_event(
    connection: &Connection,
    id: &str,
    event: Option<EventKind>,
) -> Result<()> {
    let id = id.to_string();
    set(connection, "Change stage end condition", move |tx| {
        let current = current_end_event(tx, &id)?;
        let condition = event.map(|event| EventCountCondition {
            event,
            count: current
                .as_ref()
                .map_or(DEFAULT_END_EVENT_COUNT, |current| current.count),
            scope: current.map(|current| current.scope).unwrap_or_default(),
        });
        editor::set_stage_end_event_count(tx, &id, condition.as_ref())
    })
}

/// How many of the chosen event end the stage (at least one). Refused when the stage has no event
/// condition.
pub fn set_stage_end_event_count(connection: &Connection, id: &str, count: u32) -> Result<()> {
    let count = count.max(1);
    let id = id.to_string();
    set(connection, "Edit stage end condition", move |tx| {
        let Some(mut condition) = current_end_event(tx, &id)? else {
            bail!("This stage has no event condition");
        };
        condition.count = count;
        editor::set_stage_end_event_count(tx, &id, Some(&condition))
    })
}

pub fn set_stage_end_strategy(
    connection: &Connection,
    id: &str,
    strategy: EndStrategy,
) -> Result<()> {
    let id = id.to_string();
    set(connection, "Change stage end condition", move |tx| {
        editor::set_stage_end_strategy(tx, &id, strategy)
    })
}

// ── A stage's media slots ───────────────────────────────────────────────────────────────────

fn require_stage_slot(slot: &MediaSlot) -> Result<()> {
    match slot {
        MediaSlot::StageWallpaper { .. }
        | MediaSlot::StageAudio { .. }
        | MediaSlot::StageEntrySplash { .. }
        | MediaSlot::StageEntrySound { .. }
        | MediaSlot::StagePromptSound { .. } => Ok(()),
        MediaSlot::Wallpaper | MediaSlot::Splash => bail!("Not one of a stage's slots"),
    }
}

/// Points one of a stage's slots at a file already in the pack. The file keeps whatever roles it
/// had (it isn't marked as scenery); a file it displaces leaves the pack if it was only ever
/// scenery. The body of `MediaPack::fill_media_slot` with `new_to_pack: false`, which is what the
/// Tauri editor's "choose from the pack" sends.
pub fn set_stage_slot(connection: &Connection, slot: &MediaSlot, media: u64) -> Result<()> {
    require_stage_slot(slot)?;
    history::record_with_media_refs(connection, slot_label(slot), 0, |tx| {
        let deleted = fill_media_slot_tx(tx, slot, media, false)?;
        // The displaced file's bytes have to outlive the entry that dropped it.
        Ok(((), [Some(media), deleted].into_iter().flatten().collect()))
    })
    .map(drop)
}

/// Empties one of a stage's slots, deleting the file it held if that was only ever scenery. The
/// body of `MediaPack::clear_media_slot`.
pub fn clear_stage_slot(connection: &Connection, slot: &MediaSlot) -> Result<()> {
    require_stage_slot(slot)?;
    history::record_with_media_refs(connection, slot_label(slot), 0, |tx| {
        let Some(media) = editor::slot_value(tx, slot)? else {
            return Ok(((), Vec::new()));
        };
        editor::set_media_slot(tx, slot, None)?;
        let deleted = delete_unreferenced_scenery(tx, media)?;
        Ok(((), deleted.into_iter().collect()))
    })
    .map(drop)
}

// ── Which stages a file is in ───────────────────────────────────────────────────────────────

/// What changing one file's membership in a stage came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MembershipOutcome {
    Applied,
    /// Joining would take these tags off the file, and they aren't the stage's own exclusion tag.
    /// Nothing was written: show them, and call again with them approved.
    ConfirmationRequired {
        tags: Vec<String>,
    },
}

/// Putting one file into a stage or taking it out, as one action, planned inside the transaction.
pub fn set_stage_membership_action(
    media: u64,
    stage: String,
    member: bool,
    label: String,
) -> BehaviourAction<editor::MembershipPlan> {
    BehaviourAction::planned(
        label,
        move |tx: &Transaction<'_>| {
            let plan = editor::plan_stage_membership(tx, media, &stage, member)?;
            Ok(Prepared {
                tag_actions: crate::pack::membership_tag_actions(media, &plan),
                value: plan,
                retiring: Vec::new(),
            })
        },
        |tx: &Transaction<'_>, plan: editor::MembershipPlan| {
            editor::apply_membership_creation(tx, &plan)
        },
    )
}

/// Puts `media` into `stage` or takes it out. Joining may need to take author tags off the file
/// (ones the stage excludes by); unless every such tag is in `approved`, nothing is written and the
/// tags come back for the author to confirm.
pub fn set_stage_membership(
    connection: &Connection,
    media: u64,
    stage: &str,
    member: bool,
    approved: &[String],
) -> Result<MembershipOutcome> {
    let required =
        editor::plan_stage_membership(connection, media, stage, member)?.non_owned_removals;
    if required.iter().any(|tag| !approved.contains(tag)) {
        return Ok(MembershipOutcome::ConfirmationRequired { tags: required });
    }
    let file: String = connection
        .query_row(
            "SELECT file_name FROM media WHERE id = ?",
            params![media],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_else(|| "file".to_string());
    let stage_label = editor::read_one_stage_label(connection, stage)?.unwrap_or_default();
    let label = if member {
        format!("Add \u{201c}{file}\u{201d} to {stage_label}")
    } else {
        format!("Remove \u{201c}{file}\u{201d} from {stage_label}")
    };
    run(
        connection,
        set_stage_membership_action(media, stage.to_string(), member, label),
    )?;
    Ok(MembershipOutcome::Applied)
}

// ── Transitions ─────────────────────────────────────────────────────────────────────────────

pub fn set_transition_duration(connection: &Connection, id: &str, seconds: f64) -> Result<()> {
    if !seconds.is_finite() || seconds < 0.0 {
        bail!("A duration can't be negative");
    }
    let id = id.to_string();
    set(connection, "Edit transition duration", move |tx| {
        editor::set_transition_duration(tx, &id, seconds)
    })
}

pub fn set_transition_easing(connection: &Connection, id: &str, easing: Easing) -> Result<()> {
    let id = id.to_string();
    set(connection, "Change transition easing", move |tx| {
        editor::set_transition_easing(tx, &id, easing)
    })
}

/// Turns one gradual change on or off, expanding a legacy broad category first.
pub fn set_transition_category(
    connection: &Connection,
    id: &str,
    category: TransitionCategory,
    enabled: bool,
) -> Result<()> {
    let id = id.to_string();
    set(connection, "Edit gradual changes", move |tx| {
        editor::set_transition_category(tx, &id, category, enabled)
    })
}
