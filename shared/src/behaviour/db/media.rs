use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{Connection, Transaction, params};

use super::common::id_array;
use crate::behaviour::schema::{AudioMedia, MonitorPreference, PopupMedia, SpawnRegion};

// ── Per-item media attributes ────────────────────────────────────────────────
//
// Keyed by media id rather than by position, which is what lets an edit address one file's
// attributes directly and lets `ON DELETE CASCADE` remove them with their media row. Slots are
// cleared separately when the editor soft-deletes media. An entry that says nothing is not stored
// (`PopupMedia::is_empty`), so
// clearing the last attribute removes the row rather than leaving a row of NULLs behind.

pub fn read_popup_media(conn: &Connection) -> Result<BTreeMap<u64, PopupMedia>> {
    read_popup_media_where(conn, "", &[])
}

/// One file's popup attributes, or `None` if the author has said nothing about it.
///
/// The editor asks about one file (or a selection) far more often than about the whole pack, and
/// answering that by reading every entry — plus a pairings query each — is the O(document) cost
/// this module exists to stop paying.
pub fn read_one_popup_media(conn: &Connection, id: u64) -> Result<Option<PopupMedia>> {
    Ok(read_popup_media_where(conn, "WHERE media_id = ?", &[&id])?.remove(&id))
}

fn read_popup_media_where(
    conn: &Connection,
    filter: &str,
    binds: &[&dyn rusqlite::ToSql],
) -> Result<BTreeMap<u64, PopupMedia>> {
    let mut statement = conn.prepare(&format!(
        "SELECT media_id, weight, scale, region_x, region_y, region_width, region_height,
                monitor, caption, video_loop, video_audio, video_volume
         FROM behaviour_popup_media {filter} ORDER BY media_id"
    ))?;
    type Row = (
        u64,
        Option<f64>,
        Option<f64>,
        (Option<f64>, Option<f64>, Option<f64>, Option<f64>),
        Option<MonitorPreference>,
        Option<String>,
        Option<bool>,
        Option<bool>,
        Option<f64>,
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
                row.get(11)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(
            |(
                media_id,
                weight,
                scale,
                region,
                monitor,
                caption,
                video_loop,
                video_audio,
                video_volume,
            )| {
                Ok((
                    media_id,
                    PopupMedia {
                        weight,
                        scale,
                        region: read_region(region),
                        monitor,
                        caption,
                        video_loop,
                        video_audio,
                        video_volume,
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

pub fn write_popup_media(tx: &Transaction<'_>, popups: &BTreeMap<u64, PopupMedia>) -> Result<()> {
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
pub fn write_one_popup_media(
    tx: &Transaction<'_>,
    media_id: u64,
    entry: &PopupMedia,
) -> Result<()> {
    let region = entry.region.map(SpawnRegion::sanitized);
    let monitor = entry.monitor.as_ref();
    tx.execute(
        "INSERT INTO behaviour_popup_media
             (media_id, weight, scale, region_x, region_y, region_width, region_height,
              monitor, caption, video_loop, video_audio, video_volume)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(media_id) DO UPDATE SET weight = excluded.weight,
                                             scale = excluded.scale,
                                             region_x = excluded.region_x,
                                             region_y = excluded.region_y,
                                             region_width = excluded.region_width,
                                             region_height = excluded.region_height,
                                             monitor = excluded.monitor,
                                             caption = excluded.caption,
                                             video_loop = excluded.video_loop,
                                             video_audio = excluded.video_audio,
                                             video_volume = excluded.video_volume",
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

fn write_popup_audio_pairs(tx: &Transaction<'_>, popup: u64, audio: &[u64]) -> Result<()> {
    tx.execute(
        "DELETE FROM behaviour_popup_audio_pair
         WHERE popup_media_id = ?1
           AND audio_media_id NOT IN (SELECT value FROM rarray(?2))",
        params![popup, id_array(audio)?],
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
                row.get(0)?,
                AudioMedia {
                    volume: row.get(1)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn write_audio_media(tx: &Transaction<'_>, audio: &BTreeMap<u64, AudioMedia>) -> Result<()> {
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
