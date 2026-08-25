//! Reading and writing a [`Behaviour`] against the pack database.
//!
//! The whole document lives in the `behaviour_*` tables (see `migrations/behaviour_schema.sql`) —
//! the media slots, the timeline, and the content pools alike — so that a slot is a foreign key
//! the database enforces, a tag is a join rather than a name copied into seven lists, and editing
//! one stage produces a changeset the size of that stage. Nothing is left in the `pack_data` blob
//! the document used to live in; see [`LEGACY_BLOB_KEY`].
//!
//! [`read`] assembles the tables into the one `Behaviour` every other layer already understands,
//! and [`write`] takes it apart again. Nothing above this module knows about the split: the engine
//! still gets a `Behaviour`, the editor still patches a `Behaviour`, and the converter still
//! produces one.
//!
//! **Writes diff rather than replace.** Deleting every row and reinserting would be far simpler,
//! and would defeat the point: the editor's undo is a SQLite session changeset, so a wholesale
//! rewrite would record every stage in the pack as changed on every keystroke — exactly the cost
//! moving out of the blob was meant to remove. So each table is reconciled: rows that are gone are
//! deleted, rows that are new are inserted, and rows that already match are left alone.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Serialize, de::DeserializeOwned};

use crate::behaviour::SpawnRegion;

use super::schema::{
    AudioMedia, Behaviour, Content, ContentGroup, ContentSelection, EventCountCondition,
    EventSchedule, Events, Experience, Interval, Mitosis, Movement, PopupMedia, Stage, StageEnd,
    StageEntry, StagePrompt, TextItem, Timeline, Transition, WebLink,
};

/// The `pack_data` key the document used to live under.
///
/// Nothing writes it any more. Its presence is how [`read`] recognises a pack from before the
/// content pools moved into tables, which it refuses rather than silently reading as empty.
const LEGACY_BLOB_KEY: &str = "behaviour";

/// The pack's behaviour document, or a default one if the pack has never had it written.
pub fn read(conn: &Connection) -> Result<Behaviour> {
    reject_legacy_blob(conn)?;
    let (wallpaper, splash) = conn
        .query_row(
            "SELECT wallpaper, splash FROM behaviour_content WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, Option<u64>>(0)?, row.get::<_, Option<u64>>(1)?)),
        )
        .optional()?
        .unwrap_or((None, None));
    Ok(Behaviour {
        content: Content {
            popups: read_popup_media(conn)?,
            audio: read_audio_media(conn)?,
            content_groups: read_content_groups(conn)?,
            captions: read_text_pool(conn, "caption")?,
            prompts: read_text_pool(conn, "prompt")?,
            notifications: read_text_pool(conn, "notification")?,
            web_links: read_web_links(conn)?,
            wallpaper,
            splash,
        },
        experience: read_experience(conn)?,
    })
}

/// Replaces the stored document with `behaviour`, touching only the rows that actually differ.
pub fn write(tx: &Transaction<'_>, behaviour: &Behaviour) -> Result<()> {
    let content = &behaviour.content;
    tx.execute(
        "INSERT INTO behaviour_content (singleton, wallpaper, splash) VALUES (1, ?, ?)
         ON CONFLICT(singleton) DO UPDATE SET wallpaper = excluded.wallpaper,
                                              splash = excluded.splash",
        params![content.wallpaper, content.splash],
    )?;
    write_popup_media(tx, &content.popups)?;
    write_audio_media(tx, &content.audio)?;
    write_content_groups(tx, &content.content_groups)?;
    write_text_pool(tx, "caption", &content.captions)?;
    write_text_pool(tx, "prompt", &content.prompts)?;
    write_text_pool(tx, "notification", &content.notifications)?;
    write_web_links(tx, &content.web_links)?;
    write_experience(tx, behaviour.experience.as_ref())
}

fn reject_legacy_blob(conn: &Connection) -> Result<()> {
    let present: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pack_data WHERE name = ?)",
        params![LEGACY_BLOB_KEY],
        |row| row.get(0),
    )?;
    if present {
        bail!(
            "this pack's behaviour document predates the behaviour tables; re-import the pack in \
             the pack editor"
        );
    }
    Ok(())
}

// ── Per-item media attributes ────────────────────────────────────────────────
//
// Keyed by media id rather than by position, which is what lets an edit address one file's
// attributes directly and lets `ON DELETE CASCADE` do the work `clear_media_reference` has to do
// by hand for the slots. An entry that says nothing is not stored (`PopupMedia::is_empty`), so
// clearing the last attribute removes the row rather than leaving a row of NULLs behind.

fn read_popup_media(conn: &Connection) -> Result<BTreeMap<u64, PopupMedia>> {
    read_popup_media_where(conn, "", &[])
}

/// One file's popup attributes, or `None` if the author has said nothing about it.
///
/// The editor asks about one file (or a selection) far more often than about the whole pack, and
/// answering that by reading every entry — plus a pairings query each — is the O(document) cost
/// this module exists to stop paying.
pub(super) fn read_one_popup_media(conn: &Connection, id: u64) -> Result<Option<PopupMedia>> {
    Ok(read_popup_media_where(conn, "WHERE media_id = ?", &[&id])?.remove(&id))
}

