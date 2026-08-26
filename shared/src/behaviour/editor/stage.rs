use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, Transaction, params};

use super::common::{add_tag, remove_tag, set_column};
use crate::behaviour::schema::{
    EndStrategy, EventCountCondition, EventKind, EventSchedule, Mitosis, Movement, StageEnd,
    StageEntry, StagePrompt,
};
use crate::behaviour::storage::{
    read_one_stage, write_owned_stage_tag, write_stage_end, write_stage_entry, write_stage_events,
    write_stage_mitosis, write_stage_movement, write_stage_prompt, write_stage_tags,
};

// ── Stages ───────────────────────────────────────────────────────────────────

const NO_STAGE: &str = "that stage is no longer in the timeline";

pub fn set_stage_label(tx: &Transaction<'_>, id: &str, label: &str) -> Result<bool> {
    set_column(
        tx,
        "behaviour_stage",
        "id",
        &id,
        "label",
        label.to_string(),
        NO_STAGE,
    )
}

/// Which media the stage selects, and which of those tags the editor maintains the name of.
///
/// The two are one command because they are one invariant: ownership is recorded on the association
/// row (`behaviour_stage_tag.owned`), so a stage cannot own a tag it does not select by. Setting
/// them separately would allow a moment where `owned_tag` names something absent from the selection
/// — and it is *checked* here rather than merely written in the right order, because until now that
/// invariant lived only in the front end's habits.
///
/// `tags` of `None` is a stage that restricts nothing, which therefore owns nothing either: there
/// is no selection for a tag to be part of.
pub fn set_stage_content_tags(
    tx: &Transaction<'_>,
    id: &str,
    tags: Option<&[String]>,
    owned_tag: Option<&str>,
) -> Result<bool> {
    if let Some(owned) = owned_tag {
        let selected = tags.is_some_and(|tags| tags.iter().any(|tag| tag == owned));
        if !selected {
            bail!("a stage can only own a tag it selects by");
        }
    }
    let Some(current) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    if current.content.tags.as_deref() == tags && current.content.owned_tag.as_deref() == owned_tag
    {
        return Ok(false);
    }
    tx.execute(
        "UPDATE behaviour_stage SET restricts_content = ?2 WHERE id = ?1",
        params![id, tags.is_some()],
    )?;
    // The exclusion side is carried through untouched: it is a separate decision, and a stage that
    // stops restricting still keeps the files the author took out of it by hand.
    write_stage_tags(tx, id, tags.unwrap_or(&[]), &current.content.exclude)?;
    write_owned_stage_tag(tx, id, false, owned_tag)?;
    Ok(true)
}

/// The tags that keep a file out of this stage, and which of them the editor maintains the name of.
///
/// The mirror of [`set_stage_content_tags`], and the same invariant: ownership lives on the
/// association row, so a stage can only own an exclusion tag it actually excludes by.
///
/// Unlike the inclusion side there is no "no exclusion list" state — an empty list excludes
/// nothing, which is what every stage does until the author says otherwise — so this takes a plain
/// slice and `restricts_content` is not touched. A stage that restricts nothing can still exclude:
/// "everything except the extreme stuff" is the ordinary reason to reach for this.
pub fn set_stage_exclude_tags(
    tx: &Transaction<'_>,
    id: &str,
    exclude: &[String],
    owned_tag: Option<&str>,
) -> Result<bool> {
    if let Some(owned) = owned_tag
        && !exclude.iter().any(|tag| tag == owned)
    {
        bail!("a stage can only own an exclusion tag it excludes by");
    }
    let Some(current) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    if current.content.exclude == exclude
        && current.content.owned_exclude_tag.as_deref() == owned_tag
    {
        return Ok(false);
    }
    // A tag cannot be on both sides at once -- the stage would show nothing carrying it while
    // claiming to select by it -- so excluding one takes it off the inclusion list.
    let include: Vec<String> = current
        .content
        .tags
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|tag| !exclude.contains(tag))
        .collect();
    write_stage_tags(tx, id, &include, exclude)?;
    write_owned_stage_tag(tx, id, false, current.content.owned_tag.as_deref())?;
    write_owned_stage_tag(tx, id, true, owned_tag)?;
    Ok(true)
}

/// Whether the stage picks a fresh background track from its own tags when it begins.
///
/// Naming a track and picking one at random are alternatives, so switching the flag on clears the
/// named track. The other direction is enforced by [`set_media_slot`], which clears this flag when
/// a track is named — between them the two columns cannot both be claimed.
pub fn set_stage_audio_random(tx: &Transaction<'_>, id: &str, random: bool) -> Result<bool> {
    let Some(current) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    let clearing_track = random && current.content.audio.is_some();
    if current.content.audio_random == random && !clearing_track {
        return Ok(false);
    }
    tx.execute(
        "UPDATE behaviour_stage SET audio_random = ?2, audio = CASE WHEN ?2 THEN NULL ELSE audio END
         WHERE id = ?1",
        params![id, random],
    )?;
    Ok(true)
}

