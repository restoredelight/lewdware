use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, Transaction, params};

use super::common::{add_tag, exists, remove_tag, set_column};
use crate::behaviour::db::{
    read_owned_stage_tag, read_stage_end, read_stage_entry, read_stage_mitosis,
    read_stage_movement, read_stage_prompt, read_stage_tags, write_one_stage_event,
    write_owned_stage_tag, write_stage_end, write_stage_entry, write_stage_mitosis,
    write_stage_movement, write_stage_prompt, write_stage_tags,
};
use crate::behaviour::schema::{
    EndStrategy, EventCountCondition, EventKind, EventSchedule, Mitosis, Movement, StageEnd,
    StageEntry, StagePrompt,
};

// ── Stages ───────────────────────────────────────────────────────────────────

const NO_STAGE: &str = "that stage is no longer in the timeline";

pub fn set_stage_label(tx: &Transaction<'_>, id: &str, label: &str) -> Result<()> {
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
) -> Result<()> {
    if let Some(owned) = owned_tag {
        let selected = tags.is_some_and(|tags| tags.iter().any(|tag| tag == owned));
        if !selected {
            bail!("a stage can only own a tag it selects by");
        }
    }
    let updated = tx.execute(
        "UPDATE behaviour_stage SET restricts_content = ?2 WHERE id = ?1",
        params![id, tags.is_some()],
    )?;
    if updated == 0 {
        bail!("{NO_STAGE}");
    }
    // The exclusion side is carried through untouched: it is a separate decision, and a stage that
    // stops restricting still keeps the files the author took out of it by hand.
    let exclude = read_stage_tags(tx, id, true)?;
    write_stage_tags(tx, id, tags.unwrap_or(&[]), &exclude)?;
    write_owned_stage_tag(tx, id, false, owned_tag)?;
    Ok(())
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
) -> Result<()> {
    if let Some(owned) = owned_tag
        && !exclude.iter().any(|tag| tag == owned)
    {
        bail!("a stage can only own an exclusion tag it excludes by");
    }
    let current_restricts: Option<bool> = tx
        .query_row(
            "SELECT restricts_content FROM behaviour_stage WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(current_restricts) = current_restricts else {
        bail!("{NO_STAGE}");
    };
    // A tag cannot be on both sides at once -- the stage would show nothing carrying it while
    // claiming to select by it -- so excluding one takes it off the inclusion list.
    let include: Vec<String> = if current_restricts {
        read_stage_tags(tx, id, false)?
            .into_iter()
            .filter(|tag| !exclude.contains(tag))
            .collect()
    } else {
        Vec::new()
    };
    write_stage_tags(tx, id, &include, exclude)?;
    let owned_include = read_owned_stage_tag(tx, id, false)?;
    write_owned_stage_tag(tx, id, false, owned_include.as_deref())?;
    write_owned_stage_tag(tx, id, true, owned_tag)?;
    Ok(())
}

/// Whether the stage picks a fresh background track from its own tags when it begins.
///
/// Naming a track and picking one at random are alternatives, so switching the flag on clears the
/// named track. The other direction is enforced by [`set_media_slot`], which clears this flag when
/// a track is named — between them the two columns cannot both be claimed.
pub fn set_stage_audio_random(tx: &Transaction<'_>, id: &str, random: bool) -> Result<()> {
    let updated = tx.execute(
        "UPDATE behaviour_stage SET audio_random = ?2, audio = CASE WHEN ?2 THEN NULL ELSE audio END
         WHERE id = ?1",
        params![id, random],
    )?;
    if updated == 0 {
        bail!("{NO_STAGE}");
    }
    Ok(())
}

/// One event kind's schedule, or `None` to stop the stage scheduling that kind at all.
pub fn set_stage_event(
    tx: &Transaction<'_>,
    id: &str,
    kind: EventKind,
    schedule: Option<&EventSchedule>,
) -> Result<()> {
    if !exists(tx, "behaviour_stage", "id", &id)? {
        bail!("{NO_STAGE}");
    }
    write_one_stage_event(tx, id, kind, schedule)?;
    Ok(())
}

