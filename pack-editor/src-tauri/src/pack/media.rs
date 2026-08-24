//! Importing media into the pack, removing it, undo/redo over those edits, and the per-file
//! attributes (title, source URL, popup role, behaviour media slots).

use super::behaviour::{
    conn_write_behaviour_if_changed, fill_media_slot_tx, slot_label,
};
use super::labels::apply_tag;
use super::*;

use std::{io, path::Path};

use crate::history;
use shared::behaviour::editor as behaviour_editor;
use anyhow::{anyhow, bail, Result};
use rusqlite::{named_params, params, OptionalExtension, TransactionBehavior};
use shared::{
    behaviour::MediaSlot,
    encode::{EncodedFile, FileInfo, FileInfoParts},
    pack::Metadata,
    tags,
};

impl MediaPack {
    /// Returns `Ok(None)` if `hash` is already present - a DB-level `UNIQUE`
    /// constraint on `media.hash`, not just the caller's own pre-check, so this
    /// is safe even when two uploads with identical content race each other (the
    /// pre-check alone can't catch that: both can see "not present yet" before
    /// either has inserted its row - the constraint is what actually closes it).
    ///
    /// `file_name` is also `UNIQUE`, but a collision there means something different: unlike
    /// identical *content*, a same-named but distinct file should still be added, just under an
    /// adjusted name (`"name (1).ext"`, counting up on repeated collisions -- see
    /// `next_candidate_name`) rather than rejected. The returned `MediaFile.file_name` reflects
    /// whatever name it actually landed on, which callers should use instead of assuming their
    /// requested name was applied verbatim.
    #[cfg(test)]
    pub(super) async fn add_file(
        &self,
        encoded_file: EncodedFile,
        path: &Path,
        hash: blake3::Hash,
    ) -> Result<Option<MediaFile>> {
        Ok(self
            .add_file_to_import(encoded_file, path, hash, None)
            .await?
            .and_then(AddOutcome::into_added))
    }

