//! The pack's recorded edits as plain functions over a connection: one transaction and one undo
//! entry each. The async `MediaPack` methods run them on a pooled connection off the calling
//! thread; `crate::session::Session` runs them synchronously on its own.
//!
//! Neither marks the pack unsaved: that's the caller's job once the write has landed.

// One module per track of the gpui port, called as `ops::<track>::<edit>`.
pub mod content;
pub mod library;
pub mod lifecycle;
pub mod timeline;

use super::*;

use super::labels::{artist_media_ids, tag_media_ids};
use crate::history;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub fn add_tag_to_files(connection: &Connection, ids: &[u64], tag: &str) -> Result<()> {
    reject_managed_tag(tag)?;
    if ids.is_empty() {
        return Ok(());
    }
    history::record(connection, "Add tag to media", 0, ids, |tx| {
        tx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?)", params![tag])?;
        let tag_id: u64 =
            tx.query_row("SELECT id FROM tags WHERE name = ?", params![tag], |row| {
                row.get("id")
            })?;
        for id in ids {
            tx.execute(
                "INSERT OR IGNORE INTO media_tags (media_id, tag_id) VALUES (?, ?)",
                params![id, tag_id],
            )?;
        }
        Ok(())
    })
}

pub fn remove_tag_from_files(connection: &Connection, ids: &[u64], tag: &str) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    history::record(connection, "Remove tag from media", 0, ids, |tx| {
        for id in ids {
            tx.execute("DELETE FROM media_tags WHERE media_id = ? AND tag_id IN (SELECT id FROM tags WHERE name = ?)", params![id, tag])?;
        }
        Ok(())
    })
}

pub fn rename_tag(conn: &Connection, from: &str, to: &str) -> Result<()> {
    reject_managed_tag(from)?;
    reject_managed_tag(to)?;
    history::record_with_media_refs(conn, "Rename tag", 0, |tx| {
        let media_ids = tag_media_ids(tx, from)?;
        let target_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tags WHERE name = ?)",
            params![to],
            |row| row.get(0),
        )?;
        if target_exists {
            bail!("A tag named \"{to}\" already exists. Merge the tags instead.");
        }
        tx.execute("UPDATE tags SET name = ? WHERE name = ?", params![to, from])?;
        Ok(((), media_ids))
    })
    .map(|_| ())
}

pub fn merge_tag(conn: &Connection, from: &str, to: &str) -> Result<()> {
    reject_managed_tag(from)?;
    reject_managed_tag(to)?;
    history::record_with_media_refs(conn, "Merge tags", 0, |tx| {
        let media_ids = tag_media_ids(tx, from)?;
        tx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?)", params![to])?;
        let source: Option<u64> = tx
            .query_row("SELECT id FROM tags WHERE name = ?", params![from], |row| {
                row.get(0)
            })
            .optional()?;
        let Some(source) = source else {
            return Ok(((), media_ids));
        };
        let target: u64 =
            tx.query_row("SELECT id FROM tags WHERE name = ?", params![to], |row| {
                row.get(0)
            })?;
        for (table, key) in TAG_JOIN_TABLES {
            tx.execute(
                &format!("UPDATE OR IGNORE {table} SET tag_id = ?1 WHERE tag_id = ?2"),
                params![target, source],
            )?;
            let _ = key;
        }
        // Whatever the re-point skipped as a duplicate, plus the tag itself. Dropping
        // the row would cascade the survivors away, so it goes last.
        tx.execute(
            "UPDATE OR IGNORE media_tags SET tag_id = ?1 WHERE tag_id = ?2",
            params![target, source],
        )?;
        tx.execute("DELETE FROM tags WHERE id = ?", params![source])?;
        Ok(((), media_ids))
    })
    .map(|_| ())
}

pub fn delete_tag(conn: &Connection, tag: &str) -> Result<()> {
    history::record_with_media_refs(conn, "Delete tag", 0, |tx| {
        let media_ids = tag_media_ids(tx, tag)?;
        tx.execute("DELETE FROM tags WHERE name = ?", params![tag])?;
        Ok(((), media_ids))
    })
    .map(|_| ())
}

pub fn add_artist_to_files(conn: &Connection, ids: &[u64], artist: &str) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    history::record(conn, "Add artist to media", 0, ids, |tx| {
        tx.execute(
            "INSERT OR IGNORE INTO artists (name) VALUES (?)",
            params![artist],
        )?;
        let artist_id: u64 = tx.query_row(
            "SELECT id FROM artists WHERE name = ?",
            params![artist],
            |row| row.get("id"),
        )?;
        for id in ids {
            tx.execute(
                "INSERT OR IGNORE INTO media_artists (media_id, artist_id) VALUES (?, ?)",
                params![id, artist_id],
            )?;
        }
        Ok(())
    })
}

pub fn remove_artist_from_files(conn: &Connection, ids: &[u64], artist: &str) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    history::record(conn, "Remove artist from media", 0, ids, |tx| {
        for id in ids {
            tx.execute("DELETE FROM media_artists WHERE media_id = ? AND artist_id IN (SELECT id FROM artists WHERE name = ?)", params![id, artist])?;
        }
        Ok(())
    })
}

pub fn rename_artist(conn: &Connection, from: &str, to: &str) -> Result<()> {
    history::record_with_media_refs(conn, "Rename artist", 0, |tx| {
        let media_ids = artist_media_ids(tx, from)?;
        let target_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM artists WHERE name = ?)",
            params![to],
            |row| row.get(0),
        )?;
        if target_exists {
            bail!("An artist named \"{to}\" already exists. Merge the artists instead.");
        }
        let changed = tx.execute(
            "UPDATE artists SET name = ? WHERE name = ?",
            params![to, from],
        )?;
        if changed == 0 {
            tx.execute("INSERT INTO artists (name) VALUES (?)", params![to])?;
        }
        Ok(((), media_ids))
    })
    .map(|_| ())
}

pub fn merge_artist(conn: &Connection, from: &str, to: &str) -> Result<()> {
    history::record_with_media_refs(conn, "Merge artists", 0, |tx| {
        let media_ids = artist_media_ids(tx, from)?;
        tx.execute("INSERT OR IGNORE INTO artists (name) VALUES (?)", params![to])?;
        tx.execute("INSERT OR IGNORE INTO media_artists (media_id, artist_id) SELECT media_artists.media_id, target.id FROM media_artists JOIN artists source ON source.id = media_artists.artist_id JOIN artists target ON target.name = ? WHERE source.name = ?", params![to, from])?;
        tx.execute("DELETE FROM media_artists WHERE artist_id IN (SELECT id FROM artists WHERE name = ?)", params![from])?;
        tx.execute("DELETE FROM artists WHERE name = ?", params![from])?;
        Ok(((), media_ids))
    })
    .map(|_| ())
}

pub fn delete_artist(conn: &Connection, artist: &str) -> Result<()> {
    history::record_with_media_refs(conn, "Delete artist", 0, |tx| {
        let media_ids = artist_media_ids(tx, artist)?;
        tx.execute(
            "DELETE FROM media_artists WHERE artist_id IN (SELECT id FROM artists WHERE name = ?)",
            params![artist],
        )?;
        tx.execute("DELETE FROM artists WHERE name = ?", params![artist])?;
        Ok(((), media_ids))
    })
    .map(|_| ())
}

pub fn set_source_url(connection: &Connection, id: u64, url: Option<&str>) -> Result<()> {
    history::record(connection, "Set media source", 0, &[id], |tx| {
        tx.execute(
            "UPDATE media SET source_url = ? WHERE id = ?",
            params![url, id],
        )?;
        Ok(())
    })
}
