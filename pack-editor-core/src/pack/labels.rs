//! Tags and artists: the associations themselves, and keeping the behaviour document's
//! mentions of a tag in step when one is renamed, merged, or deleted.

use super::*;

use crate::history;
use anyhow::Result;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use shared::{behaviour::Behaviour, tags};

/// The stored behaviour document, assembled from the `behaviour_*` tables. The mapping is entirely
/// [`Behaviour`]'s business, and everything here goes on working with whole values.
pub(super) fn read_behaviour(connection: &rusqlite::Connection) -> Result<Behaviour> {
    Behaviour::read(connection)
}

pub(super) fn tag_media_ids(tx: &rusqlite::Transaction<'_>, tag: &str) -> Result<Vec<u64>> {
    let mut statement = tx.prepare(
        "SELECT mt.media_id FROM media_tags mt
         JOIN tags t ON t.id = mt.tag_id WHERE t.name = ?",
    )?;
    let ids = statement
        .query_map([tag], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

/// What a group of [`TagAction`]s did, accumulated across both phases of one edit.
#[derive(Default)]
pub(super) struct TagActionOutcome {
    /// Whether anything actually happened. A behaviour patch that lands on the value already there
    /// is not an edit, and neither is a rename that collided or a retirement that was claimed --
    /// but a tag action that *did* something makes the edit real even when the document is
    /// unchanged, which is exactly the case a per-file "Appears in" toggle produces.
    pub(super) changed: bool,
    pub(super) media_refs: Vec<u64>,
    /// Per-file tag changes, for [`BehaviourOutcome::media_tags`].
    pub(super) media_tags: Vec<(u64, String, bool)>,
    pub(super) removed: Vec<String>,
    pub(super) renamed: Vec<(String, String)>,
}

/// The media membership edits and renames, which run before the patched document is written. See
/// [`TagAction`] for why the ordering is by phase.
pub(super) fn run_tag_actions_before_write(
    tx: &rusqlite::Transaction<'_>,
    actions: &[TagAction],
    outcome: &mut TagActionOutcome,
) -> Result<()> {
    for action in actions {
        match action {
            TagAction::Apply { tag, media } => {
                reject_managed_tag(tag)?;
                outcome.changed |=
                    tx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?)", params![tag])? > 0;
                let tag_id: u64 =
                    tx.query_row("SELECT id FROM tags WHERE name = ?", params![tag], |row| {
                        row.get(0)
                    })?;
                match media {
                    Some(ids) => {
                        for id in ids {
                            outcome.changed |= tx.execute(
                                "INSERT OR IGNORE INTO media_tags (media_id, tag_id) VALUES (?, ?)",
                                params![id, tag_id],
                            )? > 0;
                        }
                        outcome.media_refs.extend(ids.iter().copied());
                        outcome
                            .media_tags
                            .extend(ids.iter().map(|id| (*id, tag.clone(), true)));
                    }
                    None => {
                        outcome.changed |= tx.execute(
                            "INSERT OR IGNORE INTO media_tags (media_id, tag_id)
                             SELECT id, ? FROM media WHERE deleted = FALSE",
                            params![tag_id],
                        )? > 0;
                        let live = live_media_ids(tx)?;
                        outcome
                            .media_tags
                            .extend(live.iter().map(|id| (*id, tag.clone(), true)));
                        outcome.media_refs.extend(live);
                    }
                }
            }
            TagAction::Remove { tag, media } => {
                reject_managed_tag(tag)?;
                let Some(tag_id) = tx
                    .query_row("SELECT id FROM tags WHERE name = ?", params![tag], |row| {
                        row.get::<_, u64>(0)
                    })
                    .optional()?
                else {
                    continue;
                };
                for id in media {
                    outcome.changed |= tx.execute(
                        "DELETE FROM media_tags WHERE media_id = ? AND tag_id = ?",
                        params![id, tag_id],
                    )? > 0;
                }
                outcome.media_refs.extend(media.iter().copied());
                outcome
                    .media_tags
                    .extend(media.iter().map(|id| (*id, tag.clone(), false)));
            }
            TagAction::Rename { from, to } => {
                reject_managed_tag(from)?;
                reject_managed_tag(to)?;
                let taken: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM tags WHERE name = ?)",
                    params![to],
                    |row| row.get(0),
                )?;
                if taken {
                    continue;
                }
                let refs = tag_media_ids(tx, from)?;
                if tx.execute("UPDATE tags SET name = ? WHERE name = ?", params![to, from])? > 0 {
                    outcome.changed = true;
                    outcome.media_refs.extend(refs);
                    outcome.renamed.push((from.clone(), to.clone()));
                }
            }
            TagAction::RetireIfUnclaimed { .. } | TagAction::Delete { .. } => {}
        }
    }
    Ok(())
}

