//! Defines a way to see and edit whether a given media item is part of a specific stage.
//!
//! To do this, a stage will "own" up to two tags, one in the stage's include list and one
//! in the stage's exclude list, which can be used to easily add or remove an item from a
//! stage.

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;

use super::stage::{set_stage_content_tags, set_stage_exclude_tags};
use super::stage_tags::{stage_exclude_tag_name, stage_tag_name};
use crate::behaviour::db::{read_owned_stage_tag, read_stage_tags, read_stages};
use crate::behaviour::schema::ContentSelection;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MembershipPlan {
    pub creation: Option<StageTagCreation>,
    pub apply: Vec<String>,
    pub remove: Vec<String>,
    /// Removed tags which are not this stage's dedicated exclusion tag and need author approval.
    pub non_owned_removals: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct StageMembership {
    pub id: String,
    pub label: String,
    pub member: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTagCreation {
    pub stage: String,
    pub tag: String,
    pub excluded: bool,
}

pub fn selects_content(content: &ContentSelection, tags: &[String]) -> bool {
    let included = match content.tags.as_deref() {
        None => true,
        Some(restriction) => restriction.iter().any(|tag| tags.contains(tag)),
    };

    included && !content.exclude.iter().any(|tag| tags.contains(tag))
}

fn dedicated_owned_tag(
    conn: &Connection,
    content: &ContentSelection,
    excluded: bool,
) -> Result<Option<String>> {
    let (owned, side) = if excluded {
        (content.owned_exclude_tag.as_deref(), &content.exclude[..])
    } else {
        (
            content.owned_tag.as_deref(),
            content.tags.as_deref().unwrap_or(&[]),
        )
    };

    let Some(owned) = owned else {
        return Ok(None);
    };

    if !side.iter().any(|tag| tag == owned) {
        return Ok(None);
    }

    let (content_count, experience_count): (u64, u64) = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM behaviour_content_group_tag t WHERE t.tag_id = tags.id)
                  + (SELECT COUNT(*) FROM behaviour_text_item_tag t WHERE t.tag_id = tags.id)
                  + (SELECT COUNT(*) FROM behaviour_web_link_tag t WHERE t.tag_id = tags.id)
                    AS content_count,
                (SELECT COUNT(*) FROM behaviour_stage_tag t WHERE t.tag_id = tags.id)
                    AS experience_count
         FROM tags WHERE tags.name = ?1",
        params![owned],
        |row| Ok((row.get("content_count")?, row.get("experience_count")?)),
    )?;

    Ok((content_count == 0 && experience_count == 1).then(|| owned.to_string()))
}

/// The tags a file carries, as the pack holds them right now.
pub fn media_tags(conn: &Connection, media: u64) -> Result<Vec<String>> {
    let mut statement = conn.prepare(
        "SELECT t.name FROM media_tags AS mt JOIN tags AS t ON t.id = mt.tag_id
         WHERE mt.media_id = ?1 ORDER BY t.name",
    )?;
    let tags = statement
        .query_map(params![media], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tags)
}

/// Every stage and whether `media` belongs in it, calculated from the stored timeline selection.
pub fn stage_membership(conn: &Connection, media: u64) -> Result<Vec<StageMembership>> {
    let tags = media_tags(conn, media)?;
    Ok(read_stages(conn)?
        .into_iter()
        .map(|stage| StageMembership {
            member: selects_content(&stage.content, &tags),
            id: stage.id,
            label: stage.label,
        })
        .collect())
}

struct StageSelection {
    id: String,
    label: String,
    content: ContentSelection,
}

fn read_stage_selection(conn: &Connection, id: &str) -> Result<Option<StageSelection>> {
    let row: Option<(String, bool)> = conn
        .query_row(
            "SELECT label, restricts_content FROM behaviour_stage WHERE id = ?1",
            params![id],
            |row| Ok((row.get("label")?, row.get("restricts_content")?)),
        )
        .optional()?;
    let Some((label, restricts)) = row else {
        return Ok(None);
    };
    let tags = if restricts {
        Some(read_stage_tags(conn, id, false)?)
    } else {
        None
    };
    let exclude = read_stage_tags(conn, id, true)?;
    let owned_tag = read_owned_stage_tag(conn, id, false)?;
    let owned_exclude_tag = read_owned_stage_tag(conn, id, true)?;
    Ok(Some(StageSelection {
        id: id.to_string(),
        label,
        content: ContentSelection {
            tags,
            exclude,
            owned_tag,
            owned_exclude_tag,
            wallpaper: None,
            audio: None,
            audio_random: false,
        },
    }))
}

