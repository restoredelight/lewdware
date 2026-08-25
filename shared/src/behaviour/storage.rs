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

mod common;
mod content;
mod experience;
mod media;
mod stage;
mod tags;
mod transition;

use common::retain_ids;
pub(super) use common::{from_text, retain, to_text};
pub(super) use content::{read_content_groups, read_text_pool, read_web_links};
use content::{write_content_groups, write_text_pool, write_web_links};
pub use experience::{TimelineView, read_timeline, set_experience_enabled, set_experience_label};
use experience::{read_experience, write_experience};
use media::{read_audio_media, read_popup_media, write_audio_media, write_popup_media};
pub(super) use media::{read_one_popup_media, write_one_popup_media};
use stage::write_stages;
#[allow(unused_imports)]
pub(super) use stage::{
    read_one_stage, read_stage_end, read_stage_entry, read_stage_events, read_stage_mitosis,
    read_stage_movement, read_stage_prompt, read_stage_tags, read_stages, write_one_stage,
    write_owned_stage_tag, write_stage_end, write_stage_entry, write_stage_events,
    write_stage_mitosis, write_stage_movement, write_stage_prompt, write_stage_tags,
};
pub(super) use tags::{read_tags, tag_id, write_tags};
pub(super) use transition::{read_transitions, write_transitions};

#[cfg(test)]
mod tests;
