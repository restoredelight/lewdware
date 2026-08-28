use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::stage::{set_stage_content_tags, set_stage_exclude_tags};
use super::stage_tags::{stage_exclude_tag_name, stage_tag_name};
use crate::behaviour::db::{read_owned_stage_tag, read_stage_tags};
use crate::behaviour::schema::{ContentSelection, Stage};

// ── Which stages a file appears in ───────────────────────────────────────────
//
// The storage model is deliberately tag-based: a stage names tags, and any file carrying one is
// eligible. That scales — new media joins by being tagged, and reordering or deleting a stage
// touches no file. But it makes the question an author asks while *looking* at a file ("where does
// this one show up?") one they would have to run in their head, so the editor answers it and lets
// the answer be edited.
//
// The subtlety: **stages can share a tag.** Leaving one stage still means leaving only that stage,
// so before the target's tags come off, every other membership they were carrying is preserved
// through a tag of that stage's own.

/// The tag work one membership toggle comes to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MembershipPlan {
    /// A tag the stage has to be given before the file can carry it.
    pub creation: Option<StageTagCreation>,
    /// Tags to put on the file.
    pub apply: Vec<String>,
    /// Tags to take off it.
    pub remove: Vec<String>,
}

/// A tag the editor mints for a stage so that one file's membership can be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTagCreation {
    pub stage: String,
    pub tag: String,
    /// Which side of the selection it goes on — an exclusion tag keeps files out.
    pub excluded: bool,
}

/// Whether `tags` puts a file in this stage's content selection.
pub fn selects_content(content: &ContentSelection, tags: &[String]) -> bool {
    let included = match content.tags.as_deref() {
        // No inclusion list at all is a stage that restricts nothing: every file appears in it.
        None => true,
        Some(restriction) => restriction.iter().any(|tag| tags.contains(tag)),
    };
    included && !content.exclude.iter().any(|tag| tags.contains(tag))
}

/// Whether `tags` puts a file in this stage.
///
/// Included *and* not excluded. The order matters for reading it, not for the answer: exclusion
/// always wins, which is what makes it usable as "take this one file out" regardless of why the
/// file was in.
pub fn selects(stage: &Stage, tags: &[String]) -> bool {
    selects_content(&stage.content, tags)
}

/// The tag a membership toggle can add to `stage` without changing any other relationship.
///
/// Only the stage's own tag, and only while this stage is its sole reference anywhere: the moment a
/// caption or a content group also names it, adding it to a file says more than "appears here".
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
                  + (SELECT COUNT(*) FROM behaviour_web_link_tag t WHERE t.tag_id = tags.id),
                (SELECT COUNT(*) FROM behaviour_stage_tag t WHERE t.tag_id = tags.id)
         FROM tags WHERE tags.name = ?1",
        params![owned],
        |row| Ok((row.get(0)?, row.get(1)?)),
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
            |row| Ok((row.get(0)?, row.get(1)?)),
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

/// Works out how to put `media` into `stage`, or take it out, and changes nothing.
///
/// Separate from applying it because the two halves run in different phases of one transaction —
/// the stage rows are behaviour, the tag changes are media — and both have to be planned against
/// the document as it stands *before* either lands.
///
/// **Both directions are additive on a tag the editor owns.** Joining adds the stage's own tag (or
/// takes off its exclusion tag); leaving adds the exclusion tag. Neither touches the author's
/// vocabulary, which is what makes the toggle mean what it says: a stage selecting by `intense` can
/// only lose a file by that file losing `intense`, and that would also drop it from every content
/// group and text pool naming the tag. The one exception is a membership that exists *only* through
/// the stage's own tag, where taking that tag off is exact and says nothing else — and leaves the
/// file without a contradictory-looking `stage-peak` and `not-stage-peak` side by side.
///
/// Because nothing shared is ever removed, no other stage's membership can change as a side effect,
/// and the "rescue" tags an earlier design needed are gone with the problem they solved.
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
        // Excluded by hand: undoing that is the whole of joining, and it must come off whether or
        // not the inclusion list then matches -- an exclusion tag left behind would silently
        // override the tag added below.
        for tag in &target.content.exclude {
            if tags.contains(tag) {
                plan.remove.push(tag.clone());
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

/// Gives the stage the tag [`plan`] minted for it, on the side it belongs and owned by the stage.
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
///
/// The three states of `ContentSelection::tags` collapse to two here: an inclusion list, however
/// empty, is a restriction; its absence is not. See `behaviour_stage.restricts_content`.
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