fn read_popup_media_where(
    conn: &Connection,
    filter: &str,
    binds: &[&dyn rusqlite::ToSql],
) -> Result<BTreeMap<u64, PopupMedia>> {
    let mut statement = conn.prepare(&format!(
        "SELECT media_id, weight, scale, region_x, region_y, region_width, region_height,
                monitor, caption, video_loop, video_audio
         FROM behaviour_popup_media {filter} ORDER BY media_id"
    ))?;
    type Row = (
        u64,
        Option<f64>,
        Option<f64>,
        (Option<f64>, Option<f64>, Option<f64>, Option<f64>),
        Option<String>,
        Option<String>,
        Option<bool>,
        Option<bool>,
    );
    let rows: Vec<Row> = statement
        .query_map(binds, |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                (row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?),
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
                row.get(10)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(
            |(media_id, weight, scale, region, monitor, caption, video_loop, video_audio)| {
                Ok((
                    media_id,
                    PopupMedia {
                        weight,
                        scale,
                        region: read_region(region),
                        monitor: monitor.as_deref().map(from_text).transpose()?,
                        caption,
                        video_loop,
                        video_audio,
                        audio: read_popup_audio_pairs(conn, media_id)?,
                    },
                ))
            },
        )
        .collect()
}

/// The four region columns as one value: a region exists only when all four are present.
///
/// A partial row is treated as no region rather than as a rectangle with invented edges — the
/// four columns are one field split across a table, and half of a rectangle is not a placement
/// anybody asked for. `sanitized` then keeps a hand-edited pack from reaching the engine with a
/// NaN or an inverted extent.
fn read_region(
    columns: (Option<f64>, Option<f64>, Option<f64>, Option<f64>),
) -> Option<SpawnRegion> {
    let (Some(x), Some(y), Some(width), Some(height)) = columns else {
        return None;
    };
    Some(
        SpawnRegion {
            x,
            y,
            width,
            height,
        }
        .sanitized(),
    )
}

/// Ordered by id purely so the document round-trips: the pairings are a set, and the mode picks
/// one of them at random.
fn read_popup_audio_pairs(conn: &Connection, popup: u64) -> Result<Vec<u64>> {
    let mut statement = conn.prepare(
        "SELECT audio_media_id FROM behaviour_popup_audio_pair
         WHERE popup_media_id = ? ORDER BY audio_media_id",
    )?;
    Ok(statement
        .query_map(params![popup], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

fn write_popup_media(tx: &Transaction<'_>, popups: &BTreeMap<u64, PopupMedia>) -> Result<()> {
    let kept: Vec<u64> = popups
        .iter()
        .filter(|(_, entry)| !entry.is_empty())
        .map(|(media_id, _)| *media_id)
        .collect();
    // Deleting the parent takes its pairings with it, so the pair table needs no retain of its own
    // for entries that went away entirely.
    retain_ids(tx, "behaviour_popup_media", "media_id", &kept)?;
    for (media_id, entry) in popups {
        if entry.is_empty() {
            continue;
        }
        write_one_popup_media(tx, *media_id, entry)?;
    }
    Ok(())
}

/// Writes one file's popup attributes and its explicit sound pairings.
///
/// The body of [`write_popup_media`]' loop, separated so the editor can set one file's attributes
/// without re-upserting every other entry in the pack.
pub(super) fn write_one_popup_media(
    tx: &Transaction<'_>,
    media_id: u64,
    entry: &PopupMedia,
) -> Result<()> {
    let region = entry.region.map(SpawnRegion::sanitized);
    let monitor = entry.monitor.as_ref().map(to_text).transpose()?;
    tx.execute(
        "INSERT INTO behaviour_popup_media
             (media_id, weight, scale, region_x, region_y, region_width, region_height,
              monitor, caption, video_loop, video_audio)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(media_id) DO UPDATE SET weight = excluded.weight,
                                             scale = excluded.scale,
                                             region_x = excluded.region_x,
                                             region_y = excluded.region_y,
                                             region_width = excluded.region_width,
                                             region_height = excluded.region_height,
                                             monitor = excluded.monitor,
                                             caption = excluded.caption,
                                             video_loop = excluded.video_loop,
                                             video_audio = excluded.video_audio",
        params![
            media_id,
            entry.weight,
            entry.scale,
            region.map(|region| region.x),
            region.map(|region| region.y),
            region.map(|region| region.width),
            region.map(|region| region.height),
            monitor,
            entry.caption,
            entry.video_loop,
            entry.video_audio,
        ],
    )?;
    write_popup_audio_pairs(tx, media_id, &entry.audio)?;
    Ok(())
}

fn write_popup_audio_pairs(tx: &Transaction<'_>, popup: u64, audio: &[u64]) -> Result<()> {
    tx.execute(
        "DELETE FROM behaviour_popup_audio_pair
         WHERE popup_media_id = ?1
           AND audio_media_id NOT IN (SELECT value FROM json_each(?2))",
        params![popup, serde_json::to_string(audio)?],
    )?;
    for audio_media_id in audio {
        tx.execute(
            "INSERT OR IGNORE INTO behaviour_popup_audio_pair (popup_media_id, audio_media_id)
             VALUES (?, ?)",
            params![popup, audio_media_id],
        )?;
    }
    Ok(())
}

fn read_audio_media(conn: &Connection) -> Result<BTreeMap<u64, AudioMedia>> {
    let mut statement =
        conn.prepare("SELECT media_id, volume FROM behaviour_audio_media ORDER BY media_id")?;
    Ok(statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                AudioMedia {
                    volume: row.get(1)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?)
}

fn write_audio_media(tx: &Transaction<'_>, audio: &BTreeMap<u64, AudioMedia>) -> Result<()> {
    let kept: Vec<u64> = audio
        .iter()
        .filter(|(_, entry)| !entry.is_empty())
        .map(|(media_id, _)| *media_id)
        .collect();
    retain_ids(tx, "behaviour_audio_media", "media_id", &kept)?;
    for (media_id, entry) in audio {
        if entry.is_empty() {
            continue;
        }
        tx.execute(
            "INSERT INTO behaviour_audio_media (media_id, volume)
             VALUES (?, ?)
             ON CONFLICT(media_id) DO UPDATE SET volume = excluded.volume",
            params![media_id, entry.volume],
        )?;
    }
    Ok(())
}

// ── The content pools ────────────────────────────────────────────────────────

pub(super) fn read_text_pool(conn: &Connection, kind: &str) -> Result<Vec<TextItem>> {
    let mut statement = conn.prepare(
        "SELECT id, text, timeout_seconds, summary FROM behaviour_text_item WHERE kind = ? \
         ORDER BY position",
    )?;
    let rows: Vec<(i64, String, Option<f64>, Option<String>)> = statement
        .query_map(params![kind], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, text, timeout_seconds, summary)| {
            Ok(TextItem {
                text,
                tags: read_tags(conn, "behaviour_text_item_tag", "item_id", &id)?,
                timeout_seconds,
                summary,
            })
        })
        .collect()
}

fn write_text_pool(tx: &Transaction<'_>, kind: &str, items: &[TextItem]) -> Result<()> {
    // Pool entries have no author-facing identity -- nothing outside the document refers to a
    // caption -- so they are keyed by position and rewritten in place, rather than matched up.
    tx.execute(
        "DELETE FROM behaviour_text_item WHERE kind = ?1 AND position >= ?2",
        params![kind, items.len() as i64],
    )?;
    for (position, item) in items.iter().enumerate() {
        let id: i64 = tx.query_row(
            "INSERT INTO behaviour_text_item (kind, position, text, timeout_seconds, summary)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(kind, position) DO UPDATE SET text = excluded.text,
                                                       timeout_seconds = excluded.timeout_seconds,
                                                       summary = excluded.summary
             RETURNING id",
            params![
                kind,
                position as i64,
                item.text,
                item.timeout_seconds,
                item.summary
            ],
            |row| row.get(0),
        )?;
        write_tags(tx, "behaviour_text_item_tag", "item_id", &id, &item.tags)?;
    }
    Ok(())
}

pub(super) fn read_web_links(conn: &Connection) -> Result<Vec<WebLink>> {
    let mut statement = conn.prepare("SELECT id, url FROM behaviour_web_link ORDER BY position")?;
    let rows: Vec<(i64, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;

    let mut args = conn
        .prepare("SELECT value FROM behaviour_web_link_arg WHERE link_id = ? ORDER BY position")?;
    rows.into_iter()
        .map(|(id, url)| {
            Ok(WebLink {
                url,
                args: args
                    .query_map(params![id], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?,
                tags: read_tags(conn, "behaviour_web_link_tag", "link_id", &id)?,
            })
        })
        .collect()
}

fn write_web_links(tx: &Transaction<'_>, links: &[WebLink]) -> Result<()> {
    tx.execute(
        "DELETE FROM behaviour_web_link WHERE position >= ?",
        params![links.len() as i64],
    )?;
    for (position, link) in links.iter().enumerate() {
        let id: i64 = tx.query_row(
            "INSERT INTO behaviour_web_link (position, url) VALUES (?1, ?2)
             ON CONFLICT(position) DO UPDATE SET url = excluded.url
             RETURNING id",
            params![position as i64, link.url],
            |row| row.get(0),
        )?;
        tx.execute(
            "DELETE FROM behaviour_web_link_arg WHERE link_id = ?1 AND position >= ?2",
            params![id, link.args.len() as i64],
        )?;
        for (position, value) in link.args.iter().enumerate() {
            tx.execute(
                "INSERT INTO behaviour_web_link_arg (link_id, position, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(link_id, position) DO UPDATE SET value = excluded.value",
                params![id, position as i64, value],
            )?;
        }
        write_tags(tx, "behaviour_web_link_tag", "link_id", &id, &link.tags)?;
    }
    Ok(())
}

pub(super) fn read_content_groups(conn: &Connection) -> Result<Vec<ContentGroup>> {
    let mut statement = conn.prepare(
        "SELECT id, label, description, enabled_by_default
         FROM behaviour_content_group ORDER BY position",
    )?;
    let rows: Vec<(String, String, Option<String>, bool)> = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, label, description, enabled_by_default)| {
            Ok(ContentGroup {
                tags: read_tags(conn, "behaviour_content_group_tag", "group_id", &id)?,
                id,
                label,
                description,
                enabled_by_default,
            })
        })
        .collect()
}

fn write_content_groups(tx: &Transaction<'_>, groups: &[ContentGroup]) -> Result<()> {
    retain(
        tx,
        "behaviour_content_group",
        "id",
        groups.iter().map(|group| group.id.as_str()),
    )?;
    for (position, group) in groups.iter().enumerate() {
        tx.execute(
            "INSERT INTO behaviour_content_group
                 (id, position, label, description, enabled_by_default)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET position = excluded.position,
                                           label = excluded.label,
                                           description = excluded.description,
                                           enabled_by_default = excluded.enabled_by_default",
            params![
                group.id,
                position as i64,
                group.label,
                group.description,
                group.enabled_by_default,
            ],
        )?;
        write_tags(
            tx,
            "behaviour_content_group_tag",
            "group_id",
            &group.id,
            &group.tags,
        )?;
    }
    Ok(())
}

// ── Tags ─────────────────────────────────────────────────────────────────────
//
// Every tag list in the document is a join to `tags`. That is what makes renaming a tag
// `UPDATE tags SET name`, merging one a re-point, and deleting one a cascade, instead of a pass
// over the whole document rewriting strings (the late `Behaviour::rewrite_tag`).

pub(super) fn read_tags(
    conn: &Connection,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
) -> Result<Vec<String>> {
    let mut statement = conn.prepare(&format!(
        "SELECT tags.name FROM {table}
         JOIN tags ON tags.id = {table}.tag_id
         WHERE {table}.{key} = ? ORDER BY {table}.position"
    ))?;
    let tags = statement
        .query_map([owner], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tags)
}

pub(super) fn write_tags(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
    tags: &[String],
) -> Result<()> {
    let ids = tags
        .iter()
        .map(|tag| tag_id(tx, tag))
        .collect::<Result<Vec<_>>>()?;
    tx.execute(
        &format!(
            "DELETE FROM {table} WHERE {key} = ?1
             AND tag_id NOT IN (SELECT value FROM json_each(?2))"
        ),
        params![owner, serde_json::to_string(&ids)?],
    )?;
    for (position, id) in ids.iter().enumerate() {
        tx.execute(
            &format!(
                "INSERT INTO {table} ({key}, tag_id, position) VALUES (?1, ?2, ?3)
                 ON CONFLICT({key}, tag_id) DO UPDATE SET position = excluded.position"
            ),
            params![owner, id, position as i64],
        )?;
    }
    Ok(())
}

/// The id of `name`, creating the tag if the pack does not have it yet.
///
/// A tag the document names *is* a tag the pack has -- that is what the foreign key asserts. So a
/// tag typed into a caption or a content group becomes a real row, and shows up in the Tags tab
/// with no media attached, rather than existing only as a string inside the document.
pub(super) fn tag_id(tx: &Transaction<'_>, name: &str) -> Result<i64> {
    tx.execute(
        "INSERT OR IGNORE INTO tags (name) VALUES (?)",
        params![name],
    )?;
    Ok(
        tx.query_row("SELECT id FROM tags WHERE name = ?", params![name], |row| {
            row.get(0)
        })?,
    )
}

// ── The experience section ───────────────────────────────────────────────────

/// The timeline as the *editor* sees it: present even while switched off.
///
/// [`read`] deliberately cannot express this -- a suspended timeline reads as no timeline, which is
/// what keeps suspension invisible to the engine. The editor needs the other view, because the
/// stages it must still render are exactly the ones `Experience: None` is hiding.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineView {
    /// The stages and transitions, whether or not the timeline is switched on.
    pub timeline: Timeline,
    /// The mode-name override, if the author set one.
    pub label: Option<String>,
    /// Whether the pack plays this timeline. `false` is a timeline the author switched off.
    pub enabled: bool,
}

/// The pack's timeline, or `None` if it has never had one.
pub fn read_timeline(conn: &Connection) -> Result<Option<TimelineView>> {
    let row: Option<(Option<String>, bool)> = conn
        .query_row(
            "SELECT label, enabled FROM behaviour_experience WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((label, enabled)) = row else {
        return Ok(None);
    };
    Ok(Some(TimelineView {
        timeline: Timeline {
            stages: read_stages(conn)?,
            transitions: read_transitions(conn)?,
        },
        label,
        enabled,
    }))
}

/// Switches the pack's timeline on or off, keeping its stages either way.
///
/// Creates the section if the pack has never had one, so switching on from nothing is the same
/// call as switching back on. Returns whether anything changed, which is what keeps a toggle
/// landing on its current value from costing an undo entry.
pub fn set_experience_enabled(tx: &Transaction<'_>, enabled: bool) -> Result<bool> {
    let current: Option<bool> = tx
        .query_row(
            "SELECT enabled FROM behaviour_experience WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if current == Some(enabled) {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO behaviour_experience (singleton, label, enabled) VALUES (1, NULL, ?)
         ON CONFLICT(singleton) DO UPDATE SET enabled = excluded.enabled",
        params![enabled],
    )?;
    Ok(true)
}

/// Sets the timeline's mode-name override, or clears it with `None`.
///
/// Does nothing if the pack has no timeline section: a name for a timeline that does not exist is
/// a stale editor writing into something that has since gone, not a reason to create one.
pub fn set_experience_label(tx: &Transaction<'_>, label: Option<&str>) -> Result<()> {
    tx.execute(
        "UPDATE behaviour_experience SET label = ? WHERE singleton = 1",
        params![label],
    )?;
    Ok(())
}

fn read_experience(conn: &Connection) -> Result<Option<Experience>> {
    let row: Option<(Option<String>, bool)> = conn
        .query_row(
            "SELECT label, enabled FROM behaviour_experience WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    // No row at all is a pack with no timeline; a row with a NULL label is a timeline that simply
    // doesn't override the mode's name.
    let Some((label, enabled)) = row else {
        return Ok(None);
    };
    // A suspended timeline reads exactly as no timeline, which is what keeps this change invisible
    // to the engine, the converter and `lw`. Its stage rows are still there -- see
    // `read_timeline`, which is how the editor gets at them.
    if !enabled {
        return Ok(None);
    }
    Ok(Some(Experience {
        timeline: Timeline {
            stages: read_stages(conn)?,
            transitions: read_transitions(conn)?,
        },
        label,
    }))
}

pub(super) fn read_stages(conn: &Connection) -> Result<Vec<Stage>> {
    read_stages_where(conn, "", &[])
}

/// One stage, or `None` if the timeline no longer holds it.
///
/// The editor reads a single stage far more often than it reads the whole timeline — every
/// keystroke in a stage's label compares against it — so this narrows the query rather than
/// filtering [`read_stages`]' result, which would read every stage and its seven child tables to
/// answer a question about one.
pub(super) fn read_one_stage(conn: &Connection, id: &str) -> Result<Option<Stage>> {
    Ok(read_stages_where(conn, "WHERE id = ?", &[&id])?.pop())
}

fn read_stages_where(
    conn: &Connection,
    filter: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<Stage>> {
    let mut statement = conn.prepare(&format!(
        "SELECT id, label, restricts_content, wallpaper, audio, audio_random FROM behaviour_stage \
         {filter} ORDER BY position"
    ))?;

    statement
        .query_and_then(params, |row| -> anyhow::Result<_> {
            let id: String = row.get("id")?;

            Ok(Stage {
                content: ContentSelection {
                    tags: if row.get("restricts_content")? {
                        Some(read_stage_tags(conn, &id, false)?)
                    } else {
                        None
                    },
                    exclude: read_stage_tags(conn, &id, true)?,
                    owned_tag: read_owned_stage_tag(conn, &id, false)?,
                    owned_exclude_tag: read_owned_stage_tag(conn, &id, true)?,
                    wallpaper: row.get("wallpaper")?,
                    audio: row.get("audio")?,
                    audio_random: row.get("audio_random")?,
                },
                end: read_stage_end(conn, &id)?,
                events: read_stage_events(conn, &id)?,
                movement: read_stage_movement(conn, &id)?,
                mitosis: read_stage_mitosis(conn, &id)?,
                on_enter: read_stage_entry(conn, &id)?.unwrap_or_default(),
                prompt: read_stage_prompt(conn, &id)?,
                id,
                label: row.get("label")?,
            })
        })?
        .collect()
}

pub(super) fn read_stage_entry(conn: &Connection, stage: &str) -> Result<Option<StageEntry>> {
    Ok(conn.query_row(
        "SELECT splash_media, sound_media, popup_burst, notification_text FROM behaviour_stage_entry WHERE stage_id = ?",
        params![stage],
        |row| Ok(StageEntry { splash: row.get(0)?, sound: row.get(1)?, popup_burst: row.get(2)?, notification: row.get(3)? }),
    ).optional()?)
}

pub(super) fn read_stage_prompt(conn: &Connection, stage: &str) -> Result<StagePrompt> {
    Ok(conn.query_row(
        "SELECT timeouts_enabled, timeout_multiplier, popup_burst, sound_media FROM behaviour_stage_prompt WHERE stage_id = ?",
        params![stage],
        |row| Ok(StagePrompt { timeouts_enabled: row.get(0)?, timeout_multiplier: row.get(1)?, popup_burst: row.get(2)?, sound: row.get(3)? }),
    ).optional()?.unwrap_or_default())
}

/// The tag this stage owns, if it has one.
///
/// Reads the flag off the association row rather than off the stage, which is what keeps ownership
/// and membership from drifting apart: a tag the stage no longer selects by has no row to carry the
/// flag, so it is no longer owned. `LIMIT 1` because a stage owns at most one — the editor creates
/// exactly one per stage — and a document that somehow named two would be an ambiguity, not a
/// feature.
fn read_owned_stage_tag(conn: &Connection, stage: &str, excluded: bool) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT tags.name FROM behaviour_stage_tag
             JOIN tags ON tags.id = behaviour_stage_tag.tag_id
             WHERE behaviour_stage_tag.stage_id = ?1 AND behaviour_stage_tag.owned = 1
               AND behaviour_stage_tag.excluded = ?2
             ORDER BY behaviour_stage_tag.position LIMIT 1",
            params![stage, excluded],
            |row| row.get(0),
        )
        .optional()?)
}

/// One side of a stage's selection: the tags it includes by, or the tags it excludes by.
pub(super) fn read_stage_tags(conn: &Connection, stage: &str, excluded: bool) -> Result<Vec<String>> {
    let mut statement = conn.prepare(
        "SELECT tags.name FROM behaviour_stage_tag
         JOIN tags ON tags.id = behaviour_stage_tag.tag_id
         WHERE behaviour_stage_tag.stage_id = ?1 AND behaviour_stage_tag.excluded = ?2
         ORDER BY behaviour_stage_tag.position",
    )?;
    let tags = statement
        .query_map(params![stage, excluded], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tags)
}

/// Both sides at once, because they share a table and a primary key.
///
/// Written together rather than through [`write_tags`] twice: the rows to delete are the ones on
/// *neither* side, and a per-side pass would delete the other side's rows on its way through. The
/// shared key is also what enforces that a tag is not on both sides — an `INSERT` moving a tag
/// across takes the same row with it rather than making a second one.
pub(super) fn write_stage_tags(
    tx: &Transaction<'_>,
    stage: &str,
    include: &[String],
    exclude: &[String],
) -> Result<()> {
    let ids = |tags: &[String]| {
        tags.iter()
            .map(|tag| tag_id(tx, tag))
            .collect::<Result<Vec<_>>>()
    };
    let (included, excluded) = (ids(include)?, ids(exclude)?);
    let keeping: Vec<i64> = included.iter().chain(excluded.iter()).copied().collect();
    tx.execute(
        "DELETE FROM behaviour_stage_tag WHERE stage_id = ?1
         AND tag_id NOT IN (SELECT value FROM json_each(?2))",
        params![stage, serde_json::to_string(&keeping)?],
    )?;
    for (side, ids) in [(false, &included), (true, &excluded)] {
        for (position, id) in ids.iter().enumerate() {
            tx.execute(
                "INSERT INTO behaviour_stage_tag (stage_id, tag_id, position, excluded)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(stage_id, tag_id) DO UPDATE SET position = excluded.position,
                                                             excluded = excluded.excluded",
                params![stage, id, position as i64, side],
            )?;
        }
    }
    Ok(())
}

/// Marks `owned` on the stage's row for that tag, and clears it on the rest.
///
/// Runs after [`write_tags`], which has already reconciled which rows exist: a tag the document no
/// longer lists has no row left to mark, so naming one that is not in `tags` silently owns nothing
/// rather than resurrecting the association. That is the invariant the schema comment states, kept
/// here rather than validated — an owned tag outside the selection is not a document a writer can
/// produce, and refusing it would turn a stale editor's harmless patch into a failed save.
pub(super) fn write_owned_stage_tag(
    tx: &Transaction<'_>,
    stage: &str,
    excluded: bool,
    owned: Option<&str>,
) -> Result<()> {
    tx.execute(
        "UPDATE behaviour_stage_tag SET owned = 0
         WHERE stage_id = ?1 AND owned = 1 AND excluded = ?3
         AND tag_id IS NOT (SELECT id FROM tags WHERE name = ?2)",
        params![stage, owned, excluded],
    )?;
    if let Some(tag) = owned {
        tx.execute(
            "UPDATE behaviour_stage_tag SET owned = 1
             WHERE stage_id = ?1 AND owned = 0 AND excluded = ?3
             AND tag_id = (SELECT id FROM tags WHERE name = ?2)",
            params![stage, tag, excluded],
        )?;
    }
    Ok(())
}

pub(super) fn read_stage_end(conn: &Connection, stage: &str) -> Result<Option<StageEnd>> {
    conn.query_row(
        "SELECT duration_seconds, strategy, event_kind, event_count, event_scope
         FROM behaviour_stage_end WHERE stage_id = ?",
        params![stage],
        |row| {
            Ok((
                row.get::<_, Option<f64>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<u32>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        },
    )
    .optional()?
    .map(|(duration_seconds, strategy, kind, count, scope)| {
        let event_count = match (kind, count, scope) {
            (Some(kind), Some(count), Some(scope)) => Some(EventCountCondition {
                event: from_text(&kind)?,
                count,
                scope: from_text(&scope)?,
            }),
            _ => None,
        };
        Ok(StageEnd {
            duration_seconds,
            event_count,
            strategy: from_text(&strategy)?,
        })
    })
    .transpose()
}

pub(super) fn read_stage_movement(conn: &Connection, stage: &str) -> Result<Option<Movement>> {
    Ok(conn
        .query_row(
            "SELECT minimum_speed, maximum_speed FROM behaviour_stage_movement WHERE stage_id = ?",
            params![stage],
            |row| {
                Ok(Movement {
                    minimum_speed: row.get(0)?,
                    maximum_speed: row.get(1)?,
                })
            },
        )
        .optional()?)
}

pub(super) fn read_stage_mitosis(conn: &Connection, stage: &str) -> Result<Option<Mitosis>> {
    Ok(conn
        .query_row(
            "SELECT chance, count FROM behaviour_stage_mitosis WHERE stage_id = ?",
            params![stage],
            |row| {
                Ok(Mitosis {
                    chance: row.get(0)?,
                    count: row.get(1)?,
                })
            },
        )
        .optional()?)
}

/// One `behaviour_stage_event` row. Which interval columns carry a value depends on
/// `interval_kind`, mirroring the `Interval` enum's own tag.
struct EventRow {
    kind: String,
    interval_kind: String,
    seconds: Option<f64>,
    minimum_seconds: Option<f64>,
    maximum_seconds: Option<f64>,
    initial_delay_seconds: Option<f64>,
    max_concurrent: Option<u32>,
}

pub(super) fn read_stage_events(conn: &Connection, stage: &str) -> Result<Events> {
    let mut statement = conn.prepare(
        "SELECT kind, interval_kind, seconds, minimum_seconds, maximum_seconds,
                initial_delay_seconds, max_concurrent
         FROM behaviour_stage_event WHERE stage_id = ?",
    )?;
    let rows: Vec<EventRow> = statement
        .query_map(params![stage], |row| {
            Ok(EventRow {
                kind: row.get(0)?,
                interval_kind: row.get(1)?,
                seconds: row.get(2)?,
                minimum_seconds: row.get(3)?,
                maximum_seconds: row.get(4)?,
                initial_delay_seconds: row.get(5)?,
                max_concurrent: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let mut events = Events::default();
    for row in rows {
        let interval = match row.interval_kind.as_str() {
            "fixed" => Interval::Fixed {
                seconds: row.seconds.unwrap_or_default(),
            },
            "random" => Interval::Random {
                minimum_seconds: row.minimum_seconds.unwrap_or_default(),
                maximum_seconds: row.maximum_seconds.unwrap_or_default(),
            },
            other => bail!("unknown event interval kind {other:?}"),
        };
        let schedule = Some(EventSchedule {
            interval,
            initial_delay_seconds: row.initial_delay_seconds,
            max_concurrent: row.max_concurrent,
        });
        match row.kind.as_str() {
            "popup" => events.popup = schedule,
            "web" => events.web = schedule,
            "notification" => events.notification = schedule,
            "prompt" => events.prompt = schedule,
            "sound" => events.sound = schedule,
            other => bail!("unknown event kind {other:?}"),
        }
    }
    Ok(events)
}

pub(super) fn read_transitions(conn: &Connection) -> Result<Vec<Transition>> {
    let mut statement = conn.prepare(
        "SELECT id, from_stage, to_stage, duration_seconds, easing
         FROM behaviour_transition ORDER BY position",
    )?;
    let rows: Vec<(String, String, String, f64, String)> = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;

    let mut categories = conn.prepare(
        "SELECT category FROM behaviour_transition_category
         WHERE transition_id = ? ORDER BY position",
    )?;
    rows.into_iter()
        .map(|(id, from_stage, to_stage, duration_seconds, easing)| {
            let affected: Vec<String> = categories
                .query_map(params![&id], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(Transition {
                id,
                from_stage,
                to_stage,
                duration_seconds,
                easing: from_text(&easing)?,
                affected: affected
                    .iter()
                    .map(|value| from_text(value))
                    .collect::<Result<_>>()?,
            })
        })
        .collect()
}

fn write_experience(tx: &Transaction<'_>, experience: Option<&Experience>) -> Result<()> {
    let Some(experience) = experience else {
        // The pack has no timeline at all -- what a converted pack without one produces. This is
        // not suspension: switching a timeline off keeps its rows and clears `enabled` instead
        // (`set_experience_enabled`), because absence cannot hold anything to switch back on.
        // The media the stages pointed at is deliberately *not* retired either way.
        tx.execute("DELETE FROM behaviour_experience", [])?;
        tx.execute("DELETE FROM behaviour_stage", [])?;
        tx.execute("DELETE FROM behaviour_transition", [])?;
        return Ok(());
    };
    // `enabled` is set rather than left alone: writing a document that *has* a timeline is the
    // whole-document path's way of saying the pack has one, and a stale suspension flag would
    // hide it.
    tx.execute(
        "INSERT INTO behaviour_experience (singleton, label, enabled) VALUES (1, ?, 1)
         ON CONFLICT(singleton) DO UPDATE SET label = excluded.label, enabled = 1",
        params![experience.label],
    )?;
    // Stages first: a transition's endpoints are foreign keys, so a transition into a stage this
    // write is adding can only be inserted once that stage exists. Removing a stage cascades to
    // the transitions touching it, which is why they are reconciled afterwards rather than before.
    write_stages(tx, &experience.timeline.stages)?;
    write_transitions(tx, &experience.timeline.transitions)?;
    Ok(())
}

fn write_stages(tx: &Transaction<'_>, stages: &[Stage]) -> Result<()> {
    retain(
        tx,
        "behaviour_stage",
        "id",
        stages.iter().map(|s| s.id.as_str()),
    )?;
    for (position, stage) in stages.iter().enumerate() {
        write_one_stage(tx, position as i64, stage)?;
    }
    Ok(())
}

/// Writes one stage and every table hanging off it, at `position` in the timeline.
///
/// The body of [`write_stages`]' loop, separated so the editor can write a single stage without
/// rewriting the timeline around it — which is the whole point of editing one stage at a time.
pub(super) fn write_one_stage(tx: &Transaction<'_>, position: i64, stage: &Stage) -> Result<()> {
    tx.execute(
        "INSERT INTO behaviour_stage (id, position, label, restricts_content, wallpaper, audio, audio_random)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET position = excluded.position,
                                       label = excluded.label,
                                       restricts_content = excluded.restricts_content,
                                       wallpaper = excluded.wallpaper,
                                       audio = excluded.audio,
                                       audio_random = excluded.audio_random",
        params![
            stage.id,
            position,
            stage.label,
            stage.content.tags.is_some(),
            stage.content.wallpaper,
            stage.content.audio,
            stage.content.audio_random,
        ],
    )?;
    write_stage_tags(
        tx,
        &stage.id,
        stage.content.tags.as_deref().unwrap_or(&[]),
        &stage.content.exclude,
    )?;
    write_owned_stage_tag(tx, &stage.id, false, stage.content.owned_tag.as_deref())?;
    write_owned_stage_tag(tx, &stage.id, true, stage.content.owned_exclude_tag.as_deref())?;
    write_stage_end(tx, &stage.id, stage.end.as_ref())?;
    write_stage_movement(tx, &stage.id, stage.movement.as_ref())?;
    write_stage_mitosis(tx, &stage.id, stage.mitosis.as_ref())?;
    write_stage_entry(
        tx,
        &stage.id,
        (!stage.on_enter.is_default()).then_some(&stage.on_enter),
    )?;
    write_stage_prompt(tx, &stage.id, &stage.prompt)?;
    write_stage_events(tx, &stage.id, &stage.events)?;
    Ok(())
}

pub(super) fn write_stage_entry(tx: &Transaction<'_>, stage: &str, entry: Option<&StageEntry>) -> Result<()> {
    let Some(entry) = entry else {
        tx.execute(
            "DELETE FROM behaviour_stage_entry WHERE stage_id = ?",
            params![stage],
        )?;
        return Ok(());
    };
    tx.execute(
        "INSERT INTO behaviour_stage_entry (stage_id, splash_media, sound_media, popup_burst, notification_text)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(stage_id) DO UPDATE SET splash_media = excluded.splash_media,
             sound_media = excluded.sound_media, popup_burst = excluded.popup_burst,
             notification_text = excluded.notification_text",
        params![
            stage,
            entry.splash,
            entry.sound,
            entry.popup_burst,
            entry.notification
        ],
    )?;
    Ok(())
}

pub(super) fn write_stage_prompt(tx: &Transaction<'_>, stage: &str, prompt: &StagePrompt) -> Result<()> {
    if prompt == &StagePrompt::default() {
        tx.execute(
            "DELETE FROM behaviour_stage_prompt WHERE stage_id = ?",
            params![stage],
        )?;
        return Ok(());
    }
    tx.execute(
        "INSERT INTO behaviour_stage_prompt
             (stage_id, timeouts_enabled, timeout_multiplier, popup_burst, sound_media)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(stage_id) DO UPDATE SET timeouts_enabled = excluded.timeouts_enabled,
             timeout_multiplier = excluded.timeout_multiplier,
             popup_burst = excluded.popup_burst, sound_media = excluded.sound_media",
        params![
            stage,
            prompt.timeouts_enabled,
            prompt.timeout_multiplier,
            prompt.popup_burst,
            prompt.sound
        ],
    )?;
    Ok(())
}

pub(super) fn write_stage_end(tx: &Transaction<'_>, stage: &str, end: Option<&StageEnd>) -> Result<()> {
    let Some(end) = end else {
        tx.execute(
            "DELETE FROM behaviour_stage_end WHERE stage_id = ?",
            params![stage],
        )?;
        return Ok(());
    };
    let (kind, count, scope) = match &end.event_count {
        Some(condition) => (
            Some(to_text(&condition.event)?),
            Some(condition.count),
            Some(to_text(&condition.scope)?),
        ),
        None => (None, None, None),
    };
    tx.execute(
        "INSERT INTO behaviour_stage_end
             (stage_id, duration_seconds, strategy, event_kind, event_count, event_scope)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(stage_id) DO UPDATE SET duration_seconds = excluded.duration_seconds,
                                             strategy = excluded.strategy,
                                             event_kind = excluded.event_kind,
                                             event_count = excluded.event_count,
                                             event_scope = excluded.event_scope",
        params![
            stage,
            end.duration_seconds,
            to_text(&end.strategy)?,
            kind,
            count,
            scope
        ],
    )?;
    Ok(())
}

pub(super) fn write_stage_movement(
    tx: &Transaction<'_>,
    stage: &str,
    movement: Option<&Movement>,
) -> Result<()> {
    let Some(movement) = movement else {
        tx.execute(
            "DELETE FROM behaviour_stage_movement WHERE stage_id = ?",
            params![stage],
        )?;
        return Ok(());
    };
    tx.execute(
        "INSERT INTO behaviour_stage_movement (stage_id, minimum_speed, maximum_speed)
         VALUES (?, ?, ?)
         ON CONFLICT(stage_id) DO UPDATE SET minimum_speed = excluded.minimum_speed,
                                             maximum_speed = excluded.maximum_speed",
        params![stage, movement.minimum_speed, movement.maximum_speed],
    )?;
    Ok(())
}

pub(super) fn write_stage_mitosis(tx: &Transaction<'_>, stage: &str, mitosis: Option<&Mitosis>) -> Result<()> {
    let Some(mitosis) = mitosis else {
        tx.execute(
            "DELETE FROM behaviour_stage_mitosis WHERE stage_id = ?",
            params![stage],
        )?;
        return Ok(());
    };
    tx.execute(
        "INSERT INTO behaviour_stage_mitosis (stage_id, chance, count) VALUES (?, ?, ?)
         ON CONFLICT(stage_id) DO UPDATE SET chance = excluded.chance, count = excluded.count",
        params![stage, mitosis.chance, mitosis.count],
    )?;
    Ok(())
}

pub(super) fn write_stage_events(tx: &Transaction<'_>, stage: &str, events: &Events) -> Result<()> {
    let scheduled = [
        ("popup", events.popup.as_ref()),
        ("web", events.web.as_ref()),
        ("notification", events.notification.as_ref()),
        ("prompt", events.prompt.as_ref()),
        ("sound", events.sound.as_ref()),
    ];
    for (kind, schedule) in scheduled {
        let Some(schedule) = schedule else {
            tx.execute(
                "DELETE FROM behaviour_stage_event WHERE stage_id = ? AND kind = ?",
                params![stage, kind],
            )?;
            continue;
        };
        let (interval_kind, seconds, minimum, maximum) = match schedule.interval {
            Interval::Fixed { seconds } => ("fixed", Some(seconds), None, None),
            Interval::Random {
                minimum_seconds,
                maximum_seconds,
            } => ("random", None, Some(minimum_seconds), Some(maximum_seconds)),
        };
        tx.execute(
            "INSERT INTO behaviour_stage_event
                 (stage_id, kind, interval_kind, seconds, minimum_seconds, maximum_seconds,
                  initial_delay_seconds, max_concurrent)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(stage_id, kind) DO UPDATE SET
                 interval_kind = excluded.interval_kind,
                 seconds = excluded.seconds,
                 minimum_seconds = excluded.minimum_seconds,
                 maximum_seconds = excluded.maximum_seconds,
                 initial_delay_seconds = excluded.initial_delay_seconds,
                 max_concurrent = excluded.max_concurrent",
            params![
                stage,
                kind,
                interval_kind,
                seconds,
                minimum,
                maximum,
                schedule.initial_delay_seconds,
                schedule.max_concurrent,
            ],
        )?;
    }
    Ok(())
}

pub(super) fn write_transitions(tx: &Transaction<'_>, transitions: &[Transition]) -> Result<()> {
    retain(
        tx,
        "behaviour_transition",
        "id",
        transitions.iter().map(|t| t.id.as_str()),
    )?;
    for (position, transition) in transitions.iter().enumerate() {
        tx.execute(
            "INSERT INTO behaviour_transition
                 (id, position, from_stage, to_stage, duration_seconds, easing)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET position = excluded.position,
                                           from_stage = excluded.from_stage,
                                           to_stage = excluded.to_stage,
                                           duration_seconds = excluded.duration_seconds,
                                           easing = excluded.easing",
            params![
                transition.id,
                position as i64,
                transition.from_stage,
                transition.to_stage,
                transition.duration_seconds,
                to_text(&transition.easing)?,
            ],
        )?;
        let categories = transition
            .affected
            .iter()
            .map(to_text)
            .collect::<Result<Vec<_>>>()?;
        tx.execute(
            "DELETE FROM behaviour_transition_category WHERE transition_id = ?1
             AND category NOT IN (SELECT value FROM json_each(?2))",
            params![transition.id, serde_json::to_string(&categories)?],
        )?;
        for (position, category) in categories.iter().enumerate() {
            tx.execute(
                "INSERT INTO behaviour_transition_category (transition_id, category, position)
                 VALUES (?, ?, ?)
                 ON CONFLICT(transition_id, category) DO UPDATE SET position = excluded.position",
                params![transition.id, category, position as i64],
            )?;
        }
    }
    Ok(())
}

/// [`retain`] for a numeric key column. Separate because the ids never exist as strings anywhere
/// else, and stringifying them just to satisfy one signature would invite the comparison to be
/// made against SQLite's text affinity by accident.
fn retain_ids(tx: &Transaction<'_>, table: &str, key: &str, keep: &[u64]) -> Result<()> {
    tx.execute(
        &format!("DELETE FROM {table} WHERE {key} NOT IN (SELECT value FROM json_each(?))"),
        params![serde_json::to_string(keep)?],
    )?;
    Ok(())
}

/// Deletes every row of `table` whose `key` column is not in `keep`.
///
/// The delete half of the diff. Passing the surviving keys as JSON rather than building an `IN
/// (?, ?, …)` keeps one prepared statement whatever the timeline's length.
pub(super) fn retain<'a>(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    keep: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let keep: Vec<&str> = keep.collect();
    tx.execute(
        &format!("DELETE FROM {table} WHERE {key} NOT IN (SELECT value FROM json_each(?))"),
        params![serde_json::to_string(&keep)?],
    )?;
    Ok(())
}

/// An enum's database text, which is deliberately the same string its JSON uses.
///
/// Routed through serde rather than a hand-written match so the two representations cannot drift:
/// renaming a variant's `rename_all` spelling changes both at once.
pub(super) fn to_text<T: Serialize>(value: &T) -> Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(text) => Ok(text),
        other => bail!("expected a string-valued enum, got {other}"),
    }
}

pub(super) fn from_text<T: DeserializeOwned>(text: &str) -> Result<T> {
    Ok(serde_json::from_value(serde_json::Value::String(
        text.to_string(),
    ))?)
}

#[cfg(test)]
mod tests;