/// [`TagAction::RetireIfUnclaimed`] and [`TagAction::Delete`], which run once the patched document
/// is stored -- "claimed" being a question about the document the edit produced.
pub(super) fn run_tag_actions_after_write(
    tx: &rusqlite::Transaction<'_>,
    actions: &[TagAction],
    outcome: &mut TagActionOutcome,
) -> Result<()> {
    for action in actions {
        let tag = match action {
            TagAction::RetireIfUnclaimed { tag } if !tag_is_claimed(tx, tag)? => tag,
            TagAction::Delete { tag } => tag,
            _ => continue,
        };
        let refs = tag_media_ids(tx, tag)?;
        if tx.execute("DELETE FROM tags WHERE name = ?", params![tag])? > 0 {
            outcome.changed = true;
            outcome.media_refs.extend(refs);
            outcome.removed.push(tag.clone());
        }
    }
    Ok(())
}

/// Whether anything in the pack still holds `tag`: media that carries it, or any of the behaviour
/// document's tag lists.
///
/// Soft-deleted media does not count, so that the Tags tab's file count and this agree -- a stage
/// tag left only on files the author already removed is machinery, not work. Undo covers the
/// asymmetry: restoring those files and restoring the tag are the same two entries, in order.
pub(super) fn tag_is_claimed(tx: &rusqlite::Transaction<'_>, tag: &str) -> Result<bool> {
    let on_media: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM media_tags mt
             JOIN tags t ON t.id = mt.tag_id
             JOIN media m ON m.id = mt.media_id
             WHERE t.name = ? AND m.deleted = FALSE)",
        params![tag],
        |row| row.get(0),
    )?;
    if on_media {
        return Ok(true);
    }
    for (table, _) in TAG_JOIN_TABLES {
        let named: bool = tx.query_row(
            &format!(
                "SELECT EXISTS(
                     SELECT 1 FROM {table} j JOIN tags t ON t.id = j.tag_id WHERE t.name = ?)"
            ),
            params![tag],
            |row| row.get(0),
        )?;
        if named {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn live_media_ids(tx: &rusqlite::Transaction<'_>) -> Result<Vec<u64>> {
    let mut statement = tx.prepare("SELECT id FROM media WHERE deleted = FALSE")?;
    let ids = statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

pub(super) fn artist_media_ids(tx: &rusqlite::Transaction<'_>, artist: &str) -> Result<Vec<u64>> {
    let mut statement = tx.prepare(
        "SELECT ma.media_id FROM media_artists ma
         JOIN artists a ON a.id = ma.artist_id WHERE a.name = ?",
    )?;
    let ids = statement
        .query_map([artist], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

impl MediaPack {
    pub async fn get_tag_summaries(&self) -> Result<Vec<TagSummary>> {
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT tags.name, COUNT(media.id) FROM tags
                 LEFT JOIN media_tags ON media_tags.tag_id = tags.id
                 LEFT JOIN media ON media.id = media_tags.media_id AND media.deleted = FALSE
                 GROUP BY tags.id ORDER BY tags.name COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(TagSummary {
                    name: row.get(0)?,
                    media_count: row.get(1)?,
                })
            })?;
            // Same filter as `get_all_tags`: the Tags tab never lists a tag the author can't edit.
            Ok(rows
                .collect::<rusqlite::Result<Vec<TagSummary>>>()?
                .into_iter()
                .filter(|summary| !tags::is_managed(&summary.name))
                .collect())
        })
        .await
    }

    /// Every tag the pack knows about, with how much of it uses each.
    ///
    /// One query rather than the three-way merge the Tags tab used to do in TypeScript — over the
    /// whole behaviour document, the media list, and a summary — which needed the document resident
    /// to answer a question SQL can answer directly.
    ///
    /// `content_uses` and `experience_uses` are counted separately because they are different
    /// answers to "what breaks if I delete this?": one is captions and groups, the other is which
    /// media a timeline stage selects.
    pub async fn get_tag_rows(&self) -> Result<Vec<TagRow>> {
        self.db_execute(move |conn| ops::library::tag_rows(&conn))
            .await
    }

    /// Every media slot pointing at `media`, described the way an author would recognize it.
    ///
    /// Deleting a file clears the slots referencing it, which is right and is a surprising thing to
    /// discover afterwards — the pack quietly stops having a wallpaper. Naming them beforehand is
    /// what makes that a decision rather than an accident.
    ///
    /// Includes stages of a switched-off timeline, deliberately: those slots still exist, and a
    /// removal would still empty them.
    pub async fn get_media_usage(&self, media: u64) -> Result<Vec<MediaUsage>> {
        self.db_execute(move |conn| ops::library::media_usage(&conn, media))
            .await
    }

    /// Renames a tag. The behaviour document follows on its own: every tag list in it is a join
    /// to `tags`, so the rows that mention this tag hold its *id* and nothing about them changes.
    ///
    /// This used to walk the whole document rewriting strings (`Behaviour::rewrite_tag`) and write
    /// it back, then hand the result to the front end to adopt. It writes one row, and the surfaces
    /// showing tags refetch.
    pub async fn rename_tag(&self, from: String, to: String) -> Result<()> {
        reject_managed_tag(&from)?;
        reject_managed_tag(&to)?;
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| ops::rename_tag(&conn, &from, &to))
            .await?;
        self.mark_unsaved().await
    }

    /// Folds one tag into another, everywhere it appears -- on media and throughout the behaviour
    /// document alike.
    ///
    /// Each join table is re-pointed with `UPDATE OR IGNORE`, which skips the rows where the
    /// target tag is already present (that pair is the primary key), and the leftovers are then
    /// deleted. That is the deduplication `rewrite_tag` did by hand with a `HashSet`.
    pub async fn merge_tag(&self, from: String, to: String) -> Result<()> {
        reject_managed_tag(&from)?;
        reject_managed_tag(&to)?;
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| ops::merge_tag(&conn, &from, &to))
            .await?;
        self.mark_unsaved().await
    }

    /// Deletes a tag from the pack entirely.
    ///
    /// One statement: every table that references a tag does so with `ON DELETE CASCADE`, so the
    /// media associations and every mention in the behaviour document go with the row. This used
    /// to be three statements and a document rewrite.
    pub async fn delete_tag(&self, tag: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| ops::delete_tag(&conn, &tag))
            .await?;
        self.mark_unsaved().await
    }

    pub async fn get_tags(&self, id: u64) -> Result<Vec<String>> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT tags.name FROM media_tags LEFT JOIN tags ON media_tags.tag_id = tags.id WHERE media_tags.media_id = ?",
            )?;
            let rows = stmt.query_map(params![id], |row| row.get("name"))?;
            rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
        })
        .await
    }

    pub async fn add_tag(&self, id: u64, tag: String) -> Result<()> {
        reject_managed_tag(&tag)?;
        let _handle = self.saving.read().await;
        self.db_execute(move |connection| {
            history::record(&connection, "Add media tag", 0, &[id], |tx| {
                let tag_id: u64 =
                    tx.query_row("SELECT id FROM tags WHERE name = ?", params![tag], |row| {
                        row.get("id")
                    })?;
                tx.execute(
                    "INSERT INTO media_tags (media_id, tag_id) VALUES (?, ?)",
                    params![id, tag_id],
                )?;
                Ok(())
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn create_and_add_tag(&self, id: u64, tag: String) -> Result<()> {
        reject_managed_tag(&tag)?;
        let _handle = self.saving.read().await;
        self.db_execute(move |connection| {
            history::record(&connection, "Create and add media tag", 0, &[id], |tx| {
                let tag_id: u64 = tx.query_row(
                    "INSERT INTO tags (name) VALUES (?) RETURNING id",
                    params![tag],
                    |row| row.get("id"),
                )?;
                tx.execute(
                    "INSERT INTO media_tags (media_id, tag_id) VALUES (?, ?)",
                    params![id, tag_id],
                )?;
                Ok(())
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    /// Bulk get-or-create + associate, for a caller (the Edgeware importer) that already knows a
    /// media file's full tag set up front rather than adding tags one at a time via the UI --
    /// unlike `add_tag`, a tag here doesn't need to already exist.
    pub async fn add_tags(&self, id: u64, tags: Vec<String>) -> Result<()> {
        if tags.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        self.db_execute(move |mut conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            for tag in &tags {
                tx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?)", params![tag])?;
                let tag_id: u64 =
                    tx.query_row("SELECT id FROM tags WHERE name = ?", params![tag], |row| {
                        row.get("id")
                    })?;
                tx.execute(
                    "INSERT OR IGNORE INTO media_tags (media_id, tag_id) VALUES (?, ?)",
                    params![id, tag_id],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
        self.mark_unsaved().await
    }

    /// Bulk get-or-create + associate, for a caller (media import, and the Edgeware importer's
    /// converted attribution) that already knows a media file's full artist set up front rather
    /// than adding artists one at a time via the UI -- unlike `add_artist`, an artist here
    /// doesn't need to already exist.
    pub async fn add_artists(&self, id: u64, artists: Vec<String>) -> Result<()> {
        if artists.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        self.db_execute(move |mut conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            for artist in &artists {
                tx.execute(
                    "INSERT OR IGNORE INTO artists (name) VALUES (?)",
                    params![artist],
                )?;
                let artist_id: u64 = tx.query_row(
                    "SELECT id FROM artists WHERE name = ?",
                    params![artist],
                    |row| row.get("id"),
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO media_artists (media_id, artist_id) VALUES (?, ?)",
                    params![id, artist_id],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn remove_tag(&self, id: u64, tag: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |connection| {
            history::record(&connection, "Remove media tag", 0, &[id], |tx| {
                tx.execute(
                    "DELETE FROM media_tags WHERE media_id = ? AND tag_id IN (SELECT id FROM tags WHERE name = ?)",
                    params![id, tag],
                )?;
                Ok(())
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn add_tag_to_files(&self, ids: Vec<u64>, tag: String) -> Result<()> {
        reject_managed_tag(&tag)?;
        if ids.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| ops::add_tag_to_files(&conn, &ids, &tag))
            .await?;
        self.mark_unsaved().await
    }

    pub async fn remove_tag_from_files(&self, ids: Vec<u64>, tag: String) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| ops::remove_tag_from_files(&conn, &ids, &tag))
            .await?;
        self.mark_unsaved().await
    }

    pub async fn get_artists(&self, id: u64) -> Result<Vec<String>> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT artists.name FROM media_artists LEFT JOIN artists ON media_artists.artist_id = artists.id WHERE media_artists.media_id = ?",
            )?;
            let rows = stmt.query_map(params![id], |row| row.get("name"))?;
            rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
        })
        .await
    }

    pub async fn add_artist(&self, id: u64, artist: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |connection| {
            history::record(&connection, "Add media artist", 0, &[id], |tx| {
                let artist_id: u64 = tx.query_row(
                    "SELECT id FROM artists WHERE name = ?",
                    params![artist],
                    |row| row.get("id"),
                )?;
                tx.execute(
                    "INSERT INTO media_artists (media_id, artist_id) VALUES (?, ?)",
                    params![id, artist_id],
                )?;
                Ok(())
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn create_and_add_artist(&self, id: u64, artist: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |connection| {
            history::record(&connection, "Create and add media artist", 0, &[id], |tx| {
                let artist_id: u64 = tx.query_row(
                    "INSERT INTO artists (name) VALUES (?) RETURNING id",
                    params![artist],
                    |row| row.get("id"),
                )?;
                tx.execute(
                    "INSERT INTO media_artists (media_id, artist_id) VALUES (?, ?)",
                    params![id, artist_id],
                )?;
                Ok(())
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn remove_artist(&self, id: u64, artist: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |connection| {
            history::record(&connection, "Remove media artist", 0, &[id], |tx| {
                tx.execute(
                    "DELETE FROM media_artists WHERE media_id = ? AND artist_id IN (SELECT id FROM artists WHERE name = ?)",
                    params![id, artist],
                )?;
                Ok(())
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn get_all_artists(&self) -> Result<Vec<String>> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare("SELECT name FROM artists")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>("name"))?;
            rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
        })
        .await
    }

    pub async fn get_artist_summaries(&self) -> Result<Vec<ArtistSummary>> {
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT artists.name, COUNT(media.id) FROM artists
                 LEFT JOIN media_artists ON media_artists.artist_id = artists.id
                 LEFT JOIN media ON media.id = media_artists.media_id AND media.deleted = FALSE
                 GROUP BY artists.id ORDER BY artists.name COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(ArtistSummary {
                    name: row.get(0)?,
                    media_count: row.get(1)?,
                })
            })?;
            rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
        })
        .await
    }

    pub async fn rename_artist(&self, from: String, to: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| ops::rename_artist(&conn, &from, &to))
            .await?;
        self.mark_unsaved().await
    }

    pub async fn merge_artist(&self, from: String, to: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| ops::merge_artist(&conn, &from, &to))
            .await?;
        self.mark_unsaved().await
    }

    pub async fn delete_artist(&self, artist: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| ops::delete_artist(&conn, &artist))
            .await?;
        self.mark_unsaved().await
    }

    pub async fn add_artist_to_files(&self, ids: Vec<u64>, artist: String) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| ops::add_artist_to_files(&conn, &ids, &artist))
            .await?;
        self.mark_unsaved().await
    }

    pub async fn remove_artist_from_files(&self, ids: Vec<u64>, artist: String) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| ops::remove_artist_from_files(&conn, &ids, &artist))
            .await?;
        self.mark_unsaved().await
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaUsageTarget {
    Wallpaper,
    Splash,
    Stage { id: String },
}

#[derive(Serialize)]
pub struct MediaUsage {
    pub label: String,
    pub target: MediaUsageTarget,
}

/// Get-or-create a tag and attach it to one media file. Used for the managed markers, which is
/// why it doesn't go through the author-facing commands (those refuse the reserved namespace).
pub(super) fn apply_tag(tx: &rusqlite::Transaction<'_>, media_id: u64, tag: &str) -> Result<()> {
    tx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?)", params![tag])?;
    let tag_id: u64 = tx.query_row("SELECT id FROM tags WHERE name = ?", params![tag], |row| {
        row.get(0)
    })?;
    tx.execute(
        "INSERT OR IGNORE INTO media_tags (media_id, tag_id) VALUES (?, ?)",
        params![media_id, tag_id],
    )?;
    Ok(())
}
