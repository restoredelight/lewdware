//! The behaviour document as the pack editor edits it: entities addressed by stable id.
//!
//! [`storage`](super::storage) reads and writes the document *whole*, which is what the engine, the
//! converter and `lw` want — they consume a pack, they do not edit one. The editor wants the
//! opposite: to change one caption without touching the other four hundred, so that the changeset
//! undo records is the size of the edit rather than the size of the pack.
//!
//! **Entities are addressed by the primary key they already have.** Every table here was designed
//! with one — `behaviour_text_item.id`, `behaviour_web_link.id`, `behaviour_content_group.id`,
//! `behaviour_stage.id` — and only [`Behaviour`](super::Behaviour) loses them, because a `Vec` has
//! positions instead. Reading through this module keeps the ids, so an edit names the row it means.
//! That matters beyond tidiness: a position is a guess about what the document looked like when the
//! author clicked, and acting on a stale one edits the wrong entry, silently.
//!
//! **Granularity is one entity, not one field.** An entity here is a handful of small columns, so
//! sending it whole costs a row write and removes the need for a per-field command (and for the
//! "was this field set, or merely not mentioned?" double-option every such command would carry).
//! The rule the per-item attributes already state applies throughout: absent means *no opinion*,
//! never a zero.
//!
//! Every function taking a `&Transaction` returns whether it changed anything, so a caller can skip
//! recording an undo entry for an edit that landed on the value already there.

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Deserializer, Serialize};

use super::schema::{
    AudioMedia, ContentGroup, ContentSelection, EndStrategy, MediaSlot, PopupMedia, Stage, StageEnd,
    TextItem, Timeline, Transition, WebLink,
};
use super::storage::{
    read_one_stage, read_stages, read_tags, read_transitions, write_one_stage, write_tags,
    TimelineView,
};

// Switching the timeline on and off is a storage-level concern -- it is the `enabled` column, and
// `read_experience` has to agree with it -- but it is an editor action, so it is named here too.
pub use super::storage::{set_experience_enabled, set_experience_label};

/// Which of the three text pools an item belongs to.
///
/// The pools share a table and differ only in which one a mode draws from, so this is the `kind`
/// column rather than three near-identical types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolKind {
    Caption,
    Prompt,
    Notification,
}

impl PoolKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Caption => "caption",
            Self::Prompt => "prompt",
            Self::Notification => "notification",
        }
    }
}

/// One text-pool entry, with the row id the editor addresses it by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextItemRow {
    pub id: i64,
    #[serde(flatten)]
    pub item: TextItem,
}

/// One web link, with the row id the editor addresses it by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebLinkRow {
    pub id: i64,
    #[serde(flatten)]
    pub link: WebLink,
}

// ── Text pools ───────────────────────────────────────────────────────────────

/// One pool's entries, in author order.
pub fn text_pool(conn: &Connection, kind: PoolKind) -> Result<Vec<TextItemRow>> {
    let mut statement = conn.prepare(
        "SELECT id, text, timeout_seconds, summary FROM behaviour_text_item
         WHERE kind = ? ORDER BY position",
    )?;
    let rows: Vec<(i64, String, Option<f64>, Option<String>)> = statement
        .query_map(params![kind.as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, text, timeout_seconds, summary)| {
            Ok(TextItemRow {
                id,
                item: TextItem {
                    text,
                    tags: read_tags(conn, "behaviour_text_item_tag", "item_id", &id)?,
                    timeout_seconds,
                    summary,
                },
            })
        })
        .collect()
}

/// Appends an entry to `kind`'s pool, returning its new id.
pub fn add_text_item(tx: &Transaction<'_>, kind: PoolKind, item: &TextItem) -> Result<i64> {
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_text_item WHERE kind = ?",
        params![kind.as_str()],
        |row| row.get(0),
    )?;
    let id: i64 = tx.query_row(
        "INSERT INTO behaviour_text_item (kind, position, text, timeout_seconds, summary)
         VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
        params![
            kind.as_str(),
            position,
            item.text,
            item.timeout_seconds,
            item.summary
        ],
        |row| row.get(0),
    )?;
    write_tags(tx, "behaviour_text_item_tag", "item_id", &id, &item.tags)?;
    Ok(id)
}