    pub async fn begin_media_import(&self) -> Result<i64> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| history::begin_media_import(&mut connection))
            .await
    }

    pub async fn finish_media_import(
        &self,
        history_id: i64,
        label: Option<String>,
    ) -> Result<history::Status> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| {
            history::finish_media_import(&mut connection, history_id, label.as_deref())
        })
        .await
    }

    pub async fn add_file_to_import(
        &self,
        encoded_file: EncodedFile,
        path: &Path,
        hash: blake3::Hash,
        history_id: Option<i64>,
    ) -> Result<Option<AddOutcome>> {
        let handle = self.saving.read().await;

        let FileInfoParts {
            file_type,
            width,
            height,
            transparent,
            duration,
            audio,
        } = encoded_file.info.to_parts();

        let mut file_name = file_name(path);
        let file_path = encoded_file
            .path
            .strip_prefix(self.dir.join("media"))
            .unwrap_or(&encoded_file.path)
            .to_string_lossy()
            .to_string();
        let file_info = encoded_file.info.clone();
        let file_type_str = file_type.as_str();
        let thumbnail = encoded_file.thumbnail.clone();
        let hash_bytes = *hash.as_bytes();
        let size = tokio::fs::metadata(&encoded_file.path).await?.len();
        let source_url = encoded_file.source_url.clone();
        let encoded_path = encoded_file.path.clone();

        let add_result = loop {
            let file_name_clone = file_name.clone();
            let file_path = file_path.clone();
            let thumbnail = thumbnail.clone();
            let source_url = source_url.clone();

            let insert_result = self
                .db_execute(move |mut conn| {
                    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    let existing: Option<(u64, bool, u64)> = tx
                        .query_row(
                            "SELECT m.id, m.deleted, COALESCE(l.length, p.length, 0)
                             FROM media m
                             LEFT JOIN loose_media l ON l.media_id = m.id
                             LEFT JOIN pack_media p ON p.media_id = m.id
                             WHERE m.hash = ?",
                            [hash_bytes],
                            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                        )
                        .optional()?;
                    if let Some((id, deleted, existing_size)) = existing {
                        if !deleted {
                            tx.commit()?;
                            return Ok(AddFileResult::Duplicate(id));
                        }
                        tx.execute("UPDATE media SET deleted = 0 WHERE id = ?", [id])?;
                        if let Some(history_id) = history_id {
                            history::add_imported_media(&tx, history_id, id, existing_size)?;
                        }
                        tx.commit()?;
                        return Ok(AddFileResult::Restored(id));
                    }
                    let id = tx.query_row(
                        "INSERT INTO media (file_name, file_type, width, height, transparent, duration, audio, hash, thumbnail, source_url)
                        VALUES (:file_name, :file_type, :width, :height, :transparent, :duration, :audio, :hash, :thumbnail, :source_url) RETURNING id",
                        named_params! {
                            ":file_name": file_name_clone,
                            ":file_type": file_type_str,
                            ":width": width,
                            ":height": height,
                            ":transparent": transparent,
                            ":duration": duration,
                            ":audio": audio,
                            ":hash": hash_bytes,
                            ":thumbnail": thumbnail,
                            ":source_url": source_url,
                        },
                        |row| row.get::<_, u64>("id"),
                    )?;
                    tx.execute(
                        "INSERT INTO loose_media(media_id, path, length) VALUES (?, ?, ?)",
                        params![id, file_path, size],
                    )?;
                    if let Some(history_id) = history_id {
                        history::add_imported_media(&tx, history_id, id, size)?;
                    }
                    tx.commit()?;
                    Ok(AddFileResult::Inserted(id))
                })
                .await;

            match insert_result {
                Ok(result) => break result,
                Err(err) if violates_unique_constraint(&err, "hash") => {
                    match tokio::fs::remove_file(&encoded_path).await {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                    return Ok(None);
                }
                Err(err) if violates_unique_constraint(&err, "file_name") => {
                    file_name = next_candidate_name(&file_name);
                }
                Err(err) => return Err(err),
            }
        };

        let id = match add_result {
            AddFileResult::Inserted(id) => id,
            AddFileResult::Restored(id) => {
                match tokio::fs::remove_file(&encoded_path).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                self.mark_unsaved_now()?;
                drop(handle);
                return self
                    .get_files()
                    .await?
                    .into_iter()
                    .find(|file| file.id == id)
                    .map(|file| Some(AddOutcome::Added(file)))
                    .ok_or_else(|| anyhow!("restored media {id} is not active"));
            }
            AddFileResult::Duplicate(existing) => {
                match tokio::fs::remove_file(&encoded_path).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                drop(handle);
                // The bytes are already here under some other name. A bulk import treats that as
                // a skip, but a media slot needs to know *which* file it was so it can point at
                // it -- "add to the slot" and "the pack already has this" are the same answer.
                return Ok(self
                    .get_files()
                    .await?
                    .into_iter()
                    .find(|file| file.id == existing)
                    .map(AddOutcome::Duplicate));
            }
        };
        // Do this synchronously immediately after the database commit. Upload cancellation can
        // drop the async task at its next await; the recovery marker must already exist by then.
        self.mark_unsaved_now()?;
        if !encoded_file.artists.is_empty() {
            self.add_artists(id, encoded_file.artists.clone()).await?;
        }

        Ok(Some(AddOutcome::Added(MediaFile {
            id,
            file_name,
            file_info,
            hash: hash.to_string(),
            tags: vec![],
            artists: encoded_file.artists,
            source_url: encoded_file.source_url,
            size,
        })))
    }

    pub async fn remove_files(&self, ids: Vec<u64>) -> Result<()> {
        let _handle = self.saving.read().await;
        let label = if ids.len() == 1 {
            "Remove media item".to_string()
        } else {
            format!("Remove {} media items", ids.len())
        };
        self.db_execute(move |mut conn| {
                if ids.is_empty() {
                    return Ok(());
                }
                history::record_media_visibility(&mut conn, &label, &ids, true)?;
                // Deleting a file has to take any slot pointing at it with it, or the slot is
                // left pointing at media the pack no longer has. Deletion is the only thing that
                // can do this now that slots hold ids -- a rename leaves them alone.
                conn_write_behaviour_if_changed(&mut conn, &label, |behaviour| {
                    // Every id, not the first match: `any` would stop early and leave the other
                    // slots pointing at files this same call deleted.
                    let mut changed = false;
                    for id in &ids {
                        changed |= behaviour.clear_media_reference(*id);
                    }
                    changed
                })
                .map(|_| ())
            })
            .await?;
        self.mark_unsaved().await
    }

    pub async fn history_status(&self) -> Result<history::Status> {
        self.db_execute(move |conn| history::status(&conn)).await
    }

    pub async fn undo(&self) -> Result<history::Status> {
        let _handle = self.saving.read().await;
        let status = self
            .db_execute(move |mut conn| history::undo(&mut conn))
            .await?;
        self.reload_metadata_from_db().await?;
        if status.at_saved_state {
            self.mark_saved().await?;
        } else {
            self.mark_unsaved().await?;
        }
        Ok(status)
    }

    pub async fn redo(&self) -> Result<history::Status> {
        let _handle = self.saving.read().await;
        let status = self
            .db_execute(move |mut conn| history::redo(&mut conn))
            .await?;
        self.reload_metadata_from_db().await?;
        if status.at_saved_state {
            self.mark_saved().await?;
        } else {
            self.mark_unsaved().await?;
        }
        Ok(status)
    }

    pub(super) async fn reload_metadata_from_db(&self) -> Result<()> {
        let bytes = self
            .db_execute(move |conn| {
                conn.query_row(
                    "SELECT metadata FROM pack_metadata WHERE singleton = 1",
                    [],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .map_err(Into::into)
            })
            .await?;
        *self.metadata.write().unwrap() = Metadata::from_buf(&bytes)?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) async fn purge_history_files(&self, ids: Vec<u64>) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        let db_ids = ids.clone();
        self.db_execute(move |mut conn| {
            let selected = media_id_array(&db_ids)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT OR IGNORE INTO pending_file_deletions(path)
                 SELECT l.path FROM loose_media l JOIN media m ON m.id = l.media_id
                 WHERE m.deleted = 1 AND m.id IN (SELECT value FROM rarray(?))
                   AND NOT EXISTS (SELECT 1 FROM snapshot_media s WHERE s.media_id = m.id)
                   AND NOT EXISTS (SELECT 1 FROM history_media_refs h WHERE h.media_id = m.id)",
                params![&selected],
            )?;
            tx.execute(
                "DELETE FROM media WHERE deleted = 1
                 AND id IN (SELECT value FROM rarray(?))
                 AND NOT EXISTS (SELECT 1 FROM snapshot_media s WHERE s.media_id = media.id)
                 AND NOT EXISTS (SELECT 1 FROM history_media_refs h WHERE h.media_id = media.id)",
                params![&selected],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?;
        self.process_pending_file_deletions().await?;
        Ok(())
    }

    pub async fn get_files(&self) -> Result<Vec<MediaFile>> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT m.id, m.file_type, m.file_name, m.width, m.height, m.transparent,
                        m.duration, m.audio, m.hash, COALESCE(l.length, p.length) AS length,
                        m.source_url
                 FROM media m
                 LEFT JOIN loose_media l ON l.media_id = m.id
                 LEFT JOIN pack_media p ON p.media_id = m.id
                 WHERE m.deleted = 0",
            )?;
            let mut files: Vec<MediaFile> = {
                let rows = stmt.query_and_then([], |row| -> Result<_> {
                    Ok(MediaFile {
                        id: row.get("id")?,
                        file_name: row.get("file_name")?,
                        file_info: FileInfo::try_from_parts(&FileInfoParts {
                            file_type: row.get::<_, String>("file_type")?.parse()?,
                            width: row.get("width")?,
                            height: row.get("height")?,
                            transparent: row.get("transparent")?,
                            duration: row.get("duration")?,
                            audio: row.get("audio")?,
                        })?,
                        hash: blake3::Hash::from_bytes(row.get("hash")?).to_string(),
                        tags: vec![],
                        artists: vec![],
                        source_url: row.get("source_url")?,
                        size: row.get::<_, Option<u64>>("length")?.unwrap_or(0),
                    })
                })?;
                rows.collect::<Result<Vec<_>>>()?
            };

            // Build id → index map then load all tag/artist associations in one query each.
            let id_to_idx: std::collections::HashMap<u64, usize> =
                files.iter().enumerate().map(|(i, f)| (f.id, i)).collect();

            let mut tag_stmt = conn.prepare(
                "SELECT mt.media_id, t.name FROM media_tags mt JOIN tags t ON mt.tag_id = t.id",
            )?;
            let tag_rows = tag_stmt.query_map([], |row| {
                Ok((row.get::<_, u64>("media_id")?, row.get::<_, String>("name")?))
            })?;
            for row in tag_rows {
                let (media_id, tag_name) = row?;
                if let Some(&idx) = id_to_idx.get(&media_id) {
                    files[idx].tags.push(tag_name);
                }
            }

            let mut artist_stmt = conn.prepare(
                "SELECT ma.media_id, a.name FROM media_artists ma JOIN artists a ON ma.artist_id = a.id",
            )?;
            let artist_rows = artist_stmt.query_map([], |row| {
                Ok((row.get::<_, u64>("media_id")?, row.get::<_, String>("name")?))
            })?;
            for row in artist_rows {
                let (media_id, artist_name) = row?;
                if let Some(&idx) = id_to_idx.get(&media_id) {
                    files[idx].artists.push(artist_name);
                }
            }

            Ok(files)
        })
        .await
    }

    /// Every tag the author owns. Managed tags are filtered out here rather than at each of the
    /// half-dozen pickers this feeds (the Tags tab, `TagPicker`, the content-group pickers), so
    /// there is one place that decides what an author can see -- see `shared::tags`.
    pub async fn get_all_tags(&self) -> Result<Vec<String>> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare("SELECT name FROM tags")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>("name"))?;
            Ok(rows
                .collect::<rusqlite::Result<Vec<String>>>()?
                .into_iter()
                .filter(|tag| !tags::is_managed(tag))
                .collect())
        })
        .await
    }

    pub async fn get_modes(&self) -> Result<Vec<(u64, Vec<u8>)>> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare("SELECT id, file FROM modes ORDER BY id")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, u64>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
        })
        .await
    }

    pub async fn add_mode(&self, file: Vec<u8>) -> Result<u64> {
        let _handle = self.saving.read().await;
        let storage_bytes = file.len() as u64;
        let id = self
            .db_execute(move |mut connection| {
                history::record(
                    &mut connection,
                    "Add custom mode",
                    storage_bytes,
                    &[],
                    |tx| {
                        tx.query_row(
                            "INSERT INTO modes (file) VALUES (?) RETURNING id",
                            params![file],
                            |row| row.get(0),
                        )
                        .map_err(Into::into)
                    },
                )
            })
            .await?;
        self.mark_unsaved().await?;
        Ok(id)
    }

    pub async fn remove_mode(&self, id: u64) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| {
            let storage_bytes: u64 = connection.query_row(
                "SELECT length(file) FROM modes WHERE id = ?",
                [id],
                |row| row.get(0),
            )?;
            history::record(
                &mut connection,
                "Remove custom mode",
                storage_bytes,
                &[],
                |tx| {
                    tx.execute("DELETE FROM modes WHERE id = ?", [id])?;
                    Ok(())
                },
            )
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn set_source_url(&self, id: u64, url: Option<String>) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| {
            history::record(&mut connection, "Set media source", 0, &[id], |tx| {
                tx.execute(
                    "UPDATE media SET source_url = ? WHERE id = ?",
                    params![url, id],
                )?;
                Ok(())
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    /// The active file with these bytes, if the pack already has one. `check_hash`'s answer with
    /// the identity attached, for the slot path -- which points at the existing file rather than
    /// skipping.
    pub async fn file_with_hash(&self, hash: &blake3::Hash) -> Result<Option<MediaFile>> {
        let hash_bytes = *hash.as_bytes();
        let existing: Option<u64> = self
            .db_execute(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT id FROM media WHERE hash = ? AND deleted = 0",
                        params![hash_bytes],
                        |row| row.get(0),
                    )
                    .optional()?)
            })
            .await?;
        let Some(id) = existing else {
            return Ok(None);
        };
        Ok(self
            .get_files()
            .await?
            .into_iter()
            .find(|file| file.id == id))
    }

    pub async fn check_hash(&self, hash: &blake3::Hash) -> Result<bool> {
        let hash_bytes = *hash.as_bytes();
        self.db_execute(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT 1 FROM media WHERE hash = ? AND deleted = 0",
                    params![hash_bytes],
                    |_| Ok(1),
                )
                .optional()?
                .is_some())
        })
        .await
    }

    /// Unlike `add_file`, a name collision here is a deliberate user action (renaming an existing
    /// file to a name they typed), so it's rejected with a clear error rather than silently
    /// adjusted -- auto-renaming would mean the name shown afterwards isn't the one they asked
    /// for, with no indication why.
    /// The behaviour document is deliberately untouched: its media slots hold ids, so a rename
    /// cannot invalidate one. This used to have to rewrite every slot naming the old name and
    /// hand the document back for the front end to adopt -- the round trip that referencing by
    /// id exists to remove (`design/behaviour-storage.md`, step 1).
    pub async fn set_title(&self, id: u64, name: String) -> Result<()> {
        let _handle = self.saving.read().await;
        let name_clone = name.clone();
        let result = self
            .db_execute(move |mut connection| {
                history::record(&mut connection, "Rename media item", 0, &[id], |tx| {
                    tx.execute(
                        "UPDATE media SET file_name = ? WHERE id = ?",
                        params![name_clone, id],
                    )?;
                    Ok(())
                })
            })
            .await;

        match result {
            Ok(()) => {}
            Err(err) if violates_unique_constraint(&err, "file_name") => {
                bail!("A file named \"{name}\" already exists");
            }
            Err(err) => return Err(err),
        }

        self.mark_unsaved().await?;
        Ok(())
    }

    /// Points a media slot at `media_id`, marking that file as scenery rather than popup content
    /// if `new_to_pack` (see [`MediaAddition::new_to_pack`]).
    ///
    /// The two halves are one operation because they mean one thing to the author ("this is the
    /// wallpaper") and because a slot filled without the marker would put the desktop background
    /// in a popup. One history entry, so undo takes both back together.
    pub async fn fill_media_slot(
        &self,
        slot: MediaSlot,
        media_id: u64,
        new_to_pack: bool,
    ) -> Result<Option<u64>> {
        let _handle = self.saving.read().await;
        let result = self
            .db_execute(move |mut conn| {
                history::record_with_media_refs(&mut conn, slot_label(&slot), 0, |tx| {
                    let deleted = fill_media_slot_tx(tx, &slot, media_id, new_to_pack)?;
                    // The displaced file's bytes have to outlive the entry that dropped it, or undo
                    // would restore a slot pointing at media the collector has taken away.
                    let refs = [Some(media_id), deleted].into_iter().flatten().collect();
                    Ok((deleted, refs))
                })
            })
            .await?;
        self.mark_unsaved().await?;
        Ok(result)
    }

    /// `fill_media_slot` folded into a media import that is still open, so importing the file and
    /// pointing the slot at it land as one undo step -- see `history::record_into_pending`. The
    /// caller finishes the import afterwards, naming the entry for the whole action.
    pub async fn fill_media_slot_during_import(
        &self,
        slot: MediaSlot,
        media_id: u64,
        history_id: i64,
        new_to_pack: bool,
    ) -> Result<Option<u64>> {
        let _handle = self.saving.read().await;
        let result = self
            .db_execute(move |mut conn| {
                history::record_into_pending(&mut conn, history_id, |tx| {
                    let deleted = fill_media_slot_tx(tx, &slot, media_id, new_to_pack)?;
                    // The soft delete itself rides in the entry's changeset, but the collector only
                    // spares media some entry names -- and this one is displaced, not imported, so
                    // `add_imported_media` never sees it.
                    if let Some(id) = deleted {
                        history::add_media_ref(tx, history_id, id)?;
                    }
                    Ok(deleted)
                })
            })
            .await?;
        self.mark_unsaved().await?;
        Ok(result)
    }

    /// Empties a media slot, deleting the file it held if that file was only ever scenery.
    ///
    /// A file still carrying `__lewdware-non-popup` exists for this slot and nothing else, so
    /// leaving it behind would litter the pack with images the author can't see in the media grid
    /// as anything but clutter. One that has been un-marked ("show as popup") is real content and
    /// stays. Either way it stays if another slot still references it -- a base wallpaper reused
    /// by a timeline stage is Edgeware's ordinary case, and clearing the stage must not take the
    /// pack's main wallpaper with it.
    ///
    /// Returns the id of the file it deleted, if any.
    pub async fn clear_media_slot(&self, slot: MediaSlot) -> Result<Option<u64>> {
        let _handle = self.saving.read().await;
        let result = self
            .db_execute(move |mut conn| {
                history::record_with_media_refs(&mut conn, slot_label(&slot), 0, |tx| {
                    let Some(media) = behaviour_editor::slot_value(tx, &slot)? else {
                        return Ok((None, vec![]));
                    };
                    behaviour_editor::set_media_slot(tx, &slot, None)?;
                    // After the slot is emptied, so "still referenced" means what it means once
                    // the slot has let go of it.
                    let deleted = delete_unreferenced_scenery(tx, media)?;
                    let refs = deleted.map(|id| vec![id]).unwrap_or_default();
                    Ok((deleted, refs))
                })
            })
            .await?;
        self.mark_unsaved().await?;
        Ok(result)
    }

    /// Adds or removes `__lewdware-non-popup` on one file -- the "show as popup" toggle.
    ///
    /// Slot media carries the marker by default, and clearing it is how an author says "this is
    /// also real content": the file stays the wallpaper and starts appearing in popups too. The
    /// same toggle keeps any other file out of popups without involving a slot at all.
    pub async fn set_shown_as_popup(&self, id: u64, shown: bool) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut conn| {
            let label = if shown {
                "Show media in popups"
            } else {
                "Hide media from popups"
            };
            history::record(&mut conn, label, 0, &[id], |tx| {
                if shown {
                    tx.execute(
                        "DELETE FROM media_tags WHERE media_id = ? AND tag_id IN
                         (SELECT id FROM tags WHERE name = ?)",
                        params![id, tags::NON_POPUP_TAG],
                    )?;
                    Ok(())
                } else {
                    apply_tag(tx, id, tags::NON_POPUP_TAG)
                }
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    /// Moves audio between the mutually-exclusive background (unmarked) and popup roles.
    pub async fn set_popup_audio(&self, ids: Vec<u64>, popup: bool) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        self.db_execute(move |mut conn| {
            let label = if popup {
                "Move audio to Popup"
            } else {
                "Move audio to Background"
            };
            history::record(&mut conn, label, 0, &ids, |tx| {
                for id in &ids {
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
                        apply_tag(tx, *id, tags::POPUP_AUDIO_TAG)?;
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
        })
        .await?;
        self.mark_unsaved().await
    }
}

/// Deletes `media` if it exists only as scenery that `behaviour` no longer refers to -- the rule
/// behind both ways a slot can stop pointing at a file, being cleared and being replaced.
///
/// A file still carrying `__lewdware-non-popup` was brought in for a slot and nothing else, so a
/// slot letting go of it leaves a file the author cannot see anywhere: not in Popups, which
/// excludes the marker, and not in any slot or pool. One that has been un-marked ("show as popup")
/// is real content and stays, and so does one another slot still references -- a base wallpaper
/// reused by a timeline stage is Edgeware's ordinary case.
///
/// Call with the *updated* behaviour, so "still referenced" means after the change, not before.
pub(super) fn delete_unreferenced_scenery(
    tx: &rusqlite::Transaction<'_>,
    media: u64,
) -> Result<Option<u64>> {
    // Asked of the tables, not of a read document: a document read while the timeline is switched
    // off has no stages in it, so every stage wallpaper would look unreferenced to this and be
    // deleted. See `shared::behaviour::editor::is_media_referenced`.
    if shared::behaviour::editor::is_media_referenced(tx, media)? {
        return Ok(None);
    }
    let scenery: Option<u64> = tx
        .query_row(
            "SELECT media.id FROM media
             JOIN media_tags ON media_tags.media_id = media.id
             JOIN tags ON tags.id = media_tags.tag_id
             WHERE media.id = ? AND media.deleted = 0 AND tags.name IN (?, ?)",
            params![media, tags::NON_POPUP_TAG, tags::EXPLICIT_ONLY_TAG],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = scenery {
        tx.execute("UPDATE media SET deleted = 1 WHERE id = ?", params![id])?;
    }
    Ok(scenery)
}