/// One event kind's schedule, or `None` to stop the stage scheduling that kind at all.
pub fn set_stage_event(
    tx: &Transaction<'_>,
    id: &str,
    kind: EventKind,
    schedule: Option<&EventSchedule>,
) -> Result<bool> {
    let Some(mut current) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    if current.events.get(kind) == schedule {
        return Ok(false);
    }
    current.events.set(kind, schedule.cloned());
    write_stage_events(tx, id, &current.events)?;
    Ok(true)
}

// The stage's entry and prompt blocks are not optional in the schema -- `StageEntry` and
// `StagePrompt` are plain structs with defaults, not `Option`s -- so a leaf setter can create the
// row it needs rather than requiring a parent to be switched on first.

pub fn set_stage_entry_notification(
    tx: &Transaction<'_>,
    id: &str,
    text: Option<&str>,
) -> Result<bool> {
    edit_stage_entry(tx, id, |entry| {
        entry.notification = text.filter(|text| !text.is_empty()).map(str::to_string)
    })
}

pub fn set_stage_entry_popup_burst(
    tx: &Transaction<'_>,
    id: &str,
    count: Option<u32>,
) -> Result<bool> {
    edit_stage_entry(tx, id, |entry| entry.popup_burst = count)
}

fn edit_stage_entry(
    tx: &Transaction<'_>,
    id: &str,
    change: impl FnOnce(&mut StageEntry),
) -> Result<bool> {
    let Some(stage) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    let mut entry = stage.on_enter.clone();
    change(&mut entry);
    if entry == stage.on_enter {
        return Ok(false);
    }
    write_stage_entry(tx, id, (!entry.is_default()).then_some(&entry))?;
    Ok(true)
}

pub fn set_stage_prompt_timeouts_enabled(
    tx: &Transaction<'_>,
    id: &str,
    enabled: bool,
) -> Result<bool> {
    edit_stage_prompt(tx, id, |prompt| prompt.timeouts_enabled = enabled)
}

/// Scales every prompt's explicit or automatically derived deadline while this stage is playing.
pub fn set_stage_prompt_timeout_multiplier(
    tx: &Transaction<'_>,
    id: &str,
    multiplier: f64,
) -> Result<bool> {
    edit_stage_prompt(tx, id, |prompt| prompt.timeout_multiplier = multiplier)
}

pub fn set_stage_prompt_popup_burst(
    tx: &Transaction<'_>,
    id: &str,
    count: Option<u32>,
) -> Result<bool> {
    edit_stage_prompt(tx, id, |prompt| prompt.popup_burst = count)
}

fn edit_stage_prompt(
    tx: &Transaction<'_>,
    id: &str,
    change: impl FnOnce(&mut StagePrompt),
) -> Result<bool> {
    let Some(stage) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    let mut prompt = stage.prompt.clone();
    change(&mut prompt);
    if prompt == stage.prompt {
        return Ok(false);
    }
    write_stage_prompt(tx, id, &prompt)?;
    Ok(true)
}

/// Switches window movement on or off. One command rather than three, because one click sets both
/// speeds — as separate commands it would cost three undo entries for one action.
pub fn set_stage_movement(
    tx: &Transaction<'_>,
    id: &str,
    movement: Option<&Movement>,
) -> Result<bool> {
    let Some(stage) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    if stage.movement.as_ref() == movement {
        return Ok(false);
    }
    write_stage_movement(tx, id, movement)?;
    Ok(true)
}

pub fn set_stage_movement_speed(
    tx: &Transaction<'_>,
    id: &str,
    minimum: Option<f64>,
    maximum: Option<f64>,
) -> Result<bool> {
    let Some(stage) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    // Only meaningful while movement is switched on; a speed for a stage that does not move is a
    // stale editor, not a request to start moving.
    let Some(current) = stage.movement.as_ref() else {
        bail!("this stage does not move");
    };
    let updated = Movement {
        minimum_speed: minimum.or(current.minimum_speed),
        maximum_speed: maximum.or(current.maximum_speed),
    };
    if &updated == current {
        return Ok(false);
    }
    write_stage_movement(tx, id, Some(&updated))?;
    Ok(true)
}