/// Replaces the entry `id` holds. Errors if it has since been removed, rather than re-creating it:
/// an edit arriving for an entry that is gone is a stale editor, and reviving the entry would undo
/// the removal.
pub fn update_text_item(tx: &Transaction<'_>, id: i64, item: &TextItem) -> Result<bool> {
    let current: Option<(String, Option<f64>, Option<String>)> = tx
        .query_row(
            "SELECT text, timeout_seconds, summary FROM behaviour_text_item WHERE id = ?",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some(current) = current else {
        bail!("that entry is no longer in the pack");
    };
    let tags = read_tags(tx, "behaviour_text_item_tag", "item_id", &id)?;
    if current.0 == item.text
        && current.1 == item.timeout_seconds
        && current.2 == item.summary
        && tags == item.tags
    {
        return Ok(false);
    }
    tx.execute(
        "UPDATE behaviour_text_item SET text = ?2, timeout_seconds = ?3, summary = ?4
         WHERE id = ?1",
        params![id, item.text, item.timeout_seconds, item.summary],
    )?;
    write_tags(tx, "behaviour_text_item_tag", "item_id", &id, &item.tags)?;
    Ok(true)
}

/// Removes an entry and closes the gap its position left.
pub fn remove_text_item(tx: &Transaction<'_>, id: i64) -> Result<bool> {
    let found: Option<(String, i64)> = tx
        .query_row(
            "SELECT kind, position FROM behaviour_text_item WHERE id = ?",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((kind, position)) = found else {
        return Ok(false);
    };
    tx.execute("DELETE FROM behaviour_text_item WHERE id = ?", params![id])?;
    // Positions are a dense sequence the pool's order depends on, and `(kind, position)` is
    // unique -- so the gap has to close now rather than at the next full write.
    tx.execute(
        "UPDATE behaviour_text_item SET position = position - 1
         WHERE kind = ?1 AND position > ?2",
        params![kind, position],
    )?;
    Ok(true)
}

/// Reorders a pool to exactly `ids`, which must name every entry it holds.
pub fn reorder_text_items(tx: &Transaction<'_>, kind: PoolKind, ids: &[i64]) -> Result<bool> {
    let mut current: Vec<i64> = tx
        .prepare("SELECT id FROM behaviour_text_item WHERE kind = ? ORDER BY position")?
        .query_map(params![kind.as_str()], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if current == ids {
        return Ok(false);
    }
    current.sort_unstable();
    let mut wanted = ids.to_vec();
    wanted.sort_unstable();
    if current != wanted {
        bail!("that reordering does not match the entries the pack has");
    }
    // Two passes through a negative scratch range: `(kind, position)` is unique, so a direct
    // swap would collide with a row that has not moved yet.
    for (position, id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE behaviour_text_item SET position = ?2 WHERE id = ?1",
            params![id, -(position as i64) - 1],
        )?;
    }
    tx.execute(
        "UPDATE behaviour_text_item SET position = -position - 1 WHERE kind = ? AND position < 0",
        params![kind.as_str()],
    )?;
    Ok(true)
}

// ── Web links ────────────────────────────────────────────────────────────────

/// Every web link, in author order.
pub fn web_links(conn: &Connection) -> Result<Vec<WebLinkRow>> {
    let mut statement =
        conn.prepare("SELECT id, url FROM behaviour_web_link ORDER BY position")?;
    let rows: Vec<(i64, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, url)| {
            Ok(WebLinkRow {
                id,
                link: WebLink {
                    url,
                    args: link_args(conn, id)?,
                    tags: read_tags(conn, "behaviour_web_link_tag", "link_id", &id)?,
                },
            })
        })
        .collect()
}

fn link_args(conn: &Connection, id: i64) -> Result<Vec<String>> {
    let mut statement = conn
        .prepare("SELECT value FROM behaviour_web_link_arg WHERE link_id = ? ORDER BY position")?;
    let args = statement
        .query_map(params![id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(args)
}

/// Appends a web link, returning its new id.
pub fn add_web_link(tx: &Transaction<'_>, link: &WebLink) -> Result<i64> {
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_web_link",
        [],
        |row| row.get(0),
    )?;
    let id: i64 = tx.query_row(
        "INSERT INTO behaviour_web_link (position, url) VALUES (?1, ?2) RETURNING id",
        params![position, link.url],
        |row| row.get(0),
    )?;
    write_link_args(tx, id, &link.args)?;
    write_tags(tx, "behaviour_web_link_tag", "link_id", &id, &link.tags)?;
    Ok(id)
}

/// Replaces the link `id` holds. See [`update_text_item`] for why a missing row is an error.
pub fn update_web_link(tx: &Transaction<'_>, id: i64, link: &WebLink) -> Result<bool> {
    let url: Option<String> = tx
        .query_row(
            "SELECT url FROM behaviour_web_link WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(url) = url else {
        bail!("that web link is no longer in the pack");
    };
    let args = link_args(tx, id)?;
    let tags = read_tags(tx, "behaviour_web_link_tag", "link_id", &id)?;
    if url == link.url && args == link.args && tags == link.tags {
        return Ok(false);
    }
    tx.execute(
        "UPDATE behaviour_web_link SET url = ?2 WHERE id = ?1",
        params![id, link.url],
    )?;
    write_link_args(tx, id, &link.args)?;
    write_tags(tx, "behaviour_web_link_tag", "link_id", &id, &link.tags)?;
    Ok(true)
}

fn write_link_args(tx: &Transaction<'_>, id: i64, args: &[String]) -> Result<()> {
    // A set of suffixes is ordered but not unique -- the same suffix twice is how a pack weights
    // it -- so `position` is part of the key and the tail is truncated rather than diffed.
    tx.execute(
        "DELETE FROM behaviour_web_link_arg WHERE link_id = ?1 AND position >= ?2",
        params![id, args.len() as i64],
    )?;
    for (position, value) in args.iter().enumerate() {
        tx.execute(
            "INSERT INTO behaviour_web_link_arg (link_id, position, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(link_id, position) DO UPDATE SET value = excluded.value",
            params![id, position as i64, value],
        )?;
    }
    Ok(())
}

/// Removes a web link and closes the gap its position left.
pub fn remove_web_link(tx: &Transaction<'_>, id: i64) -> Result<bool> {
    let position: Option<i64> = tx
        .query_row(
            "SELECT position FROM behaviour_web_link WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(position) = position else {
        return Ok(false);
    };
    tx.execute("DELETE FROM behaviour_web_link WHERE id = ?", params![id])?;
    tx.execute(
        "UPDATE behaviour_web_link SET position = position - 1 WHERE position > ?",
        params![position],
    )?;
    Ok(true)
}

// ── Content groups ───────────────────────────────────────────────────────────

/// Every content group, in author order.
pub fn content_groups(conn: &Connection) -> Result<Vec<ContentGroup>> {
    super::storage::read_content_groups(conn)
}

/// Adds a content group. Its `id` is the author-facing key the resolver turns into a mode option
/// (`content_group.<id>`), so a collision is the author's to resolve, not ours to paper over.
pub fn add_content_group(tx: &Transaction<'_>, group: &ContentGroup) -> Result<()> {
    let taken: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM behaviour_content_group WHERE id = ?)",
        params![group.id],
        |row| row.get(0),
    )?;
    if taken {
        bail!("a content group named “{}” already exists", group.id);
    }
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_content_group",
        [],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT INTO behaviour_content_group (id, position, label, description, enabled_by_default)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            group.id,
            position,
            group.label,
            group.description,
            group.enabled_by_default
        ],
    )?;
    write_tags(
        tx,
        "behaviour_content_group_tag",
        "group_id",
        &group.id,
        &group.tags,
    )?;
    Ok(())
}

