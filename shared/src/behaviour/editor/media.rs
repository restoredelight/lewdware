use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::behaviour::db::{read_one_popup_media, sql_array, write_popup_audio_pairs};
use crate::behaviour::schema::{
    AudioMedia, MediaSlot, MonitorPreference, PopupMedia, SlotColumn, SpawnRegion,
};

pub fn popup_attributes(conn: &Connection, ids: &[u64]) -> Result<Vec<(u64, PopupMedia)>> {
    let mut found = Vec::new();

    for id in ids {
        if let Some(entry) = read_one_popup_media(conn, *id)? {
            found.push((*id, entry));
        }
    }

    Ok(found)
}

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

pub fn set_popup_weight(tx: &Transaction<'_>, ids: &[u64], weight: Option<f64>) -> Result<()> {
    set_popup_column(tx, ids, "weight", weight)
}

pub fn set_popup_scale(tx: &Transaction<'_>, ids: &[u64], scale: Option<f64>) -> Result<()> {
    set_popup_column(tx, ids, "scale", scale)
}

/// The part of the monitor these files may spawn in.
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

pub fn set_popup_video_volume(
    tx: &Transaction<'_>,
    ids: &[u64],
    volume: Option<f64>,
) -> Result<()> {
    set_popup_column(tx, ids, "video_volume", volume)
}

pub fn set_popup_audio(tx: &Transaction<'_>, popup: u64, audio: &[u64]) -> Result<()> {
    write_popup_audio_pairs(tx, popup, audio)
}

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
            "SELECT wallpaper, splash FROM behaviour_content",
            [],
            |row| Ok((row.get("wallpaper")?, row.get("splash")?)),
        )
        .optional()?
        .unwrap_or((None, None));
    Ok(MediaSlots { wallpaper, splash })
}

pub fn slot_value(conn: &Connection, slot: &MediaSlot) -> Result<Option<u64>> {
    let SlotColumn {
        table,
        column,
        key,
        stage,
    } = slot.slot_column();
    let value: Option<Option<u64>> = match stage {
        None => conn
            .query_row(&format!("SELECT {column} FROM {table}"), [], |row| {
                row.get(0)
            })
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

/// Points `slot` at `media`
pub fn set_media_slot(tx: &Transaction<'_>, slot: &MediaSlot, media: Option<u64>) -> Result<()> {
    let SlotColumn {
        table,
        column,
        stage,
        ..
    } = slot.slot_column();

    let Some(stage) = stage else {
        tx.execute(&format!("UPDATE {table} SET {column} = ?"), params![media])?;
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

    tx.execute(
        &format!("UPDATE {table} SET {column} = ? WHERE id = ?"),
        params![media, stage],
    )?;

    // Naming a track and "pick one at random" are alternatives
    if matches!(slot, MediaSlot::StageAudio { .. }) && media.is_some() {
        tx.execute(
            "UPDATE behaviour_stage SET audio_random = FALSE WHERE id = ?",
            params![stage],
        )?;
    }

    Ok(())
}

/// Whether any media slot in the pack points at `media`.
pub fn is_media_referenced(conn: &Connection, media: u64) -> Result<bool> {
    let referenced: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM behaviour_content WHERE wallpaper = ?1 OR splash = ?1
             UNION ALL
             SELECT 1 FROM behaviour_stage WHERE wallpaper = ?1 OR audio = ?1
                 OR entry_splash_media = ?1 OR entry_sound_media = ?1 OR prompt_sound_media = ?1
         )",
        params![media],
        |row| row.get(0),
    )?;
    Ok(referenced)
}

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
            "UPDATE behaviour_stage SET entry_splash_media = NULL WHERE entry_splash_media = ?",
            params![media_id],
        )?;
        tx.execute(
            "UPDATE behaviour_stage SET entry_sound_media = NULL WHERE entry_sound_media = ?",
            params![media_id],
        )?;
        tx.execute(
            "UPDATE behaviour_stage SET prompt_sound_media = NULL WHERE prompt_sound_media = ?",
            params![media_id],
        )?;
    }
    Ok(())
}
