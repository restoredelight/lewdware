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

use std::collections::HashSet;

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use super::schema::{
    AudioMedia, ContentGroup, ContentSelection, Easing, EndStrategy, EventCountCondition, EventKind,
    EventSchedule, MediaSlot, Mitosis, Movement, PopupMedia, Stage, StageEnd, TextItem, Timeline,
    MonitorPreference, SpawnRegion, Transition, TransitionCategory, WebLink,
};
use super::storage::{
    read_one_stage, read_stages, read_tags, read_transitions, write_one_stage, write_owned_stage_tag,
    write_stage_tags,
    write_stage_end, write_stage_entry, write_stage_events, write_stage_mitosis,
    write_stage_movement, write_stage_prompt, write_tags, to_text, TimelineView,
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
///
/// Its suffixes carry their own ids rather than arriving as a bare list, because nothing else
/// identifies one: the list is ordered and may repeat, so a value names no particular entry, and an
/// index into the rendered array shifts the moment any other suffix goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebLinkRow {
    pub id: i64,
    pub url: String,
    pub tags: Vec<String>,
    pub args: Vec<WebLinkArg>,
}

/// One suffix appended at random when a link is opened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebLinkArg {
    /// Names this suffix for as long as it exists, and is never given to another.
    pub id: i64,
    pub value: String,
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


// ── Web links ────────────────────────────────────────────────────────────────

/// Every web link, in author order.
pub fn web_links(conn: &Connection) -> Result<Vec<WebLinkRow>> {
    let mut statement = conn.prepare("SELECT id, url FROM behaviour_web_link ORDER BY position")?;
    let rows: Vec<(i64, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, url)| {
            Ok(WebLinkRow {
                id,
                url,
                tags: read_tags(conn, "behaviour_web_link_tag", "link_id", &id)?,
                args: numbered_link_args(conn, id)?,
            })
        })
        .collect()
}