/// Replaces the group `id` holds, which may rename it — `group.id` is allowed to differ from `id`.
pub fn update_content_group(tx: &Transaction<'_>, id: &str, group: &ContentGroup) -> Result<bool> {
    let current: Option<(String, Option<String>, bool)> = tx
        .query_row(
            "SELECT label, description, enabled_by_default FROM behaviour_content_group
             WHERE id = ?",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some(current) = current else {
        bail!("that content group is no longer in the pack");
    };
    let tags = read_tags(tx, "behaviour_content_group_tag", "group_id", &id)?;
    if id == group.id
        && current.0 == group.label
        && current.1 == group.description
        && current.2 == group.enabled_by_default
        && tags == group.tags
    {
        return Ok(false);
    }
    if id != group.id {
        let taken: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM behaviour_content_group WHERE id = ?)",
            params![group.id],
            |row| row.get(0),
        )?;
        if taken {
            bail!("a content group named “{}” already exists", group.id);
        }
    }
    tx.execute(
        "UPDATE behaviour_content_group
         SET id = ?2, label = ?3, description = ?4, enabled_by_default = ?5 WHERE id = ?1",
        params![
            id,
            group.id,
            group.label,
            group.description,
            group.enabled_by_default
        ],
    )?;
    write_tags(
        tx,
        "behaviour_content_group_tag",
        "group_id",
        &group.id,
        &group.tags,
    )?;
    Ok(true)
}

