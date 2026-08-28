use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::behaviour::db::{read_one_popup_media, sql_array, write_popup_audio_pairs};
use crate::behaviour::schema::{AudioMedia, MediaSlot, MonitorPreference, PopupMedia, SpawnRegion};

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
        if let Some(entry) = read_one_popup_media(conn, *id)? {
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

/// Updates one nullable popup field across an inspector selection.
///
/// `behaviour_popup_media` has a row for every media item. Its update trigger materializes the
/// sparse backing row when the first field is set and removes it when the last field is cleared,
/// so callers only have to name the field the author actually changed.
fn set_popup_column<T: rusqlite::ToSql>(
    tx: &Transaction<'_>,
    ids: &[u64],
    column: &str,
    value: T,
) -> Result<()> {
    tx.execute(
        &format!(
            "UPDATE behaviour_popup_media SET {column} = ?2
             WHERE media_id IN (SELECT value FROM rarray(?1))"
        ),
        params![sql_array(ids)?, value],
    )?;
    Ok(())
}

/// How often these files are drawn relative to their neighbours.
pub fn set_popup_weight(tx: &Transaction<'_>, ids: &[u64], weight: Option<f64>) -> Result<()> {
    set_popup_column(tx, ids, "weight", weight)
}

/// Multiplies the size the mode would otherwise have chosen.
pub fn set_popup_scale(tx: &Transaction<'_>, ids: &[u64], scale: Option<f64>) -> Result<()> {
    set_popup_column(tx, ids, "scale", scale)
}

/// The part of the monitor these files may spawn in. One field, not four: a region is a rectangle,
/// and half of one is not a placement anybody asked for.
pub fn set_popup_region(
    tx: &Transaction<'_>,
    ids: &[u64],
    region: Option<SpawnRegion>,
) -> Result<()> {
    let region = region.map(SpawnRegion::sanitized);
    tx.execute(
        "UPDATE behaviour_popup_media
         SET region_x = ?2, region_y = ?3, region_width = ?4, region_height = ?5
         WHERE media_id IN (SELECT value FROM rarray(?1))",
        params![
            sql_array(ids)?,
            region.map(|region| region.x),
            region.map(|region| region.y),
            region.map(|region| region.width),
            region.map(|region| region.height),
        ],
    )?;
    Ok(())
}

pub fn set_popup_monitor(
    tx: &Transaction<'_>,
    ids: &[u64],
    monitor: Option<MonitorPreference>,
) -> Result<()> {
    set_popup_column(tx, ids, "monitor", monitor)
}

/// A caption belonging to these files, as opposed to the tag-matched pool. An empty one is the
/// absence of a caption rather than a caption of "".
pub fn set_popup_caption(tx: &Transaction<'_>, ids: &[u64], caption: Option<&str>) -> Result<()> {
    let caption = caption.filter(|text| !text.is_empty());
    set_popup_column(tx, ids, "caption", caption)
}

pub fn set_popup_video_loop(tx: &Transaction<'_>, ids: &[u64], value: Option<bool>) -> Result<()> {
    set_popup_column(tx, ids, "video_loop", value)
}

pub fn set_popup_video_audio(tx: &Transaction<'_>, ids: &[u64], value: Option<bool>) -> Result<()> {
    set_popup_column(tx, ids, "video_audio", value)
}

/// This clip's soundtrack's own level. The audio-file counterpart is [`set_audio_volume`].
pub fn set_popup_video_volume(
    tx: &Transaction<'_>,
    ids: &[u64],
    volume: Option<f64>,
) -> Result<()> {
    set_popup_column(tx, ids, "video_volume", volume)
}

/// Replaces one popup's explicit set of sound pairings.
pub fn set_popup_audio(tx: &Transaction<'_>, popup: u64, audio: &[u64]) -> Result<()> {
    write_popup_audio_pairs(tx, popup, audio)
}

/// This track's own level, for levelling a pack assembled from mixed sources.
pub fn set_audio_volume(tx: &Transaction<'_>, ids: &[u64], volume: Option<f64>) -> Result<()> {
    for id in ids {
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
    Ok(())
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

/// Points `slot` at `media`, or empties it with `None`.
///
/// Errors if the slot's stage has been removed — a slot on a stage that is gone is a stale editor,
/// and creating the row would resurrect part of what the removal took away.
pub fn set_media_slot(tx: &Transaction<'_>, slot: &MediaSlot, media: Option<u64>) -> Result<()> {
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
        return Ok(());
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
                    "UPDATE behaviour_stage SET audio_random = FALSE WHERE id = ?",
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
    Ok(())
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

/// Empties every behaviour slot that points at one of `media_ids`.
///
/// Media visibility is changed separately from the behaviour tables, so SQLite's foreign keys do
/// not clear these nullable references for us. Keeping this as targeted updates means removing a
/// file records only the slots it actually displaced in editor history.
pub fn clear_media_slots(tx: &Transaction<'_>, media_ids: &[u64]) -> Result<()> {
    for media_id in media_ids {
        tx.execute(
            "UPDATE behaviour_content SET wallpaper = NULL WHERE wallpaper = ?",
            params![media_id],
        )?;
        tx.execute(
            "UPDATE behaviour_content SET splash = NULL WHERE splash = ?",
            params![media_id],
        )?;
        tx.execute(
            "UPDATE behaviour_stage SET wallpaper = NULL WHERE wallpaper = ?",
            params![media_id],
        )?;
        tx.execute(
            "UPDATE behaviour_stage SET audio = NULL WHERE audio = ?",
            params![media_id],
        )?;
        tx.execute(
            "UPDATE behaviour_stage_entry SET splash_media = NULL WHERE splash_media = ?",
            params![media_id],
        )?;
        tx.execute(
            "UPDATE behaviour_stage_entry SET sound_media = NULL WHERE sound_media = ?",
            params![media_id],
        )?;
        tx.execute(
            "UPDATE behaviour_stage_prompt SET sound_media = NULL WHERE sound_media = ?",
            params![media_id],
        )?;
    }
    Ok(())
}