/// How to put `media` into `stage`, or take it out.
pub fn plan_stage_membership(
    conn: &Connection,
    media: u64,
    stage_id: &str,
    member: bool,
) -> Result<MembershipPlan> {
    let Some(target) = read_stage_selection(conn, stage_id)? else {
        bail!("no stage {stage_id}");
    };

    let tags = media_tags(conn, media)?;
    let mut plan = MembershipPlan::default();
    if selects_content(&target.content, &tags) == member {
        return Ok(plan);
    }

    if member {
        let dedicated_exclusion = dedicated_owned_tag(conn, &target.content, true)?;
        for tag in &target.content.exclude {
            if tags.contains(tag) {
                plan.remove.push(tag.clone());
                if dedicated_exclusion.as_deref() != Some(tag) {
                    plan.non_owned_removals.push(tag.clone());
                }
            }
        }

        let without: Vec<String> = tags
            .iter()
            .filter(|tag| !plan.remove.contains(tag))
            .cloned()
            .collect();

        if selects_content(&target.content, &without) {
            return Ok(plan);
        }

        let tag = match dedicated_owned_tag(conn, &target.content, false)? {
            Some(owned) => owned,
            None => {
                let tag = stage_tag_name(conn, &target.label, None)?;
                plan.creation = Some(StageTagCreation {
                    stage: target.id.clone(),
                    tag: tag.clone(),
                    excluded: false,
                });
                tag
            }
        };

        plan.apply.push(tag);

        return Ok(plan);
    }

    // Leaving. Where the stage's own tag is the only thing putting this file here, take it off:
    // exact, and it leaves no contradictory pair on the file.
    if let Some(owned) = dedicated_owned_tag(conn, &target.content, false)?
        && tags.contains(&owned)
    {
        let without: Vec<String> = tags.iter().filter(|tag| **tag != owned).cloned().collect();
        if !selects_content(&target.content, &without) {
            plan.remove.push(owned);
            return Ok(plan);
        }
    }

    // Otherwise the file is here through the author's own tags, or because the stage restricts
    // nothing at all. Either way it leaves by being excluded.
    let tag = match dedicated_owned_tag(conn, &target.content, true)? {
        Some(owned) => owned,
        None => {
            let tag = stage_exclude_tag_name(conn, &target.label, None)?;
            plan.creation = Some(StageTagCreation {
                stage: target.id.clone(),
                tag: tag.clone(),
                excluded: true,
            });
            tag
        }
    };
    plan.apply.push(tag);
    Ok(plan)
}

/// Gives the stage the tag `plan` minted for it, on the side it belongs and owned by the stage.
pub fn apply_membership_creation(tx: &Transaction<'_>, plan: &MembershipPlan) -> Result<()> {
    let Some(creation) = &plan.creation else {
        return Ok(());
    };
    let restricts: Option<bool> = tx
        .query_row(
            "SELECT restricts_content FROM behaviour_stage WHERE id = ?",
            params![&creation.stage],
            |row| row.get(0),
        )
        .optional()?;
    let Some(restricts) = restricts else {
        return Ok(());
    };
    if creation.excluded {
        let mut exclude = read_stage_tags(tx, &creation.stage, true)?;
        exclude.push(creation.tag.clone());
        return set_stage_exclude_tags(tx, &creation.stage, &exclude, Some(&creation.tag));
    }
    let mut tags = if restricts {
        read_stage_tags(tx, &creation.stage, false)?
    } else {
        Vec::new()
    };
    tags.push(creation.tag.clone());
    set_stage_content_tags(tx, &creation.stage, Some(&tags), Some(&creation.tag))
}

/// Whether the stage restricts what it shows, rather than showing the whole pack.
pub fn stage_restricts_content(conn: &Connection, id: &str) -> Result<bool> {
    let restricts: Option<bool> = conn
        .query_row(
            "SELECT restricts_content FROM behaviour_stage WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(restricts.unwrap_or(false))
}