/// Removes a content group and closes the gap its position left.
pub fn remove_content_group(tx: &Transaction<'_>, id: &str) -> Result<bool> {
    let position: Option<i64> = tx
        .query_row(
            "SELECT position FROM behaviour_content_group WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(position) = position else {
        return Ok(false);
    };
    tx.execute(
        "DELETE FROM behaviour_content_group WHERE id = ?",
        params![id],
    )?;
    tx.execute(
        "UPDATE behaviour_content_group SET position = position - 1 WHERE position > ?",
        params![position],
    )?;
    Ok(true)
}

// ── The timeline ─────────────────────────────────────────────────────────────
//
// Two invariants hold over the stage list, and every structural edit below restores them rather
// than asking its caller to:
//
// - **The last stage has no end condition.** There is nothing after it to move on to, so an end
//   there would be a promise the mode cannot keep.
// - **There is exactly one transition per adjacent pair of stages**, and a transition whose two
//   endpoints are still adjacent keeps its identity — so reordering stages around it does not
//   silently reset a duration the author set.
//
// This used to live in the editor's front end (`timelineModel.ts`) because that is where the edit
// was constructed. It belongs here: the front end never needs it to *render* a timeline, only to
// build one, and building one is now the backend's job.

/// The default an end condition takes when a stage stops being last and needs one.
const DEFAULT_STAGE_SECONDS: f64 = 300.0;

/// Restores the two timeline invariants, in place.
///
/// `new_id` mints ids for transitions that have to be created, so tests can make them predictable.
fn normalize(timeline: &mut Timeline, mut new_id: impl FnMut() -> String) {
    let last = timeline.stages.len().saturating_sub(1);
    for (index, stage) in timeline.stages.iter_mut().enumerate() {
        if index == last {
            stage.end = None;
        } else if stage.end.is_none() {
            stage.end = Some(StageEnd {
                duration_seconds: Some(DEFAULT_STAGE_SECONDS),
                event_count: None,
                strategy: EndStrategy::Any,
            });
        }
    }

    let existing = std::mem::take(&mut timeline.transitions);
    timeline.transitions = timeline
        .stages
        .windows(2)
        .map(|pair| {
            let (from, to) = (&pair[0], &pair[1]);
            existing
                .iter()
                .find(|candidate| candidate.from_stage == from.id && candidate.to_stage == to.id)
                .cloned()
                .unwrap_or_else(|| Transition {
                    id: format!("transition-{}", new_id()),
                    from_stage: from.id.clone(),
                    to_stage: to.id.clone(),
                    duration_seconds: 0.0,
                    easing: Default::default(),
                    affected: Vec::new(),
                })
        })
        .collect();
}

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The timeline as the editor sees it, switched off or not. See [`TimelineView`].
pub fn timeline(conn: &Connection) -> Result<Option<TimelineView>> {
    super::storage::read_timeline(conn)
}

/// Writes `timeline` back over the stored one, touching only the rows that differ.
///
/// The structural edits below all share this tail: they read the timeline, rearrange the stage
/// list, normalize, and write. Only the rows that actually changed are touched, so reordering two
/// stages records a changeset naming two stages rather than the whole timeline.
fn write_timeline(tx: &Transaction<'_>, timeline: &Timeline) -> Result<()> {
    super::storage::retain(
        tx,
        "behaviour_stage",
        "id",
        timeline.stages.iter().map(|stage| stage.id.as_str()),
    )?;
    for (position, stage) in timeline.stages.iter().enumerate() {
        write_one_stage(tx, position as i64, stage)?;
    }
    super::storage::write_transitions(tx, &timeline.transitions)?;
    Ok(())
}

/// Reads the timeline for a structural edit, refusing if the pack has none.
fn timeline_for_edit(tx: &Transaction<'_>) -> Result<Timeline> {
    let Some(view) = super::storage::read_timeline(tx)? else {
        bail!("this pack has no timeline");
    };
    Ok(view.timeline)
}

/// The stages, in order, whether or not the timeline is switched on.
pub fn stages(conn: &Connection) -> Result<Vec<Stage>> {
    read_stages(conn)
}

/// The transitions, in order.
pub fn transitions(conn: &Connection) -> Result<Vec<Transition>> {
    read_transitions(conn)
}