// The stage's entry and prompt blocks are not optional in the schema -- `StageEntry` and
// `StagePrompt` are plain structs with defaults, and their fields live on the stage row -- so a
// leaf setter can update them without requiring a parent to be switched on first.

pub fn set_stage_entry_notification(
    tx: &Transaction<'_>,
    id: &str,
    text: Option<&str>,
) -> Result<()> {
    edit_stage_entry(tx, id, |entry| {
        entry.notification = text.filter(|text| !text.is_empty()).map(str::to_string)
    })
}

pub fn set_stage_entry_popup_burst(
    tx: &Transaction<'_>,
    id: &str,
    count: Option<u32>,
) -> Result<()> {
    edit_stage_entry(tx, id, |entry| entry.popup_burst = count)
}

fn edit_stage_entry(
    tx: &Transaction<'_>,
    id: &str,
    change: impl FnOnce(&mut StageEntry),
) -> Result<()> {
    if !exists(tx, "behaviour_stage", "id", &id)? {
        bail!("{NO_STAGE}");
    }
    let mut entry = read_stage_entry(tx, id)?.unwrap_or_default();
    change(&mut entry);
    write_stage_entry(tx, id, (!entry.is_default()).then_some(&entry))?;
    Ok(())
}

pub fn set_stage_prompt_timeouts_enabled(
    tx: &Transaction<'_>,
    id: &str,
    enabled: bool,
) -> Result<()> {
    edit_stage_prompt(tx, id, |prompt| prompt.timeouts_enabled = enabled)
}

/// Scales every prompt's explicit or automatically derived deadline while this stage is playing.
pub fn set_stage_prompt_timeout_multiplier(
    tx: &Transaction<'_>,
    id: &str,
    multiplier: f64,
) -> Result<()> {
    edit_stage_prompt(tx, id, |prompt| prompt.timeout_multiplier = multiplier)
}

pub fn set_stage_prompt_popup_burst(
    tx: &Transaction<'_>,
    id: &str,
    count: Option<u32>,
) -> Result<()> {
    edit_stage_prompt(tx, id, |prompt| prompt.popup_burst = count)
}

fn edit_stage_prompt(
    tx: &Transaction<'_>,
    id: &str,
    change: impl FnOnce(&mut StagePrompt),
) -> Result<()> {
    if !exists(tx, "behaviour_stage", "id", &id)? {
        bail!("{NO_STAGE}");
    }
    let mut prompt = read_stage_prompt(tx, id)?;
    change(&mut prompt);
    write_stage_prompt(tx, id, &prompt)?;
    Ok(())
}

/// Switches window movement on or off. One command rather than three, because one click sets both
/// speeds — as separate commands it would cost three undo entries for one action.
pub fn set_stage_movement(
    tx: &Transaction<'_>,
    id: &str,
    movement: Option<&Movement>,
) -> Result<()> {
    if !exists(tx, "behaviour_stage", "id", &id)? {
        bail!("{NO_STAGE}");
    }
    write_stage_movement(tx, id, movement)?;
    Ok(())
}

pub fn set_stage_movement_speed(
    tx: &Transaction<'_>,
    id: &str,
    minimum: Option<f64>,
    maximum: Option<f64>,
) -> Result<()> {
    if !exists(tx, "behaviour_stage", "id", &id)? {
        bail!("{NO_STAGE}");
    }
    let Some(current) = read_stage_movement(tx, id)? else {
        bail!("this stage does not move");
    };
    let updated = Movement {
        minimum_speed: minimum.or(current.minimum_speed),
        maximum_speed: maximum.or(current.maximum_speed),
    };
    write_stage_movement(tx, id, Some(&updated))?;
    Ok(())
}

/// Switches mitosis on or off. One command for the same reason as [`set_stage_movement`].
pub fn set_stage_mitosis(tx: &Transaction<'_>, id: &str, mitosis: Option<&Mitosis>) -> Result<()> {
    if !exists(tx, "behaviour_stage", "id", &id)? {
        bail!("{NO_STAGE}");
    }
    write_stage_mitosis(tx, id, mitosis)?;
    Ok(())
}