/// Switches mitosis on or off. One command for the same reason as [`set_stage_movement`].
pub fn set_stage_mitosis(
    tx: &Transaction<'_>,
    id: &str,
    mitosis: Option<&Mitosis>,
) -> Result<bool> {
    let Some(stage) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    if stage.mitosis.as_ref() == mitosis {
        return Ok(false);
    }
    write_stage_mitosis(tx, id, mitosis)?;
    Ok(true)
}

pub fn set_stage_mitosis_values(
    tx: &Transaction<'_>,
    id: &str,
    chance: Option<f64>,
    count: Option<u32>,
) -> Result<bool> {
    let Some(stage) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    let Some(current) = stage.mitosis.as_ref() else {
        bail!("this stage does not use mitosis");
    };
    let updated = Mitosis {
        chance: chance.or(current.chance),
        count: count.or(current.count),
    };
    if &updated == current {
        return Ok(false);
    }
    write_stage_mitosis(tx, id, Some(&updated))?;
    Ok(true)
}

// A stage's end condition exists for every stage but the last, and whether it exists is decided by
// `normalize` rather than by the author -- so a leaf setter refuses when there is none rather than
// creating one, which would give the final stage an end it must not have.

pub fn set_stage_end_duration(
    tx: &Transaction<'_>,
    id: &str,
    seconds: Option<f64>,
) -> Result<bool> {
    edit_stage_end(tx, id, |end| end.duration_seconds = seconds)
}

pub fn set_stage_end_event_count(
    tx: &Transaction<'_>,
    id: &str,
    condition: Option<&EventCountCondition>,
) -> Result<bool> {
    edit_stage_end(tx, id, |end| end.event_count = condition.cloned())
}

pub fn set_stage_end_strategy(
    tx: &Transaction<'_>,
    id: &str,
    strategy: EndStrategy,
) -> Result<bool> {
    edit_stage_end(tx, id, |end| end.strategy = strategy)
}

fn edit_stage_end(
    tx: &Transaction<'_>,
    id: &str,
    change: impl FnOnce(&mut StageEnd),
) -> Result<bool> {
    let Some(stage) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    let Some(current) = stage.end.as_ref() else {
        bail!("the last stage has no end condition");
    };
    let mut updated = current.clone();
    change(&mut updated);
    if &updated == current {
        return Ok(false);
    }
    write_stage_end(tx, id, Some(&updated))?;
    Ok(true)
}

/// Adds a tag to a stage's selection.
///
/// Only for a stage that already restricts its content: on one that shows everything there is no
/// selection for a tag to join, and switching the restriction on is a different author action
/// ([`set_stage_content_tags`]) that has to seed the tag onto what the stage was showing.
pub fn add_stage_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<bool> {
    let restricts: Option<bool> = tx
        .query_row(
            "SELECT restricts_content FROM behaviour_stage WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    match restricts {
        None => bail!("{NO_STAGE}"),
        Some(false) => bail!("this stage shows every file in the pack"),
        Some(true) => add_tag(tx, "behaviour_stage_tag", "stage_id", &id, tag),
    }
}

/// Adds a tag to the list that keeps media *out* of the stage.
///
/// Exclusion wins over inclusion, so a tag the stage includes by is *moved* rather than added: a
/// stage on both sides of one tag would claim to select by something that also guarantees the file
/// is excluded, and would show nothing carrying it.
pub fn add_stage_exclude_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<bool> {
    let Some(stage) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    if stage.content.exclude.iter().any(|item| item == tag) {
        return Ok(false);
    }
    let mut exclude = stage.content.exclude;
    exclude.push(tag.to_string());
    set_stage_exclude_tags(tx, id, &exclude, stage.content.owned_exclude_tag.as_deref())
}

/// Stops the stage excluding by a tag. Every file it was keeping out comes back.
pub fn remove_stage_exclude_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<bool> {
    let Some(stage) = read_one_stage(tx, id)? else {
        bail!("{NO_STAGE}");
    };
    let exclude: Vec<String> = stage
        .content
        .exclude
        .into_iter()
        .filter(|item| item != tag)
        .collect();
    // Ownership lives on the association row, so a tag the stage no longer excludes by is no longer
    // owned -- the same rule the inclusion side has always had.
    let owned = stage
        .content
        .owned_exclude_tag
        .filter(|owned| exclude.contains(owned));
    set_stage_exclude_tags(tx, id, &exclude, owned.as_deref())
}

/// Removes a tag from a stage's selection, and its ownership with it if the stage owned it.
///
/// The ownership needs no separate step: the `owned` flag lives on the association row, so a stage
/// that stops selecting by a tag stops owning it by construction.
pub fn remove_stage_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<bool> {
    remove_tag(tx, "behaviour_stage_tag", "stage_id", &id, tag)
}