/// Inserts a stage after `after` (or at the end if `after` is `None`), returning its id.
///
/// `source` is the stage the new one copies its settings from — the author's expectation when
/// adding a stage next to one they have configured. The copy never inherits tag *ownership*: two
/// stages claiming one tag would mean renaming either rewrote a name the other reads.
pub fn add_stage(
    tx: &Transaction<'_>,
    after: Option<&str>,
    source: Option<&str>,
    label: &str,
) -> Result<String> {
    let mut timeline = timeline_for_edit(tx)?;
    let template = source
        .and_then(|id| timeline.stages.iter().find(|stage| stage.id == id))
        .cloned();
    let mut stage = template.unwrap_or_else(|| Stage {
        id: String::new(),
        label: String::new(),
        end: None,
        content: ContentSelection::default(),
        events: Default::default(),
        movement: None,
        mitosis: None,
        on_enter: Default::default(),
        prompt: Default::default(),
    });
    stage.id = format!("stage-{}", uuid());
    stage.label = label.to_string();
    stage.content.owned_tag = None;

    let id = stage.id.clone();
    let at = match after {
        Some(after) => timeline
            .stages
            .iter()
            .position(|stage| stage.id == after)
            .map(|index| index + 1)
            .unwrap_or(timeline.stages.len()),
        None => timeline.stages.len(),
    };
    timeline.stages.insert(at, stage);
    normalize(&mut timeline, uuid);
    write_timeline(tx, &timeline)?;
    Ok(id)
}

/// Copies `id` in beside itself, returning the copy's id.
pub fn duplicate_stage(tx: &Transaction<'_>, id: &str) -> Result<String> {
    let label = {
        let timeline = timeline_for_edit(tx)?;
        let Some(source) = timeline.stages.iter().find(|stage| stage.id == id) else {
            bail!("that stage is no longer in the timeline");
        };
        format!("{} copy", source.label)
    };
    add_stage(tx, Some(id), Some(id), &label)
}

/// Moves `id` to index `to`, clamped to the timeline's bounds.
pub fn move_stage(tx: &Transaction<'_>, id: &str, to: usize) -> Result<bool> {
    let mut timeline = timeline_for_edit(tx)?;
    let Some(from) = timeline.stages.iter().position(|stage| stage.id == id) else {
        bail!("that stage is no longer in the timeline");
    };
    let to = to.min(timeline.stages.len().saturating_sub(1));
    if from == to {
        return Ok(false);
    }
    let stage = timeline.stages.remove(from);
    timeline.stages.insert(to, stage);
    normalize(&mut timeline, uuid);
    write_timeline(tx, &timeline)?;
    Ok(true)
}

/// Removes `id` from the timeline.
///
/// Refuses to remove the last remaining stage: a timeline with no stages is not a state the editor
/// can show or the mode can play. Retiring the stage's media and its owned tag is the caller's
/// business — both need the pack's media tables, which this module deliberately does not touch.
pub fn remove_stage(tx: &Transaction<'_>, id: &str) -> Result<bool> {
    let mut timeline = timeline_for_edit(tx)?;
    if timeline.stages.len() <= 1 {
        bail!("a timeline needs at least one stage");
    }
    let Some(index) = timeline.stages.iter().position(|stage| stage.id == id) else {
        return Ok(false);
    };
    timeline.stages.remove(index);
    normalize(&mut timeline, uuid);
    write_timeline(tx, &timeline)?;
    Ok(true)
}

/// Replaces the settings of one stage, leaving its position and the timeline around it alone.
///
/// The stage's `id` is not editable here — it is what addresses the row, and the transitions point
/// at it. Everything else is written whole; see the module docs for why that is the granularity.
pub fn update_stage(tx: &Transaction<'_>, id: &str, stage: &Stage) -> Result<bool> {
    let position: Option<i64> = tx
        .query_row(
            "SELECT position FROM behaviour_stage WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(position) = position else {
        bail!("that stage is no longer in the timeline");
    };
    let current = read_one_stage(tx, id)?;
    let stage = Stage {
        id: id.to_string(),
        ..stage.clone()
    };
    if current.as_ref() == Some(&stage) {
        return Ok(false);
    }
    write_one_stage(tx, position, &stage)?;
    Ok(true)
}