pub fn set_stage_mitosis_values(
    tx: &Transaction<'_>,
    id: &str,
    chance: Option<f64>,
    count: Option<u32>,
) -> Result<()> {
    if !exists(tx, "behaviour_stage", "id", &id)? {
        bail!("{NO_STAGE}");
    }
    let Some(current) = read_stage_mitosis(tx, id)? else {
        bail!("this stage does not use mitosis");
    };
    let updated = Mitosis {
        chance: chance.or(current.chance),
        count: count.or(current.count),
    };
    write_stage_mitosis(tx, id, Some(&updated))?;
    Ok(())
}

// A stage's end condition exists for every stage but the last, and whether it exists is decided by
// `normalize` rather than by the author -- so a leaf setter refuses when there is none rather than
// creating one, which would give the final stage an end it must not have.

pub fn set_stage_end_duration(tx: &Transaction<'_>, id: &str, seconds: Option<f64>) -> Result<()> {
    edit_stage_end(tx, id, |end| end.duration_seconds = seconds)
}

pub fn set_stage_end_event_count(
    tx: &Transaction<'_>,
    id: &str,
    condition: Option<&EventCountCondition>,
) -> Result<()> {
    edit_stage_end(tx, id, |end| end.event_count = condition.cloned())
}

pub fn set_stage_end_strategy(tx: &Transaction<'_>, id: &str, strategy: EndStrategy) -> Result<()> {
    edit_stage_end(tx, id, |end| end.strategy = strategy)
}

fn edit_stage_end(
    tx: &Transaction<'_>,
    id: &str,
    change: impl FnOnce(&mut StageEnd),
) -> Result<()> {
    if !exists(tx, "behaviour_stage", "id", &id)? {
        bail!("{NO_STAGE}");
    }
    let Some(mut updated) = read_stage_end(tx, id)? else {
        bail!("the last stage has no end condition");
    };
    change(&mut updated);
    write_stage_end(tx, id, Some(&updated))?;
    Ok(())
}

/// Adds a tag to a stage's selection.
///
/// Only for a stage that already restricts its content: on one that shows everything there is no
/// selection for a tag to join, and switching the restriction on is a different author action
/// ([`set_stage_content_tags`]) that has to seed the tag onto what the stage was showing.
pub fn add_stage_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<()> {
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
pub fn add_stage_exclude_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<()> {
    if !exists(tx, "behaviour_stage", "id", &id)? {
        bail!("{NO_STAGE}");
    }
    let mut exclude = read_stage_tags(tx, id, true)?;
    // Guarding the push, not the write: the list is a set, and adding a tag twice would put it on
    // the stage twice. Re-writing an unchanged list is fine and costs nothing.
    if !exclude.iter().any(|item| item == tag) {
        exclude.push(tag.to_string());
    }
    let owned = read_owned_stage_tag(tx, id, true)?;
    set_stage_exclude_tags(tx, id, &exclude, owned.as_deref())
}

/// Stops the stage excluding by a tag. Every file it was keeping out comes back.
pub fn remove_stage_exclude_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<()> {
    if !exists(tx, "behaviour_stage", "id", &id)? {
        bail!("{NO_STAGE}");
    }
    let exclude: Vec<String> = read_stage_tags(tx, id, true)?
        .into_iter()
        .filter(|item| item != tag)
        .collect();
    let owned = read_owned_stage_tag(tx, id, true)?;
    let owned = owned.filter(|owned| exclude.contains(owned));
    set_stage_exclude_tags(tx, id, &exclude, owned.as_deref())
}

/// Removes a tag from a stage's selection, and its ownership with it if the stage owned it.
///
/// The ownership needs no separate step: the `owned` flag lives on the association row, so a stage
/// that stops selecting by a tag stops owning it by construction.
pub fn remove_stage_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<()> {
    remove_tag(tx, "behaviour_stage_tag", "stage_id", &id, tag)
}
