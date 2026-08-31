use std::collections::BTreeMap;

use anyhow::{Result, bail};
use rusqlite::{Connection, Transaction, named_params, params};

use crate::behaviour::{
    db::common::sql_array,
    schema::{AudioMedia, PopupMedia, SpawnRegion},
};

pub fn read_popup_media(conn: &Connection) -> Result<BTreeMap<u64, PopupMedia>> {
    read_popup_media_where(conn, "", &[])
}

pub fn read_one_popup_media(conn: &Connection, id: u64) -> Result<Option<PopupMedia>> {
    Ok(read_popup_media_where(conn, "AND popup.media_id = ?", &[&id])?.remove(&id))
}

fn read_popup_media_where(
    conn: &Connection,
    filter: &str,
    binds: &[&dyn rusqlite::ToSql],
) -> Result<BTreeMap<u64, PopupMedia>> {
    let mut statement = conn.prepare(&format!(
        "SELECT popup.media_id, popup.weight, popup.scale, popup.region_x, popup.region_y,
                popup.region_width, popup.region_height, popup.monitor, popup.caption,
                popup.video_loop, popup.video_audio, popup.video_volume
         FROM behaviour_popup_media AS popup
         WHERE (popup.weight IS NOT NULL
             OR popup.scale IS NOT NULL
             OR popup.region_x IS NOT NULL
             OR popup.region_y IS NOT NULL
             OR popup.region_width IS NOT NULL
             OR popup.region_height IS NOT NULL
             OR popup.monitor IS NOT NULL
             OR popup.caption IS NOT NULL
             OR popup.video_loop IS NOT NULL
             OR popup.video_audio IS NOT NULL
             OR popup.video_volume IS NOT NULL
             OR EXISTS (
                 SELECT 1 FROM behaviour_popup_audio_pair AS pair
                 WHERE pair.popup_media_id = popup.media_id
             ))
         {filter}
         ORDER BY popup.media_id"
    ))?;

    statement
        .query_and_then(binds, |row| {
            let media_id: u64 = row.get("media_id")?;
            let region = (
                row.get("region_x")?,
                row.get("region_y")?,
                row.get("region_width")?,
                row.get("region_height")?,
            );
            Ok((
                media_id,
                PopupMedia {
                    weight: row.get("weight")?,
                    scale: row.get("scale")?,
                    region: read_region(region),
                    monitor: row.get("monitor")?,
                    caption: row.get("caption")?,
                    video_loop: row.get("video_loop")?,
                    video_audio: row.get("video_audio")?,
                    video_volume: row.get("video_volume")?,
                    audio: read_popup_audio_pairs(conn, media_id)?,
                },
            ))
        })?
        .collect()
}

fn read_region(
    columns: (Option<f64>, Option<f64>, Option<f64>, Option<f64>),
) -> Option<SpawnRegion> {
    if let (Some(x), Some(y), Some(width), Some(height)) = columns {
        Some(
            SpawnRegion {
                x,
                y,
                width,
                height,
            }
            .sanitized(),
        )
    } else {
        None
    }
}

fn read_popup_audio_pairs(conn: &Connection, popup: u64) -> Result<Vec<u64>> {
    let mut statement = conn.prepare(
        "SELECT audio_media_id FROM behaviour_popup_audio_pair
         WHERE popup_media_id = :popup ORDER BY audio_media_id",
    )?;

    Ok(statement
        .query_map(named_params![":popup": popup], |row| {
            row.get("audio_media_id")
        })?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn write_popup_media(tx: &Transaction<'_>, popups: &BTreeMap<u64, PopupMedia>) -> Result<()> {
    for (media_id, entry) in popups {
        write_one_popup_media(tx, *media_id, entry)?;
    }

    Ok(())
}

/// Writes one file's popup attributes and its explicit sound pairings.
pub fn write_one_popup_media(
    tx: &Transaction<'_>,
    media_id: u64,
    entry: &PopupMedia,
) -> Result<()> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM media WHERE id = ?)",
        [media_id],
        |row| row.get(0),
    )?;

    if !exists {
        bail!("popup media {media_id} does not exist");
    }

    let region = entry.region.map(SpawnRegion::sanitized);
    let monitor = entry.monitor.as_ref();
    tx.execute(
        "UPDATE behaviour_popup_media
         SET weight = ?2,
             scale = ?3,
             region_x = ?4,
             region_y = ?5,
             region_width = ?6,
             region_height = ?7,
             monitor = ?8,
             caption = ?9,
             video_loop = ?10,
             video_audio = ?11,
             video_volume = ?12
         WHERE media_id = ?1",
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
            entry.video_volume,
        ],
    )?;

    write_popup_audio_pairs(tx, media_id, &entry.audio)?;

    Ok(())
}

pub(crate) fn write_popup_audio_pairs(
    tx: &Transaction<'_>,
    popup: u64,
    audio: &[u64],
) -> Result<()> {
    tx.execute(
        "DELETE FROM behaviour_popup_audio_pair
         WHERE popup_media_id = ?1
           AND audio_media_id NOT IN (SELECT value FROM rarray(?2))",
        params![popup, sql_array(audio)?],
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

pub fn read_audio_media(conn: &Connection) -> Result<BTreeMap<u64, AudioMedia>> {
    let mut statement =
        conn.prepare("SELECT media_id, volume FROM behaviour_audio_media ORDER BY media_id")?;

    Ok(statement
        .query_map([], |row| {
            Ok((
                row.get("media_id")?,
                AudioMedia {
                    volume: row.get("volume")?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn write_audio_media(tx: &Transaction<'_>, audio: &BTreeMap<u64, AudioMedia>) -> Result<()> {
    for (media_id, entry) in audio {
        let Some(volume) = entry.volume else {
            continue;
        };

        tx.execute(
            "INSERT INTO behaviour_audio_media (media_id, volume)
             VALUES (?, ?)
             ON CONFLICT(media_id) DO UPDATE SET volume = excluded.volume",
            params![media_id, volume],
        )?;
    }

    Ok(())
}