/// Replaces one transition's settings. Its endpoints are not editable: which stages a transition
/// joins is decided by their order, and rewriting them here would contradict the timeline.
pub fn update_transition(tx: &Transaction<'_>, id: &str, transition: &Transition) -> Result<bool> {
    let current: Option<(f64, String)> = tx
        .query_row(
            "SELECT duration_seconds, easing FROM behaviour_transition WHERE id = ?",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((duration_seconds, easing)) = current else {
        bail!("that transition is no longer in the timeline");
    };
    let easing: super::schema::Easing = super::storage::from_text(&easing)?;
    let affected = transition_categories(tx, id)?;
    if duration_seconds == transition.duration_seconds
        && easing == transition.easing
        && affected == transition.affected
    {
        return Ok(false);
    }
    tx.execute(
        "UPDATE behaviour_transition SET duration_seconds = ?2, easing = ?3 WHERE id = ?1",
        params![
            id,
            transition.duration_seconds,
            super::storage::to_text(&transition.easing)?
        ],
    )?;
    tx.execute(
        "DELETE FROM behaviour_transition_category WHERE transition_id = ?",
        params![id],
    )?;
    for (position, category) in transition.affected.iter().enumerate() {
        tx.execute(
            "INSERT INTO behaviour_transition_category (transition_id, category, position)
             VALUES (?1, ?2, ?3)",
            params![
                id,
                super::storage::to_text(category)?,
                position as i64
            ],
        )?;
    }
    Ok(true)
}

fn transition_categories(
    conn: &Connection,
    id: &str,
) -> Result<Vec<super::schema::TransitionCategory>> {
    let mut statement = conn.prepare(
        "SELECT category FROM behaviour_transition_category WHERE transition_id = ?
         ORDER BY position",
    )?;
    let rows: Vec<String> = statement
        .query_map(params![id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    rows.iter()
        .map(|value| super::storage::from_text(value))
        .collect()
}

// ── Per-file attributes ──────────────────────────────────────────────────────
//
// What a pack author says about *this file*, rather than about a tag, a stage, or the pack. The
// rule that shapes the whole section: **absent means "no opinion", never a zero.** A field the
// author never set has to stay distinguishable from one they set to today's default, because
// defaults move under the user across engine releases (`behaviour-design/default-mode.md`,
// Ownership). So an unset field is NULL, and an entry with nothing left to say is not stored at
// all.
//
// This is the one place a partial update is unavoidable, because the inspector edits a *selection*
// — "set scale to 2 on these forty files", leaving each file's other attributes alone. Hence the
// double option below: absent leaves a field as it is, `null` clears it.

/// A field in a partial change: absent leaves it alone, `null` clears it.
///
/// Serde folds `null` into `None` for a plain `Option`, which would make "clear this" and "don't
/// mention this" the same message — exactly the distinction this section exists to keep.
fn double_option<'de, T, D>(deserializer: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

/// A partial edit to one or more files' popup attributes. See [`double_option`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PopupChanges {
    #[serde(deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub weight: Option<Option<f64>>,
    #[serde(deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub scale: Option<Option<f64>>,
    #[serde(deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub region: Option<Option<super::schema::SpawnRegion>>,
    #[serde(deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub monitor: Option<Option<super::schema::MonitorPreference>>,
    #[serde(deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub caption: Option<Option<String>>,
    #[serde(deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub video_loop: Option<Option<bool>>,
    #[serde(deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub video_audio: Option<Option<bool>>,
    /// A set, so an empty list *is* the cleared state — there is no separate null to send.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<Vec<u64>>,
}

impl PopupChanges {
    fn apply(&self, entry: &mut PopupMedia) {
        if let Some(value) = self.weight {
            entry.weight = value;
        }
        if let Some(value) = self.scale {
            entry.scale = value;
        }
        if let Some(value) = self.region {
            entry.region = value;
        }
        if let Some(value) = self.monitor {
            entry.monitor = value;
        }
        if let Some(value) = self.caption.clone() {
            // An empty caption says nothing, and would otherwise keep an entry alive holding "".
            entry.caption = value.filter(|text| !text.is_empty());
        }
        if let Some(value) = self.video_loop {
            entry.video_loop = value;
        }
        if let Some(value) = self.video_audio {
            entry.video_audio = value;
        }
        if let Some(value) = self.audio.clone() {
            entry.audio = value;
        }
    }
}

/// A partial edit to one or more files' audio attributes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioChanges {
    #[serde(deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub volume: Option<Option<f64>>,
}

/// The popup attributes of each of `ids` that has any, keyed by media id.
///
/// Files the author has said nothing about are absent rather than present-and-empty, which is what
/// lets the caller tell "no opinion" from "an opinion that happens to match the default".
pub fn popup_attributes(conn: &Connection, ids: &[u64]) -> Result<Vec<(u64, PopupMedia)>> {
    let mut found = Vec::new();
    for id in ids {
        if let Some(entry) = super::storage::read_one_popup_media(conn, *id)? {
            found.push((*id, entry));
        }
    }
    Ok(found)
}

/// The audio attributes of each of `ids` that has any. See [`popup_attributes`].
pub fn audio_attributes(conn: &Connection, ids: &[u64]) -> Result<Vec<(u64, AudioMedia)>> {
    let mut found = Vec::new();
    for id in ids {
        let volume: Option<Option<f64>> = conn
            .query_row(
                "SELECT volume FROM behaviour_audio_media WHERE media_id = ?",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(volume) = volume {
            found.push((*id, AudioMedia { volume }));
        }
    }
    Ok(found)
}

/// Applies `changes` to every file in `ids`, dropping any entry left saying nothing.
pub fn set_popup_attributes(
    tx: &Transaction<'_>,
    ids: &[u64],
    changes: &PopupChanges,
) -> Result<bool> {
    let mut changed = false;
    for id in ids {
        let before = super::storage::read_one_popup_media(tx, *id)?;
        let mut after = before.clone().unwrap_or_default();
        changes.apply(&mut after);
        if before.as_ref() == Some(&after) || (before.is_none() && after.is_empty()) {
            continue;
        }
        changed = true;
        if after.is_empty() {
            // An entry with nothing left to say is not stored -- the same answer a whole-document
            // write reaches, arrived at now so the UI never shows a state the next read denies.
            tx.execute(
                "DELETE FROM behaviour_popup_media WHERE media_id = ?",
                params![id],
            )?;
            continue;
        }
        super::storage::write_one_popup_media(tx, *id, &after)?;
    }
    Ok(changed)
}

/// Applies `changes` to every file in `ids`. See [`set_popup_attributes`].
pub fn set_audio_attributes(
    tx: &Transaction<'_>,
    ids: &[u64],
    changes: &AudioChanges,
) -> Result<bool> {
    let mut changed = false;
    for id in ids {
        let before: Option<Option<f64>> = tx
            .query_row(
                "SELECT volume FROM behaviour_audio_media WHERE media_id = ?",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        let mut after = AudioMedia {
            volume: before.flatten(),
        };
        if let Some(value) = changes.volume {
            after.volume = value;
        }
        if before.map(|volume| AudioMedia { volume }).as_ref() == Some(&after)
            || (before.is_none() && after.is_empty())
        {
            continue;
        }
        changed = true;
        if after.is_empty() {
            tx.execute(
                "DELETE FROM behaviour_audio_media WHERE media_id = ?",
                params![id],
            )?;
            continue;
        }
        tx.execute(
            "INSERT INTO behaviour_audio_media (media_id, volume) VALUES (?1, ?2)
             ON CONFLICT(media_id) DO UPDATE SET volume = excluded.volume",
            params![id, after.volume],
        )?;
    }
    Ok(changed)
}

// ── Media slots ──────────────────────────────────────────────────────────────

/// The pack-wide media slots. Stage slots live on their stage, and are read with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaSlots {
    pub wallpaper: Option<u64>,
    pub splash: Option<u64>,
}

/// What the pack's wallpaper and splash slots point at.
pub fn media_slots(conn: &Connection) -> Result<MediaSlots> {
    let (wallpaper, splash) = conn
        .query_row(
            "SELECT wallpaper, splash FROM behaviour_content WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .unwrap_or((None, None));
    Ok(MediaSlots { wallpaper, splash })
}

// ── One slot at a time ───────────────────────────────────────────────────────
//
// A slot is one nullable column. Reading or writing it by loading the whole document, changing one
// field and writing it back is what the rest of this module exists to stop doing — and here it was
// also unsafe: a document read while the timeline is switched off comes back with no stages at all
// (that being the whole point of `enabled`), so writing it back would delete the stages the author
// switched off but expects to find again.

/// Where a slot lives: the table, the value column, the column keying it to a stage, and which
/// stage. The stage's own row is keyed by `id`; every table hanging off it uses `stage_id`.
struct SlotColumn<'a> {
    table: &'static str,
    column: &'static str,
    key: &'static str,
    stage: Option<&'a str>,
}

fn slot_column(slot: &MediaSlot) -> SlotColumn<'_> {
    let (table, column, stage) = match slot {
        MediaSlot::Wallpaper => ("behaviour_content", "wallpaper", None),
        MediaSlot::Splash => ("behaviour_content", "splash", None),
        MediaSlot::StageWallpaper { stage } => ("behaviour_stage", "wallpaper", Some(stage)),
        MediaSlot::StageAudio { stage } => ("behaviour_stage", "audio", Some(stage)),
        MediaSlot::StageEntrySplash { stage } => {
            ("behaviour_stage_entry", "splash_media", Some(stage))
        }
        MediaSlot::StageEntrySound { stage } => {
            ("behaviour_stage_entry", "sound_media", Some(stage))
        }
        MediaSlot::StagePromptSound { stage } => {
            ("behaviour_stage_prompt", "sound_media", Some(stage))
        }
    };
    SlotColumn {
        table,
        column,
        key: if table == "behaviour_stage" {
            "id"
        } else {
            "stage_id"
        },
        stage: stage.map(String::as_str),
    }
}

/// What `slot` currently points at, or `None` if it is empty or its stage is gone.
pub fn slot_value(conn: &Connection, slot: &MediaSlot) -> Result<Option<u64>> {
    let SlotColumn {
        table,
        column,
        key,
        stage,
    } = slot_column(slot);
    let value: Option<Option<u64>> = match stage {
        None => conn
            .query_row(
                &format!("SELECT {column} FROM {table} WHERE singleton = 1"),
                [],
                |row| row.get(0),
            )
            .optional()?,
        Some(stage) => conn
            .query_row(
                &format!("SELECT {column} FROM {table} WHERE {key} = ?"),
                params![stage],
                |row| row.get(0),
            )
            .optional()?,
    };
    Ok(value.flatten())
}

/// Points `slot` at `media`, or empties it with `None`. Reports whether anything changed.
///
/// Errors if the slot's stage has been removed — a slot on a stage that is gone is a stale editor,
/// and creating the row would resurrect part of what the removal took away.
pub fn set_media_slot(
    tx: &Transaction<'_>,
    slot: &MediaSlot,
    media: Option<u64>,
) -> Result<bool> {
    if slot_value(tx, slot)? == media {
        return Ok(false);
    }
    let SlotColumn {
        table,
        column,
        stage,
        ..
    } = slot_column(slot);
    let Some(stage) = stage else {
        tx.execute(
            &format!("UPDATE {table} SET {column} = ? WHERE singleton = 1"),
            params![media],
        )?;
        return Ok(true);
    };
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM behaviour_stage WHERE id = ?)",
        params![stage],
        |row| row.get(0),
    )?;
    if !exists {
        bail!("that stage is no longer in the timeline");
    }
    match table {
        // The stage's own row always exists, so this is a plain update.
        "behaviour_stage" => {
            tx.execute(
                &format!("UPDATE behaviour_stage SET {column} = ? WHERE id = ?"),
                params![media, stage],
            )?;
            // Naming a track and "pick one at random" are alternatives, not both.
            if matches!(slot, MediaSlot::StageAudio { .. }) && media.is_some() {
                tx.execute(
                    "UPDATE behaviour_stage SET audio_random = 0 WHERE id = ?",
                    params![stage],
                )?;
            }
        }
        // The optional sub-structures are 0..1 rows, so filling one may have to create it.
        "behaviour_stage_entry" => {
            tx.execute(
                &format!(
                    "INSERT INTO behaviour_stage_entry (stage_id, {column}) VALUES (?1, ?2)
                     ON CONFLICT(stage_id) DO UPDATE SET {column} = excluded.{column}"
                ),
                params![stage, media],
            )?;
        }
        _ => {
            tx.execute(
                &format!(
                    "INSERT INTO behaviour_stage_prompt (stage_id, {column}) VALUES (?1, ?2)
                     ON CONFLICT(stage_id) DO UPDATE SET {column} = excluded.{column}"
                ),
                params![stage, media],
            )?;
        }
    }
    Ok(true)
}

/// Whether any media slot in the pack points at `media`.
///
/// Asked of the tables rather than of a document, and that is not an optimisation. A document read
/// while the timeline is switched off has no stages in it, so a stage wallpaper would come back
/// unreferenced — and the caller for this question is scenery cleanup, which *deletes* what nothing
/// references. Switching the timeline off must not quietly bin every stage's wallpaper.
pub fn is_media_referenced(conn: &Connection, media: u64) -> Result<bool> {
    let referenced: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM behaviour_content WHERE wallpaper = ?1 OR splash = ?1
             UNION ALL
             SELECT 1 FROM behaviour_stage WHERE wallpaper = ?1 OR audio = ?1
             UNION ALL
             SELECT 1 FROM behaviour_stage_entry
                 WHERE splash_media = ?1 OR sound_media = ?1
             UNION ALL
             SELECT 1 FROM behaviour_stage_prompt WHERE sound_media = ?1
         )",
        params![media],
        |row| row.get(0),
    )?;
    Ok(referenced)
}

#[cfg(test)]
mod tests;
