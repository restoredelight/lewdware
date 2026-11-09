//! Recorded edits for media, tags, artists and per-file attributes, as functions over a connection.
//! See `super` for the contract. Also the library's reads that the session's media index can't
//! answer: what the Tags and Artists views list, which slots a file fills, stored thumbnails.

#[allow(unused_imports)]
use super::*;

pub use crate::pack::labels::MediaUsageTarget;
use crate::pack::{ArtistSummary, MediaUsage, TagRow};
use shared::behaviour::editor as behaviour_editor;

// ── Files ───────────────────────────────────────────────────────────────────────────────────

/// Removes files from the pack: one undo entry that also empties any slot pointing at them, so
/// undo brings back the file and the reference together. The bytes stay until history lets go of
/// them, so this is quick even for a large selection.
pub fn remove_files(connection: &Connection, ids: &[u64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let label = if ids.len() == 1 {
        "Remove media item".to_string()
    } else {
        format!("Remove {} media items", ids.len())
    };
    history::record_media_visibility(connection, &label, ids, true, |tx| {
        behaviour_editor::clear_media_slots(tx, ids)?;
        Ok(())
    })
}

/// Renames one file. A name another file already has is refused with a message rather than
/// adjusted: the author typed it, and a silently different name would be a surprise.
pub fn set_file_name(connection: &Connection, id: u64, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("A file name can't be empty");
    }
    let result = history::record(connection, "Rename media item", 0, &[id], |tx| {
        tx.execute(
            "UPDATE media SET file_name = ? WHERE id = ?",
            params![name, id],
        )?;
        Ok(())
    });
    match result {
        Err(error) if violates_unique_constraint(&error, "file_name") => {
            bail!("A file named \"{name}\" already exists")
        }
        other => other,
    }
}

/// Moves audio between the mutually exclusive background (unmarked) and popup roles.
pub fn set_popup_audio(connection: &Connection, ids: &[u64], popup: bool) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let label = if popup {
        "Move audio to Popup"
    } else {
        "Move audio to Background"
    };
    history::record(connection, label, 0, ids, |tx| {
        for id in ids {
            let file_type: Option<String> = tx
                .query_row(
                    "SELECT file_type FROM media WHERE id = ? AND deleted = 0",
                    params![id],
                    |row| row.get(0),
                )
                .optional()?;
            if file_type.as_deref() != Some("audio") {
                bail!("Only audio files can be assigned an audio role");
            }
            if popup {
                labels::apply_tag(tx, *id, tags::POPUP_AUDIO_TAG)?;
            } else {
                tx.execute(
                    "DELETE FROM media_tags WHERE media_id = ? AND tag_id IN
                     (SELECT id FROM tags WHERE name = ?)",
                    params![id, tags::POPUP_AUDIO_TAG],
                )?;
            }
        }
        Ok(())
    })
}

// ── Reads ───────────────────────────────────────────────────────────────────────────────────

/// Every tag the author can edit, with how much of the pack uses it. Includes tags only the
/// behaviour document names (a caption's tag, a stage's), which the media index can't see.
pub fn tag_rows(connection: &Connection) -> Result<Vec<TagRow>> {
    let mut statement = connection.prepare_cached(
        "SELECT tags.name,
                COUNT(DISTINCT media.id),
                (SELECT COUNT(*) FROM behaviour_content_group_tag t WHERE t.tag_id = tags.id)
                  + (SELECT COUNT(*) FROM behaviour_content_entry_tag t WHERE t.tag_id = tags.id),
                (SELECT COUNT(*) FROM behaviour_stage_tag t WHERE t.tag_id = tags.id)
         FROM tags
         LEFT JOIN media_tags ON media_tags.tag_id = tags.id
         LEFT JOIN media ON media.id = media_tags.media_id AND media.deleted = FALSE
         GROUP BY tags.id ORDER BY tags.name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(TagRow {
            name: row.get(0)?,
            media_count: row.get(1)?,
            content_uses: row.get(2)?,
            experience_uses: row.get(3)?,
        })
    })?;
    Ok(rows
        .collect::<rusqlite::Result<Vec<TagRow>>>()?
        .into_iter()
        .filter(|row| !tags::is_managed(&row.name))
        .collect())
}

/// Every artist, with how many visible files credit them.
pub fn artist_rows(connection: &Connection) -> Result<Vec<ArtistSummary>> {
    let mut statement = connection.prepare_cached(
        "SELECT artists.name, COUNT(media.id) FROM artists
         LEFT JOIN media_artists ON media_artists.artist_id = artists.id
         LEFT JOIN media ON media.id = media_artists.media_id AND media.deleted = FALSE
         GROUP BY artists.id ORDER BY artists.name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(ArtistSummary {
            name: row.get(0)?,
            media_count: row.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every media slot pointing at `media`, as an author would name it: what removing the file would
/// also clear, and the inspector's "Used as".
pub fn media_usage(connection: &Connection, media: u64) -> Result<Vec<MediaUsage>> {
    let mut usage = Vec::new();
    let (wallpaper, splash): (Option<u64>, Option<u64>) = connection
        .query_row(
            "SELECT wallpaper, splash FROM behaviour_content",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .unwrap_or((None, None));
    if wallpaper == Some(media) {
        usage.push(MediaUsage {
            label: "Wallpaper".to_string(),
            target: MediaUsageTarget::Wallpaper,
        });
    }
    if splash == Some(media) {
        usage.push(MediaUsage {
            label: "Splash".to_string(),
            target: MediaUsageTarget::Splash,
        });
    }
    let mut statement = connection.prepare_cached(
        "SELECT id, label FROM behaviour_stage WHERE wallpaper = ? ORDER BY position",
    )?;
    let stages: Vec<(String, String)> = statement
        .query_map(params![media], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, label) in stages {
        usage.push(MediaUsage {
            label: format!("Wallpaper for \"{label}\""),
            target: MediaUsageTarget::Stage { id },
        });
    }
    Ok(usage)
}

/// The volume a file has been given, or `None` at full ("no opinion").
pub fn audio_volume(connection: &Connection, id: u64) -> Result<Option<f64>> {
    Ok(behaviour_editor::audio_attributes(connection, &[id])?
        .into_iter()
        .next()
        .and_then(|(_, attributes)| attributes.volume))
}

/// The thumbnails stored for `ids`, as encoded at import (a small WebP). Files without one, and
/// ids that don't exist, are left out.
pub fn thumbnails(connection: &Connection, ids: &[u64]) -> Result<Vec<(u64, Vec<u8>)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let selected = media_id_array(ids)?;
    let mut statement = connection.prepare_cached(
        "SELECT id, thumbnail FROM media
         WHERE id IN (SELECT value FROM rarray(?)) AND thumbnail IS NOT NULL",
    )?;
    let rows = statement.query_map(params![selected], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