fn numbered_link_args(conn: &Connection, id: i64) -> Result<Vec<WebLinkArg>> {
    let mut statement = conn.prepare(
        "SELECT id, value FROM behaviour_web_link_arg WHERE link_id = ? ORDER BY position",
    )?;
    let args = statement
        .query_map(params![id], |row| {
            Ok(WebLinkArg {
                id: row.get(0)?,
                value: row.get(1)?,
            })
        })?
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

/// Applies `change` to every file in `ids`, dropping any entry left saying nothing.
///
/// The prune is what keeps "the author has said nothing about this file" and "the author set every
/// field to whatever today's default happens to be" from becoming the same stored state — defaults
/// move under the user across engine releases, and an entry that says nothing must not pin a file
/// against the ones that were current when it was written.
fn edit_popup_entries(
    tx: &Transaction<'_>,
    ids: &[u64],
    change: impl Fn(&mut PopupMedia),
) -> Result<bool> {
    let mut changed = false;
    for id in ids {
        let before = super::storage::read_one_popup_media(tx, *id)?;
        let mut after = before.clone().unwrap_or_default();
        change(&mut after);
        if before.as_ref() == Some(&after) || (before.is_none() && after.is_empty()) {
            continue;
        }
        changed = true;
        if after.is_empty() {
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

/// How often these files are drawn relative to their neighbours.
pub fn set_popup_weight(tx: &Transaction<'_>, ids: &[u64], weight: Option<f64>) -> Result<bool> {
    edit_popup_entries(tx, ids, |entry| entry.weight = weight)
}

/// Multiplies the size the mode would otherwise have chosen.
pub fn set_popup_scale(tx: &Transaction<'_>, ids: &[u64], scale: Option<f64>) -> Result<bool> {
    edit_popup_entries(tx, ids, |entry| entry.scale = scale)
}

/// The part of the monitor these files may spawn in. One field, not four: a region is a rectangle,
/// and half of one is not a placement anybody asked for.
pub fn set_popup_region(
    tx: &Transaction<'_>,
    ids: &[u64],
    region: Option<SpawnRegion>,
) -> Result<bool> {
    edit_popup_entries(tx, ids, |entry| entry.region = region)
}

pub fn set_popup_monitor(
    tx: &Transaction<'_>,
    ids: &[u64],
    monitor: Option<MonitorPreference>,
) -> Result<bool> {
    edit_popup_entries(tx, ids, |entry| entry.monitor = monitor)
}

/// A caption belonging to these files, as opposed to the tag-matched pool. An empty one is the
/// absence of a caption rather than a caption of "".
pub fn set_popup_caption(
    tx: &Transaction<'_>,
    ids: &[u64],
    caption: Option<&str>,
) -> Result<bool> {
    let caption = caption.filter(|text| !text.is_empty());
    edit_popup_entries(tx, ids, |entry| {
        entry.caption = caption.map(str::to_string)
    })
}

pub fn set_popup_video_loop(
    tx: &Transaction<'_>,
    ids: &[u64],
    value: Option<bool>,
) -> Result<bool> {
    edit_popup_entries(tx, ids, |entry| entry.video_loop = value)
}

pub fn set_popup_video_audio(
    tx: &Transaction<'_>,
    ids: &[u64],
    value: Option<bool>,
) -> Result<bool> {
    edit_popup_entries(tx, ids, |entry| entry.video_audio = value)
}

/// This track's own level, for levelling a pack assembled from mixed sources.
pub fn set_audio_volume(tx: &Transaction<'_>, ids: &[u64], volume: Option<f64>) -> Result<bool> {
    let mut changed = false;
    for id in ids {
        let before: Option<Option<f64>> = tx
            .query_row(
                "SELECT volume FROM behaviour_audio_media WHERE media_id = ?",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        if before.flatten() == volume {
            continue;
        }
        changed = true;
        match volume {
            None => {
                tx.execute(
                    "DELETE FROM behaviour_audio_media WHERE media_id = ?",
                    params![id],
                )?;
            }
            Some(volume) => {
                tx.execute(
                    "INSERT INTO behaviour_audio_media (media_id, volume) VALUES (?1, ?2)
                     ON CONFLICT(media_id) DO UPDATE SET volume = excluded.volume",
                    params![id, volume],
                )?;
            }
        }
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

// ── Setting one field ────────────────────────────────────────────────────────
//
// Everything below sets a single field, and that is the point rather than an optimisation. A
// command that writes a whole entity cannot be applied twice concurrently without one write
// carrying a stale copy of whatever the other changed — which is why the editor used to keep a
// draft, merge changes into it, and count generations to know when it could let go. Two writes that
// each touch one column commute, so none of that is needed for correctness.
//
// It also removes the "was this field set, or merely not mentioned?" problem. A partial update has
// to tell an absent field from a null one, which is what `PopupChanges` needed `double_option` for;
// a setter that names its field has nothing to not-mention, so a plain `Option<T>` says it all.
//
// **Every setter reports whether it changed anything.** An `oninput` that fires without changing a
// value is not an edit, and recording one would spend an entry out of the hundred undo keeps.

/// Writes one column of an existing row, reporting whether the value actually moved.
///
/// The row must exist: an edit arriving for something that has since been removed is a stale editor
/// acting on a document that has moved on, and re-creating the row would undo the removal.
///
/// `value` is owned rather than borrowed because the same type has to come back out of the database
/// to be compared against — and `&str` cannot, only `String` can.
fn set_column<T>(
    tx: &Transaction<'_>,
    table: &str,
    key_column: &str,
    key: &dyn rusqlite::ToSql,
    column: &str,
    value: T,
    missing: &str,
) -> Result<bool>
where
    T: rusqlite::ToSql + rusqlite::types::FromSql + PartialEq,
{
    let current: Option<T> = tx
        .query_row(
            &format!("SELECT {column} FROM {table} WHERE {key_column} = ?"),
            params![key],
            |row| row.get(0),
        )
        .optional()?;
    let Some(current) = current else {
        bail!("{missing}");
    };
    if current == value {
        return Ok(false);
    }
    tx.execute(
        &format!("UPDATE {table} SET {column} = ?2 WHERE {key_column} = ?1"),
        params![key, value],
    )?;
    Ok(true)
}

// ── Text pool entries ────────────────────────────────────────────────────────

const NO_TEXT_ITEM: &str = "that entry is no longer in the pack";

pub fn set_text_item_text(tx: &Transaction<'_>, id: i64, text: &str) -> Result<bool> {
    set_column(
        tx,
        "behaviour_text_item",
        "id",
        &id,
        "text",
        text.to_string(),
        NO_TEXT_ITEM,
    )
}

/// The notification's title. `None` is a notification shown with no title, which is what every
/// converted Edgeware pack gets — a blank one is the absence of a title, not a title of "".
pub fn set_text_item_summary(tx: &Transaction<'_>, id: i64, summary: Option<&str>) -> Result<bool> {
    set_column(
        tx,
        "behaviour_text_item",
        "id",
        &id,
        "summary",
        summary.map(str::to_string),
        NO_TEXT_ITEM,
    )
}

/// A prompt's answer deadline. `None` asks the mode to derive one from the prompt's length.
pub fn set_text_item_timeout(tx: &Transaction<'_>, id: i64, seconds: Option<f64>) -> Result<bool> {
    set_column(
        tx,
        "behaviour_text_item",
        "id",
        &id,
        "timeout_seconds",
        seconds,
        NO_TEXT_ITEM,
    )
}


// ── Web links ────────────────────────────────────────────────────────────────

const NO_WEB_LINK: &str = "that web link is no longer in the pack";

pub fn set_web_link_url(tx: &Transaction<'_>, id: i64, url: &str) -> Result<bool> {
    set_column(
        tx,
        "behaviour_web_link",
        "id",
        &id,
        "url",
        url.to_string(),
        NO_WEB_LINK,
    )
}



// ── Content groups ───────────────────────────────────────────────────────────

const NO_GROUP: &str = "that content group is no longer in the pack";

pub fn set_content_group_label(tx: &Transaction<'_>, id: &str, label: &str) -> Result<bool> {
    set_column(
        tx,
        "behaviour_content_group",
        "id",
        &id,
        "label",
        label.to_string(),
        NO_GROUP,
    )
}

pub fn set_content_group_description(
    tx: &Transaction<'_>,
    id: &str,
    description: Option<&str>,
) -> Result<bool> {
    set_column(
        tx,
        "behaviour_content_group",
        "id",
        &id,
        "description",
        description.map(str::to_string),
        NO_GROUP,
    )
}

pub fn set_content_group_enabled_by_default(
    tx: &Transaction<'_>,
    id: &str,
    enabled: bool,
) -> Result<bool> {
    set_column(
        tx,
        "behaviour_content_group",
        "id",
        &id,
        "enabled_by_default",
        enabled,
        NO_GROUP,
    )
}


/// Refuses if the row is gone. See [`set_column`] for why that is an error rather than a no-op.
fn exists(
    tx: &Transaction<'_>,
    table: &str,
    key_column: &str,
    key: &dyn rusqlite::ToSql,
    missing: &str,
) -> Result<()> {
    let found: bool = tx.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {key_column} = ?)"),
        params![key],
        |row| row.get(0),
    )?;
    if !found {
        bail!("{missing}");
    }
    Ok(())
}

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
    if current.content.tags.as_deref() == tags && current.content.owned_tag.as_deref() == owned_tag {
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
    change: impl FnOnce(&mut super::schema::StageEntry),
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
    change: impl FnOnce(&mut super::schema::StagePrompt),
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
pub fn set_stage_mitosis(tx: &Transaction<'_>, id: &str, mitosis: Option<&Mitosis>) -> Result<bool> {
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

// ── Transitions ──────────────────────────────────────────────────────────────

const NO_TRANSITION: &str = "that transition is no longer in the timeline";

pub fn set_transition_duration(tx: &Transaction<'_>, id: &str, seconds: f64) -> Result<bool> {
    set_column(
        tx,
        "behaviour_transition",
        "id",
        &id,
        "duration_seconds",
        seconds,
        NO_TRANSITION,
    )
}

pub fn set_transition_easing(tx: &Transaction<'_>, id: &str, easing: Easing) -> Result<bool> {
    set_column(
        tx,
        "behaviour_transition",
        "id",
        &id,
        "easing",
        to_text(&easing)?,
        NO_TRANSITION,
    )
}

/// Replaces the whole set of values that interpolate gradually.
///
/// Not reached from the editor, which toggles one category at a time — this is what
/// [`set_transition_category`] writes the result with once it has expanded any legacy group.
pub fn set_transition_affected(
    tx: &Transaction<'_>,
    id: &str,
    affected: &[TransitionCategory],
) -> Result<bool> {
    if transition_categories(tx, id)? == affected {
        return Ok(false);
    }
    exists(tx, "behaviour_transition", "id", &id, NO_TRANSITION)?;
    tx.execute(
        "DELETE FROM behaviour_transition_category WHERE transition_id = ?",
        params![id],
    )?;
    for (position, category) in affected.iter().enumerate() {
        tx.execute(
            "INSERT INTO behaviour_transition_category (transition_id, category, position)
             VALUES (?1, ?2, ?3)",
            params![id, to_text(category)?, position as i64],
        )?;
    }
    Ok(true)
}

// ── One item of a collection ─────────────────────────────────────────────────
//
// A tag chip and a URL suffix are *items*, and adding or removing one is the whole of what the
// author did. Setting the resulting list instead has the same flaw whole-entity writes had, one
// level down: the list is built from what was last fetched, so two removals in quick succession
// both say "everything except the one I clicked" and the second puts the first one back.
//
// Addressing the item makes the two edits commute, which is the same reason the field setters
// above exist.

/// Adds `tag` to an owner's selection, at the end. Reports whether it was not already there.
fn add_tag(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
    tag: &str,
) -> Result<bool> {
    let id = super::storage::tag_id(tx, tag)?;
    let present: bool = tx.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {key} = ?1 AND tag_id = ?2)"),
        params![owner, id],
        |row| row.get(0),
    )?;
    if present {
        return Ok(false);
    }
    let position: i64 = tx.query_row(
        &format!("SELECT COALESCE(MAX(position) + 1, 0) FROM {table} WHERE {key} = ?"),
        params![owner],
        |row| row.get(0),
    )?;
    tx.execute(
        &format!("INSERT INTO {table} ({key}, tag_id, position) VALUES (?1, ?2, ?3)"),
        params![owner, id, position],
    )?;
    Ok(true)
}

/// Removes `tag` from an owner's selection. Reports whether it was there.
///
/// The gap it leaves in `position` is not closed: positions only order the list, and the primary
/// key is the owner and the tag, so nothing collides with a hole.
fn remove_tag(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
    tag: &str,
) -> Result<bool> {
    let removed = tx.execute(
        &format!(
            "DELETE FROM {table} WHERE {key} = ?1
             AND tag_id = (SELECT id FROM tags WHERE name = ?2)"
        ),
        params![owner, tag],
    )?;
    Ok(removed > 0)
}

pub fn add_text_item_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<bool> {
    exists(tx, "behaviour_text_item", "id", &id, NO_TEXT_ITEM)?;
    add_tag(tx, "behaviour_text_item_tag", "item_id", &id, tag)
}

pub fn remove_text_item_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<bool> {
    remove_tag(tx, "behaviour_text_item_tag", "item_id", &id, tag)
}

pub fn add_web_link_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<bool> {
    exists(tx, "behaviour_web_link", "id", &id, NO_WEB_LINK)?;
    add_tag(tx, "behaviour_web_link_tag", "link_id", &id, tag)
}

pub fn remove_web_link_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<bool> {
    remove_tag(tx, "behaviour_web_link_tag", "link_id", &id, tag)
}

pub fn add_content_group_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<bool> {
    exists(tx, "behaviour_content_group", "id", &id, NO_GROUP)?;
    add_tag(tx, "behaviour_content_group_tag", "group_id", &id, tag)
}

pub fn remove_content_group_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<bool> {
    remove_tag(tx, "behaviour_content_group_tag", "group_id", &id, tag)
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

/// Appends one suffix to a web link's list.
pub fn add_web_link_arg(tx: &Transaction<'_>, id: i64, value: &str) -> Result<bool> {
    exists(tx, "behaviour_web_link", "id", &id, NO_WEB_LINK)?;
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_web_link_arg WHERE link_id = ?",
        params![id],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT INTO behaviour_web_link_arg (link_id, position, value) VALUES (?1, ?2, ?3)",
        params![id, position, value],
    )?;
    Ok(true)
}

/// Removes one suffix, by its own id.
///
/// Not by value, because the same suffix twice is a legitimate way to weight it — the list is
/// ordered and not a set. Not by position either: a position says where a suffix sits, and where it
/// sits changes. The id is the only thing about a suffix that does not move.
///
/// The gap the removal leaves in `position` is not closed. Nothing needs the numbers to be dense —
/// they only order the list — and closing it would move every suffix after this one, which is what
/// the id exists to stop mattering.
pub fn remove_web_link_arg(tx: &Transaction<'_>, link: i64, arg: i64) -> Result<bool> {
    let removed = tx.execute(
        "DELETE FROM behaviour_web_link_arg WHERE link_id = ?1 AND id = ?2",
        params![link, arg],
    )?;
    Ok(removed > 0)
}

/// Turns one transition category on or off, expanding a legacy broad category first.
///
/// Early v3 editor builds stored one entry for a whole group ("events"), where the editor now names
/// each value. Ticking any member of such a group has to replace the broad entry with its siblings,
/// or switching one off would silently switch the rest off too. That expansion lives here rather
/// than in the editor so the whole toggle is one command — two of them built from the last fetched
/// list would each drop the other's work.
pub fn set_transition_category(
    tx: &Transaction<'_>,
    id: &str,
    category: TransitionCategory,
    enabled: bool,
) -> Result<bool> {
    let mut affected = transition_categories(tx, id)?;
    if let Some(group) = legacy_group_of(category)
        && let Some(at) = affected.iter().position(|item| *item == group.0)
    {
        affected.remove(at);
        for member in group.1 {
            if !affected.contains(member) {
                affected.push(*member);
            }
        }
    }
    let present = affected.contains(&category);
    if enabled && !present {
        affected.push(category);
    } else if !enabled && present {
        affected.retain(|item| *item != category);
    }
    set_transition_affected(tx, id, &affected)
}

/// The broad category a value used to be covered by, and everything that covers now.
fn legacy_group_of(
    category: TransitionCategory,
) -> Option<(TransitionCategory, &'static [TransitionCategory])> {
    use TransitionCategory::*;
    const INTERVALS: &[TransitionCategory] = &[
        PopupInterval,
        WebInterval,
        NotificationInterval,
        PromptInterval,
        SoundInterval,
    ];
    const MOVEMENT: &[TransitionCategory] = &[MovementMinimumSpeed, MovementMaximumSpeed];
    const MITOSIS: &[TransitionCategory] = &[MitosisChance, MitosisCount];
    match category {
        PopupInterval | WebInterval | NotificationInterval | PromptInterval | SoundInterval => {
            Some((Events, INTERVALS))
        }
        MovementMinimumSpeed | MovementMaximumSpeed => Some((Movement, MOVEMENT)),
        MitosisChance | MitosisCount => Some((Mitosis, MITOSIS)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

// ── Naming a stage's own tag ─────────────────────────────────────────────────
//
// The editor gives a stage that restricts its content a tag of its own, because the "Appears in"
// strip can only write membership if the stage *has* a tag, and asking authors to invent one per
// stage invites arbitrary names that then mean nothing to anyone reading the pack.
//
// Named from the **label**, not the index: an index-derived name churns on every reorder, while a
// label is already how the author refers to the stage — and it is what makes the tag legible in the
// Tags tab, which is the point of not hiding these behind the reserved `__lewdware-` prefix.
//
// This lives here, next to the tables, rather than in the editor front end, because every caller
// needs the same two things the front end could only approximate: the names the pack currently has,
// and the answer holding still until the write lands.

/// As long a slug as a tag should ever be. Long enough to stay recognisable, short enough that a
/// label somebody pasted a sentence into does not become the widest chip in the Tags tab.
const MAX_SLUG: usize = 40;

/// The label as a tag-shaped word: lower case, runs of anything that is not a letter or a digit
/// collapsed to one dash.
///
/// Letters and digits by Unicode property rather than by `[a-z0-9]`, so a label that is not written
/// in Latin script produces a readable tag instead of an empty one.
pub fn slugify_stage_label(label: &str) -> String {
    let mut slug = String::new();
    for ch in label.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    // By character, not by byte: `MAX_SLUG` is a length an author would recognise, and slicing a
    // string of non-Latin letters by bytes would both cut it far shorter than asked and panic on
    // the boundary.
    if slug.chars().count() <= MAX_SLUG {
        return slug.to_string();
    }
    let cut: String = slug.chars().take(MAX_SLUG).collect();
    // Cut at a dash where there is one to cut at, so the truncation lands between words.
    let cut = match cut.rfind('-') {
        Some(boundary) if boundary > 0 => &cut[..boundary],
        _ => &cut[..],
    };
    cut.trim_end_matches('-').to_string()
}

/// The name to give the tag `label`'s stage owns, avoiding every name the pack already has.
///
/// Deduped against *every* tag, not just other stages': an owned tag is an ordinary tag, so
/// colliding with `stage-intro` would mean adopting whatever that is already on. Predictable beats
/// clever — a second stage called "Intro" gets `stage-intro-2` rather than silently inheriting
/// forty-seven files' worth of classification.
///
/// A label with nothing tag-shaped in it (empty, or only punctuation) falls back to `stage`, which
/// then dedupes like any other. A label that already says "stage" keeps the prefix it has: "Stage
/// 3" is `stage-3`, not `stage-stage-3` — the prefix marks the tag as a stage's and the label has
/// done that job already. Only the word on its own counts, so "Stages of grief" is still
/// `stage-stages-of-grief`; the label is about stages, it is not one.
///
/// `keeping` is the name the caller already holds and may keep — a rename must not dedupe against
/// the very tag it is renaming.
pub fn stage_tag_name(conn: &Connection, label: &str, keeping: Option<&str>) -> Result<String> {
    named_after_stage(conn, "", label, keeping)
}

/// The name for the tag that keeps files *out* of `label`'s stage.
///
/// `not-stage-peak`, so a file's tag list reads as a sentence about the file and the pair is
/// obviously one stage's machinery when the two sit next to each other in the Tags tab.
pub fn stage_exclude_tag_name(
    conn: &Connection,
    label: &str,
    keeping: Option<&str>,
) -> Result<String> {
    named_after_stage(conn, "not-", label, keeping)
}

fn named_after_stage(
    conn: &Connection,
    prefix: &str,
    label: &str,
    keeping: Option<&str>,
) -> Result<String> {
    let slug = slugify_stage_label(label);
    let base = if slug.is_empty() {
        format!("{prefix}stage")
    } else if slug == "stage" || slug.starts_with("stage-") {
        format!("{prefix}{slug}")
    } else {
        format!("{prefix}stage-{slug}")
    };
    // Every tag in the pack, not only the ones on stages: a caption's tag and a content group's
    // are real rows here too, and landing on one would start writing that classification onto files.
    let mut statement = conn.prepare("SELECT name FROM tags")?;
    let taken: HashSet<String> = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .filter(|name| !matches!((name, keeping), (Ok(name), Some(keep)) if name == keep))
        .collect::<rusqlite::Result<_>>()?;
    if !taken.contains(&base) {
        return Ok(base);
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !taken.contains(&candidate) {
            return Ok(candidate);
        }
    }
    unreachable!("the suffix search always terminates: the pack has finitely many tags")
}

/// The tags this stage owns — the ones the editor names after it.
pub fn stage_owned_tags(conn: &Connection, id: &str) -> Result<Vec<OwnedStageTag>> {
    let mut statement = conn.prepare(
        "SELECT t.name, st.excluded FROM behaviour_stage_tag AS st
         JOIN tags AS t ON t.id = st.tag_id
         WHERE st.stage_id = ?1 AND st.owned = 1
         ORDER BY st.excluded, st.position",
    )?;
    let tags = statement
        .query_map(params![id], |row| {
            Ok(OwnedStageTag {
                tag: row.get(0)?,
                excluded: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tags)
}

/// A tag the editor created for a stage, and which side of its selection it is on.
///
/// A stage can own one of each — `stage-peak` to put files in and `not-stage-peak` to keep them
/// out — so everything that follows a stage around has to handle *both*. Renaming a stage renames
/// the pair; removing one retires the pair. Returning a single tag is how one of them gets orphaned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedStageTag {
    pub tag: String,
    pub excluded: bool,
}

/// Whether any stage other than `except` selects by `tag`.
///
/// The question a rename asks before it renames: a tag another stage reads by name is another
/// author's decision, and this one is bookkeeping.
pub fn stage_tag_shared(conn: &Connection, tag: &str, except: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM behaviour_stage_tag AS st
             JOIN tags AS t ON t.id = st.tag_id
             WHERE t.name = ?1 AND st.stage_id <> ?2
         )",
        params![tag, except],
        |row| row.get::<_, bool>(0),
    )?)
}

/// Every file this stage names in a slot of its own: its scenery.
///
/// What a removal deliberately lets go of — a file that was only ever this stage's wallpaper leaves
/// with it, exactly as the slot's own Remove does. Derived here rather than sent by the caller so
/// that the list is the one the pack actually holds at the moment of the write.
pub fn stage_scenery(conn: &Connection, id: &str) -> Result<Vec<u64>> {
    let Some(stage) = read_one_stage(conn, id)? else {
        return Ok(Vec::new());
    };
    Ok([
        stage.content.wallpaper,
        stage.content.audio,
        stage.on_enter.splash,
        stage.on_enter.sound,
        stage.prompt.sound,
    ]
    .into_iter()
    .flatten()
    .collect())
}

/// One stage's label, without reading the rest of it.
pub fn read_one_stage_label(conn: &Connection, id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT label FROM behaviour_stage WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()?)
}

/// The background track this stage names, if it names one rather than picking at random.
pub fn stage_audio(conn: &Connection, id: &str) -> Result<Option<u64>> {
    Ok(read_one_stage(conn, id)?.and_then(|stage| stage.content.audio))
}

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

/// Whether `tags` puts a file in this stage.
///
/// Included *and* not excluded. The order matters for reading it, not for the answer: exclusion
/// always wins, which is what makes it usable as "take this one file out" regardless of why the
/// file was in.
fn selects(stage: &Stage, tags: &[String]) -> bool {
    let included = match stage.content.tags.as_deref() {
        // No inclusion list at all is a stage that restricts nothing: every file appears in it.
        None => true,
        Some(restriction) => restriction.iter().any(|tag| tags.contains(tag)),
    };
    included && !stage.content.exclude.iter().any(|tag| tags.contains(tag))
}

/// The tag a membership toggle can add to `stage` without changing any other relationship.
///
/// Only the stage's own tag, and only while this stage is its sole reference anywhere: the moment a
/// caption or a content group also names it, adding it to a file says more than "appears here".
fn dedicated_owned_tag(conn: &Connection, stage: &Stage, excluded: bool) -> Result<Option<String>> {
    let (owned, side) = if excluded {
        (stage.content.owned_exclude_tag.as_deref(), &stage.content.exclude[..])
    } else {
        (
            stage.content.owned_tag.as_deref(),
            stage.content.tags.as_deref().unwrap_or(&[]),
        )
    };
    let Some(owned) = owned else {
        return Ok(None);
    };
    if !side.iter().any(|tag| tag == owned) {
        return Ok(None);
    }
    let (content, experience): (u64, u64) = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM behaviour_content_group_tag t WHERE t.tag_id = tags.id)
                  + (SELECT COUNT(*) FROM behaviour_text_item_tag t WHERE t.tag_id = tags.id)
                  + (SELECT COUNT(*) FROM behaviour_web_link_tag t WHERE t.tag_id = tags.id),
                (SELECT COUNT(*) FROM behaviour_stage_tag t WHERE t.tag_id = tags.id)
         FROM tags WHERE tags.name = ?1",
        params![owned],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok((content == 0 && experience == 1).then(|| owned.to_string()))
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
    let Some(target) = read_one_stage(conn, stage_id)? else {
        bail!("no stage {stage_id}");
    };
    let tags = media_tags(conn, media)?;
    let mut plan = MembershipPlan::default();
    if selects(&target, &tags) == member {
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
        if selects(&target, &without) {
            return Ok(plan);
        }
        let tag = match dedicated_owned_tag(conn, &target, false)? {
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
    if let Some(owned) = dedicated_owned_tag(conn, &target, false)?
        && tags.contains(&owned)
    {
        let without: Vec<String> = tags.iter().filter(|tag| **tag != owned).cloned().collect();
        if !selects(&target, &without) {
            plan.remove.push(owned);
            return Ok(plan);
        }
    }

    // Otherwise the file is here through the author's own tags, or because the stage restricts
    // nothing at all. Either way it leaves by being excluded.
    let tag = match dedicated_owned_tag(conn, &target, true)? {
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
pub fn apply_membership_creation(tx: &Transaction<'_>, plan: &MembershipPlan) -> Result<bool> {
    let Some(creation) = &plan.creation else {
        return Ok(false);
    };
    let Some(stage) = read_one_stage(tx, &creation.stage)? else {
        return Ok(false);
    };
    if creation.excluded {
        let mut exclude = stage.content.exclude;
        exclude.push(creation.tag.clone());
        return set_stage_exclude_tags(tx, &creation.stage, &exclude, Some(&creation.tag));
    }
    let mut tags = stage.content.tags.unwrap_or_default();
    tags.push(creation.tag.clone());
    set_stage_content_tags(tx, &creation.stage, Some(&tags), Some(&creation.tag))
}

/// Whether the stage restricts what it shows, rather than showing the whole pack.
///
/// The three states of `ContentSelection::tags` collapse to two here: an inclusion list, however
/// empty, is a restriction; its absence is not. See `behaviour_stage.restricts_content`.
pub fn stage_restricts_content(conn: &Connection, id: &str) -> Result<bool> {
    Ok(read_one_stage(conn, id)?.is_some_and(|stage| stage.content.tags.is_some()))
}
