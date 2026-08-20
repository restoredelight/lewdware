//! The save pipeline: snapshotting the working database, streaming media into a new archive
//! generation (or compacting the existing one in place), and committing the result.

use super::view::copy_range_forward;
use super::*;

use std::{
    fs::{self},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Result};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rayon::prelude::*;
use rusqlite::{
    backup::{Backup, StepResult},
    params,
    types::Value,
    vtab::array::Array,
    OpenFlags, TransactionBehavior,
};
use shared::pack::{Header, Metadata, HEADER_SIZE};
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    task::spawn_blocking,
};
use uuid::Uuid;

use crate::editor_db;

impl MediaPack {
    pub(super) async fn create_save_snapshot(&self) -> Result<SaveSnapshot> {
        let snapshot_started = Instant::now();
        let staging = tempfile::Builder::new()
            .prefix("pack-editor-save-")
            .tempdir_in(&self.dir)?;
        let db_path = staging.path().join("index.db");
        let runtime_path = staging.path().join("runtime.db");
        let save_id = Uuid::new_v4().to_string();
        let generation_id = Uuid::new_v4().to_string();

        // Establish the SQLite and in-memory parts of the snapshot at one precise
        // mutation boundary. The open read transaction pins the current WAL view,
        // so the expensive backup can continue after this application lock is
        // released without preventing other pooled connections from writing.
        let _mutation_guard = self.saving.write().await;
        let session_pool = self.db_pool.clone();
        let save_id_for_session = save_id.clone();
        let generation_for_session = generation_id.clone();
        let (editor_state_id, pending_history) =
            spawn_blocking(move || -> Result<(String, bool)> {
                let mut connection = session_pool.get()?;
                let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let snapshot_state: String = tx.query_row(
                    "SELECT current_state_id FROM editor_state WHERE singleton = 1",
                    [],
                    |row| row.get(0),
                )?;
                let pending_history: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM history_entries WHERE status = 'pending')",
                    [],
                    |row| row.get(0),
                )?;
                tx.execute(
                    "INSERT INTO save_sessions(id, generation_id, editor_state_id)
                 VALUES (?, ?, ?)",
                    params![save_id_for_session, generation_for_session, snapshot_state],
                )?;
                tx.execute(
                    "INSERT INTO snapshot_media(save_id, media_id)
                 SELECT ?, id FROM media WHERE deleted = 0",
                    [&save_id_for_session],
                )?;
                tx.commit()?;
                Ok((snapshot_state, pending_history))
            })
            .await??;
        log_save_phase("snapshot_register_media", snapshot_started);
        let backup_started = Instant::now();
        let live_db_path = self.db_path.clone();
        let source = spawn_blocking(move || -> Result<_> {
            // This must not come from r2d2: if establishing or copying the
            // snapshot fails, closing this dedicated connection reliably rolls
            // back its transaction instead of returning transaction state to the
            // live pool.
            let conn = rusqlite::Connection::open_with_flags(
                live_db_path,
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            conn.busy_timeout(Duration::from_millis(5000))?;
            conn.execute_batch("BEGIN DEFERRED")?;
            let _: String = conn.query_row(
                "SELECT current_state_id FROM editor_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )?;
            Ok(conn)
        })
        .await??;
        let revision = self.revision.load(Ordering::SeqCst);
        let metadata = self.metadata.read().unwrap().clone();
        drop(_mutation_guard);

        let destination = db_path.clone();
        spawn_blocking(move || -> Result<()> {
            let mut destination = rusqlite::Connection::open(destination)?;
            destination.busy_timeout(Duration::from_millis(5000))?;
            let backup = Backup::new(&source, &mut destination)?;
            loop {
                match backup.step(256)? {
                    StepResult::Done => break,
                    StepResult::More => std::thread::yield_now(),
                    StepResult::Busy | StepResult::Locked => sleep(Duration::from_millis(1)),
                    _ => sleep(Duration::from_millis(1)),
                }
            }
            // Release the pinned WAL snapshot before vacuuming the private copy.
            // On any earlier error or panic, normal RAII cleanup does the same.
            drop(backup);
            drop(source);
            // Journal mode is stored in the database and is copied by the backup.
            // Keep WAL on the live working DB, but make this self-contained snapshot
            // use a rollback journal: later offset updates must land in index.db
            // itself because that single file is what gets embedded in the pack.
            destination.pragma_update(None, "journal_mode", "DELETE")?;
            destination.execute_batch("VACUUM")?;
            Ok(())
        })
        .await??;
        log_save_phase("snapshot_backup_and_vacuum", backup_started);

        let db_pool = Pool::builder().build(sqlite_connection_manager(&db_path))?;
        Ok(SaveSnapshot {
            save_id,
            revision,
            generation_id,
            editor_state_id,
            pending_history,
            metadata,
            runtime_path,
            db_path,
            db_pool,
            _staging: staging,
        })
    }

    pub(super) async fn reconcile_save_snapshot(
        &self,
        snapshot: &SaveSnapshot,
        mark_snapshot_saved: bool,
    ) -> Result<bool> {
        let reconcile_started = Instant::now();
        let live_db_path = self.db_path.clone();
        let snapshot_db_path = snapshot.db_path.clone();
        let generation_id = snapshot.generation_id.clone();
        let editor_state_id = snapshot.editor_state_id.clone();
        let snapshot_is_current = self.revision.load(Ordering::SeqCst) == snapshot.revision;
        let saved_state_id =
            if !mark_snapshot_saved || (snapshot.pending_history && !snapshot_is_current) {
                None
            } else {
                Some(editor_state_id)
            };
        let media_count = spawn_blocking(move || -> Result<u64> {
            // A dedicated connection guarantees that ATTACH state is discarded on every error.
            let mut conn = rusqlite::Connection::open(live_db_path)?;
            conn.busy_timeout(Duration::from_millis(5000))?;
            conn.pragma_update(None, "foreign_keys", true)?;
            conn.execute(
                "ATTACH DATABASE ? AS save_snapshot",
                [snapshot_db_path.to_string_lossy().as_ref()],
            )?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let media_count: u64 = tx.query_row(
                "SELECT COUNT(*) FROM save_snapshot.media m
                 JOIN save_snapshot.pack_media p ON p.media_id = m.id
                 JOIN main.media live ON live.id = m.id AND live.hash = m.hash
                 WHERE m.deleted = 0",
                [],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO pack_media(media_id, generation_id, \"offset\", length)
                 SELECT snapshot.id, ?1, packed.\"offset\", packed.length
                 FROM save_snapshot.media snapshot
                 JOIN save_snapshot.pack_media packed ON packed.media_id = snapshot.id
                 JOIN main.media live ON live.id = snapshot.id AND live.hash = snapshot.hash
                 WHERE snapshot.deleted = 0
                 ON CONFLICT(media_id) DO UPDATE SET
                   generation_id = excluded.generation_id,
                   \"offset\" = excluded.\"offset\", length = excluded.length",
                [&generation_id],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO pending_file_deletions(path)
                 SELECT loose.path FROM loose_media loose
                 JOIN pack_media packed ON packed.media_id = loose.media_id
                 WHERE packed.generation_id = ?",
                [&generation_id],
            )?;
            tx.execute(
                "DELETE FROM loose_media WHERE media_id IN
                 (SELECT media_id FROM pack_media WHERE generation_id = ?)",
                [&generation_id],
            )?;
            tx.execute(
                "DELETE FROM pack_media WHERE generation_id <> ?",
                [&generation_id],
            )?;
            tx.execute(
                "UPDATE editor_state SET archive_generation = ?, saved_state_id = ?
                 WHERE singleton = 1",
                params![generation_id, saved_state_id],
            )?;
            tx.commit()?;
            // This is a dedicated connection. Closing it below reliably discards the attachment;
            // a fallible DETACH after commit must not turn a durable save into a reported failure.
            Ok(media_count)
        })
        .await??;
        tracing::info!(
            phase = "reconcile_live_database",
            elapsed_ms = reconcile_started.elapsed().as_secs_f64() * 1000.0,
            media_count,
            "save phase completed"
        );
        self.schedule_pending_file_deletions();
        Ok(snapshot_is_current)
    }

    pub(super) async fn clear_save_session(&self, save_id: &str) -> Result<()> {
        let pool = self.db_pool.clone();
        let save_id = save_id.to_string();
        spawn_blocking(move || -> Result<()> {
            pool.get()?
                .execute("DELETE FROM save_sessions WHERE id = ?", [&save_id])?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    pub(super) async fn write_snapshot_tail(
        &self,
        snapshot: &SaveSnapshot,
        archive_path: &Path,
        offset: u64,
    ) -> Result<Header> {
        let export_started = Instant::now();
        let snapshot_pool = snapshot.db_pool.clone();
        let runtime_path = snapshot.runtime_path.clone();
        spawn_blocking(move || -> Result<()> {
            let connection = snapshot_pool.get()?;
            editor_db::export_runtime(&connection, &runtime_path)?;
            Ok(())
        })
        .await??;
        log_save_phase("export_runtime_database", export_started);

        let tail_started = Instant::now();
        let mut file = OpenOptions::new().write(true).open(archive_path).await?;
        file.seek(SeekFrom::Start(offset)).await?;
        let index_length = {
            let mut dbf = File::open(&snapshot.runtime_path).await?;
            tokio::io::copy(&mut dbf, &mut file).await?
        };
        let metadata = snapshot.metadata.to_buf()?;
        let metadata_length = metadata.len() as u64;
        file.write_all(&metadata).await?;
        file.set_len(offset + index_length + metadata_length)
            .await?;

        let header = Header {
            id: self.header.read().unwrap().id,
            index_offset: offset,
            index_length,
            metadata_offset: offset + index_length,
            metadata_length,
        };
        file.seek(SeekFrom::Start(0)).await?;
        file.write_all(&header.to_buf()?).await?;
        log_save_phase("write_database_metadata_and_header", tail_started);
        let sync_started = Instant::now();
        file.sync_data().await?;
        log_save_phase("sync_generation_data", sync_started);
        Ok(header)
    }

    pub(super) async fn prepare_generation(
        &self,
        snapshot: &SaveSnapshot,
        on_progress: Arc<dyn Fn(usize, usize) + Send + Sync>,
    ) -> Result<PreparedGeneration> {
        let materialize_started = Instant::now();
        self.materialize_deleted_snapshot_media(snapshot).await?;
        log_save_phase("materialize_deleted_snapshot_media", materialize_started);
        let source = self
            .path
            .as_ref()
            .ok_or_else(|| anyhow!("pack has not been assigned a save location"))?
            .clone();
        let source_for_clone = source.clone();
        let pack_id = self.id();
        let clone_started = Instant::now();
        let generation =
            spawn_blocking(move || clone_archive(&source_for_clone, pack_id)).await??;
        log_save_phase("clone_archive", clone_started);
        let generation_path = generation.path().to_path_buf();
        let write_started = Instant::now();
        let offset = self
            .write_files_from(
                snapshot.db_pool.clone(),
                Some(generation_path.clone()),
                true,
                snapshot.generation_id.clone(),
                Some((self.db_pool.clone(), snapshot.save_id.clone())),
                on_progress,
            )
            .await?;
        log_save_phase("write_media_files", write_started);
        let header = self
            .write_snapshot_tail(snapshot, &generation_path, offset)
            .await?;
        Ok(PreparedGeneration {
            file: generation,
            header,
        })
    }

    pub(super) async fn materialize_deleted_snapshot_media(
        &self,
        snapshot: &SaveSnapshot,
    ) -> Result<()> {
        let source_path = match &self.path {
            Some(path) => path.clone(),
            None => return Ok(()),
        };
        let snapshot_pool = snapshot.db_pool.clone();
        let live_pool = self.db_pool.clone();
        let media_dir = self.dir.join("media");
        spawn_blocking(move || -> Result<()> {
            let snapshot_connection = snapshot_pool.get()?;
            let rows = {
                let mut statement = snapshot_connection.prepare(
                    "SELECT m.id, m.hash, p.\"offset\", p.length
                     FROM media m JOIN pack_media p ON p.media_id = m.id
                     LEFT JOIN loose_media l ON l.media_id = m.id
                     WHERE m.deleted = 1 AND l.media_id IS NULL",
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, u64>(0)?,
                            row.get::<_, Vec<u8>>(1)?,
                            row.get::<_, u64>(2)?,
                            row.get::<_, u64>(3)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            if rows.is_empty() {
                return Ok(());
            }
            let mut source = fs::File::open(source_path)?;
            for (id, hash, offset, length) in rows {
                let stored = format!("history-{}", Uuid::new_v4());
                let full_path = media_dir.join(&stored);
                let mut destination = fs::File::create(&full_path)?;
                source.seek(SeekFrom::Start(offset))?;
                io::copy(&mut (&mut source).take(length), &mut destination)?;
                destination.sync_data()?;

                let mut live = live_pool.get()?;
                let tx = live.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM media WHERE id = ? AND hash = ?)",
                    params![id, hash],
                    |row| row.get(0),
                )?;
                if exists {
                    tx.execute(
                        "INSERT INTO loose_media(media_id, path, length) VALUES (?, ?, ?)
                         ON CONFLICT(media_id) DO NOTHING",
                        params![id, stored, length],
                    )?;
                } else {
                    tx.execute(
                        "INSERT OR IGNORE INTO pending_file_deletions(path) VALUES (?)",
                        [&stored],
                    )?;
                }
                tx.commit()?;
            }
            Ok(())
        })
        .await??;
        Ok(())
    }

    pub(super) async fn commit_generation(
        &self,
        snapshot: &SaveSnapshot,
        generation: PreparedGeneration,
    ) -> Result<()> {
        let mutation_wait_started = Instant::now();
        let _mutation_guard = self.saving.write().await;
        log_save_phase("wait_for_generation_commit_lock", mutation_wait_started);
        let validation_started = Instant::now();
        self.validate_generation_commit(snapshot).await?;
        log_save_phase("validate_generation", validation_started);
        let archive_wait_started = Instant::now();
        let _archive_guard = self.archive_io.write().await;
        log_save_phase("wait_for_archive_commit_lock", archive_wait_started);
        let target = self.path.clone().unwrap();
        let persistence = spawn_blocking(move || -> Result<GenerationPersistence> {
            let persist_started = Instant::now();
            let persisted = generation
                .file
                .persist(&target)
                .map_err(|error| error.error)?;
            log_save_phase("persist_generation", persist_started);
            let sync_started = Instant::now();
            let sync_result = persisted
                .sync_all()
                .and_then(|_| sync_parent_directory(&target));
            log_save_phase("sync_persisted_generation", sync_started);
            Ok(match sync_result {
                Ok(()) => GenerationPersistence::Durable,
                Err(error) => GenerationPersistence::ReplacedButNotSynced(error.into()),
            })
        })
        .await??;
        *self.header.write().unwrap() = generation.header;

        let durable = matches!(persistence, GenerationPersistence::Durable);
        let reconcile_started = Instant::now();
        let snapshot_is_current = self.reconcile_save_snapshot(snapshot, durable).await?;
        log_save_phase("reconcile_save_snapshot_total", reconcile_started);
        match persistence {
            GenerationPersistence::Durable => {
                if snapshot_is_current {
                    self.mark_saved().await?;
                }
                Ok(())
            }
            GenerationPersistence::ReplacedButNotSynced(error) => Err(error),
        }
    }

    pub(super) async fn validate_generation_commit(&self, snapshot: &SaveSnapshot) -> Result<()> {
        let snapshot_pool = snapshot.db_pool.clone();
        let included =
            spawn_blocking(move || -> Result<std::collections::HashMap<u64, Vec<u8>>> {
                let connection = snapshot_pool.get()?;
                let mut statement =
                    connection.prepare("SELECT id, hash FROM media WHERE deleted = 0")?;
                let rows = statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                Ok(rows)
            })
            .await??;
        let pool = self.db_pool.clone();
        spawn_blocking(move || -> Result<()> {
            let connection = pool.get()?;
            let mut statement = connection.prepare(
                "SELECT m.id, m.hash FROM media m
                 JOIN pack_media p ON p.media_id = m.id
                 LEFT JOIN loose_media l ON l.media_id = m.id
                 WHERE l.media_id IS NULL",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, u64>(0)?, row.get::<_, Vec<u8>>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, hash) in rows {
                if included.get(&id) != Some(&hash) {
                    bail!("media {id} would lose its only source during generation commit");
                }
            }
            Ok(())
        })
        .await??;
        Ok(())
    }

    pub(super) async fn save_in_place(
        &self,
        snapshot: &SaveSnapshot,
        on_progress: Arc<dyn Fn(usize, usize) + Send + Sync>,
    ) -> Result<()> {
        // The fallback rewrites the live archive. Keep the editor fully quiescent until its
        // header, embedded database and live source map agree again.
        let _mutation_guard = self.saving.write().await;
        let _archive_guard = self.archive_io.write().await;
        let offset = self
            .write_files_from(
                snapshot.db_pool.clone(),
                None,
                false,
                snapshot.generation_id.clone(),
                Some((self.db_pool.clone(), snapshot.save_id.clone())),
                on_progress,
            )
            .await?;
        let path = self.path.as_ref().unwrap();
        let header = self.write_snapshot_tail(snapshot, path, offset).await?;
        *self.header.write().unwrap() = header;

        if self.reconcile_save_snapshot(snapshot, true).await? {
            self.mark_saved().await?;
        }
        Ok(())
    }

    pub async fn save(
        &self,
        on_progress: impl Fn(usize, usize) + Send + Sync + 'static,
    ) -> Result<()> {
        self.save_with_in_place_notification(on_progress, || {})
            .await
    }

    pub async fn save_with_in_place_notification(
        &self,
        on_progress: impl Fn(usize, usize) + Send + Sync + 'static,
        on_in_place: impl FnOnce() + Send + 'static,
    ) -> Result<()> {
        if self.closing.load(Ordering::SeqCst) {
            bail!("pack is closing");
        }
        if self.path.is_none() {
            bail!("pack has not been assigned a save location");
        }
        if self.saved.load(Ordering::Relaxed) {
            return Ok(());
        }
        let _save_guard = self.save_serial.lock().await;
        // Another queued save may already have persisted the current revision.
        if self.saved.load(Ordering::Relaxed) {
            return Ok(());
        }
        let save_started = Instant::now();
        let on_progress = Arc::new(on_progress);
        let snapshot = self.create_save_snapshot().await?;
        let initial_save_id = snapshot.save_id.clone();
        let generation_result = match self
            .prepare_generation(&snapshot, on_progress.clone())
            .await
        {
            Ok(generation) => self.commit_generation(&snapshot, generation).await,
            Err(error) => Err(error),
        };
        let result = match generation_result {
            Err(error) if is_storage_full(&error) => {
                tracing::warn!("generation save ran out of space; falling back in place");
                on_in_place();
                self.clear_save_session(&snapshot.save_id).await?;
                drop(snapshot);
                let snapshot = self.create_save_snapshot().await?;
                let result = self.save_in_place(&snapshot, on_progress).await;
                let cleanup = self.clear_save_session(&snapshot.save_id).await;
                result.and(cleanup)
            }
            result => result,
        };
        if let Err(cleanup) = self.clear_save_session(&initial_save_id).await {
            if result.is_ok() {
                return Err(cleanup);
            }
            tracing::error!("could not clear failed save session: {cleanup}");
        }
        tracing::info!(
            phase = "save_total",
            elapsed_ms = save_started.elapsed().as_secs_f64() * 1000.0,
            success = result.is_ok(),
            "save completed"
        );
        result
    }

    pub(super) async fn write_files(
        &self,
        to_path: Option<PathBuf>,
        on_progress: Arc<dyn Fn(usize, usize) + Send + Sync>,
    ) -> Result<u64> {
        self.write_files_from(
            self.db_pool.clone(),
            to_path,
            false,
            Uuid::new_v4().to_string(),
            None,
            on_progress,
        )
        .await
    }

    pub(super) async fn write_files_from(
        &self,
        db_pool: Pool<SqliteConnectionManager>,
        to_path: Option<PathBuf>,
        destination_is_clone: bool,
        generation_id: String,
        snapshot_tracking: Option<(Pool<SqliteConnectionManager>, String)>,
        on_progress: Arc<dyn Fn(usize, usize) + Send + Sync>,
    ) -> Result<u64> {
        let dir = self.dir.clone();
        let path = self.path.clone();
        spawn_blocking(move || -> Result<u64> {
            let mut conn = db_pool.get()?;
            let out_path = to_path
                .clone()
                .or_else(|| path.clone())
                .ok_or_else(|| anyhow!("pack has not been assigned a save location"))?;
            let in_place = to_path.is_none();
            let embedded = {
                let mut statement = conn.prepare(
                    "SELECT m.id, p.\"offset\", p.length FROM media m
                     JOIN pack_media p ON p.media_id = m.id
                     WHERE m.deleted = 0 ORDER BY p.\"offset\"",
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok(MediaData {
                            id: row.get("id")?,
                            offset: row.get("offset")?,
                            length: row.get("length")?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };

            let copy_every_embedded_file = to_path.is_some() && !destination_is_clone;
            let mut offset = HEADER_SIZE as u64;
            let mut embedded_offsets = Vec::with_capacity(embedded.len());
            let mut runs: Vec<CopyRun> = Vec::new();
            let mut unchanged_ids = Vec::new();
            for media in embedded {
                let destination = offset;
                embedded_offsets.push((media.id, destination));
                if copy_every_embedded_file || media.offset != destination {
                    if let Some(run) = runs.last_mut().filter(|run| {
                        !in_place
                            && run.source_offset + run.length == media.offset
                            && run.dest_offset + run.length == destination
                    }) {
                        run.length += media.length;
                        run.media_ids.push(media.id);
                    } else {
                        runs.push(CopyRun {
                            source_offset: media.offset,
                            dest_offset: destination,
                            length: media.length,
                            media_ids: vec![media.id],
                        });
                    }
                } else {
                    unchanged_ids.push(media.id);
                }
                offset += media.length;
            }

            if let Some((pool, save_id)) = &snapshot_tracking {
                release_snapshot_media(pool, save_id, &unchanged_ids)?;
            }

            let staged = {
                let mut statement = conn.prepare(
                    "SELECT m.id, l.path, l.length FROM media m
                     JOIN loose_media l ON l.media_id = m.id
                     LEFT JOIN pack_media p ON p.media_id = m.id
                     WHERE m.deleted = 0 AND p.media_id IS NULL",
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, i64>("id")?,
                            row.get::<_, String>("path")?,
                            row.get::<_, u64>("length")?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            let mut new_jobs = Vec::with_capacity(staged.len());
            for (id, media_path, expected_length) in staged {
                let full_path = dir.join("media").join(&media_path);
                match fs::metadata(&full_path) {
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        tracing::warn!(
                            "dropping staged media {id} because {} no longer exists",
                            full_path.display()
                        );
                        conn.execute("DELETE FROM media WHERE id = ?", [id])?;
                        if let Some((live_pool, _)) = &snapshot_tracking {
                            let mut live = live_pool.get()?;
                            let tx = live.transaction_with_behavior(TransactionBehavior::Immediate)?;
                            // The active item can no longer be saved, but its row and associations
                            // may still be required by an older history entry. Soft-delete it first,
                            // then physically collect it only when no history/save snapshot retains
                            // it, matching every other media GC path.
                            tx.execute(
                                "UPDATE media SET deleted = 1 WHERE id = ?
                                 AND NOT EXISTS (SELECT 1 FROM pack_media WHERE media_id = media.id)
                                 AND EXISTS (SELECT 1 FROM loose_media WHERE media_id = media.id AND path = ?)",
                                params![id, media_path],
                            )?;
                            tx.execute(
                                "DELETE FROM media WHERE id = ? AND deleted = 1
                                 AND NOT EXISTS (SELECT 1 FROM snapshot_media s WHERE s.media_id = media.id)
                                 AND NOT EXISTS (SELECT 1 FROM history_media_refs h WHERE h.media_id = media.id)",
                                [id],
                            )?;
                            tx.commit()?;
                        }
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                }
                new_jobs.push(NewFileJob {
                    id,
                    full_path,
                    dest_offset: offset,
                    expected_length,
                });
                offset += expected_length;
            }

            let total = runs.iter().map(|run| run.media_ids.len()).sum::<usize>() + new_jobs.len();
            let completed = AtomicUsize::new(0);
            if !runs.is_empty() {
                let source_path = path
                    .as_ref()
                    .ok_or_else(|| anyhow!("embedded media requires a source pack"))?;
                let copy_run = |run: &CopyRun| -> Result<()> {
                    let mut source = fs::File::open(source_path)?;
                    let mut destination = fs::OpenOptions::new().write(true).open(&out_path)?;
                    let mut buffer = vec![0; 4 * 1024 * 1024];
                    copy_range_forward(&mut source, &mut destination, run, &mut buffer)?;
                    if let Some((pool, save_id)) = &snapshot_tracking {
                        release_snapshot_media(pool, save_id, &run.media_ids)?;
                    }
                    let done = completed.fetch_add(run.media_ids.len(), Ordering::Relaxed)
                        + run.media_ids.len();
                    on_progress(done, total);
                    Ok(())
                };
                if !in_place {
                    runs.par_iter().try_for_each(copy_run)?;
                } else {
                    // In-place compaction writes earlier ranges over the same file that later
                    // files still read. Move files in source order, then establish a durability
                    // boundary before committing their new live offsets. The OS may write back
                    // any subset before sync_data, so a crash can corrupt any file in the active
                    // batch; completed batches remain durable and correctly indexed. Strictly
                    // limiting this to one file would require an archive sync and DB commit per
                    // file, which this bounded batching deliberately avoids.
                    const MAX_BATCH_FILES: usize = 64;
                    const MAX_BATCH_BYTES: u64 = 64 * 1024 * 1024;
                    let (live_pool, save_id) = snapshot_tracking.as_ref().ok_or_else(|| {
                        anyhow!("in-place compaction requires live snapshot tracking")
                    })?;
                    let mut source = fs::File::open(source_path)?;
                    let mut destination =
                        fs::OpenOptions::new().write(true).open(&out_path)?;
                    let mut buffer = vec![0; 4 * 1024 * 1024];
                    let mut batch = Vec::with_capacity(MAX_BATCH_FILES);
                    let mut batch_bytes = 0u64;
                    for run in &runs {
                        debug_assert_eq!(run.media_ids.len(), 1);
                        copy_range_forward(&mut source, &mut destination, run, &mut buffer)?;
                        batch.push((run.media_ids[0], run.dest_offset));
                        batch_bytes += run.length;
                        if batch.len() >= MAX_BATCH_FILES || batch_bytes >= MAX_BATCH_BYTES {
                            destination.sync_data()?;
                            commit_in_place_media(live_pool, save_id, &batch)?;
                            let done = completed.fetch_add(batch.len(), Ordering::Relaxed)
                                + batch.len();
                            on_progress(done, total);
                            batch.clear();
                            batch_bytes = 0;
                        }
                    }
                    if !batch.is_empty() {
                        destination.sync_data()?;
                        commit_in_place_media(live_pool, save_id, &batch)?;
                        let done =
                            completed.fetch_add(batch.len(), Ordering::Relaxed) + batch.len();
                        on_progress(done, total);
                    }
                }
            }

            fs::OpenOptions::new()
                .write(true)
                .open(&out_path)?
                .set_len(offset)?;
            // Releasing snapshot reachability opens and commits a live-database
            // transaction. Doing that once per small staged file makes SQLite the
            // bottleneck even though the byte copies themselves run in parallel.
            // Use several batches per Rayon worker so releases remain progressive
            // during long saves without paying one transaction per file.
            let target_batches = rayon::current_num_threads().saturating_mul(4).max(1);
            let target_batch_files = new_jobs.len().div_ceil(target_batches).max(1);
            const MAX_STAGED_BATCH_BYTES: u64 = 64 * 1024 * 1024;
            let mut staged_batches = Vec::new();
            let mut batch_start = 0;
            let mut batch_bytes = 0u64;
            for (index, job) in new_jobs.iter().enumerate() {
                let exceeds_file_target = index - batch_start >= target_batch_files;
                let exceeds_byte_limit = index > batch_start
                    && batch_bytes.saturating_add(job.expected_length) > MAX_STAGED_BATCH_BYTES;
                if exceeds_file_target || exceeds_byte_limit {
                    staged_batches.push((batch_start, index));
                    batch_start = index;
                    batch_bytes = 0;
                }
                batch_bytes = batch_bytes.saturating_add(job.expected_length);
            }
            if batch_start < new_jobs.len() {
                staged_batches.push((batch_start, new_jobs.len()));
            }
            let staged_started = Instant::now();
            let batch_count = AtomicUsize::new(0);
            let copy_micros = AtomicU64::new(0);
            let max_copy_micros = AtomicU64::new(0);
            let release_micros = AtomicU64::new(0);
            let max_release_micros = AtomicU64::new(0);
            let staged_bytes = new_jobs
                .iter()
                .map(|job| job.expected_length)
                .sum::<u64>();
            staged_batches
                .par_iter()
                .try_for_each(|&(start, end)| -> Result<()> {
                    let jobs = &new_jobs[start..end];
                    let batch_number = batch_count.fetch_add(1, Ordering::Relaxed) + 1;
                    let batch_bytes = jobs
                        .iter()
                        .map(|job| job.expected_length)
                        .sum::<u64>();
                    let copy_started = Instant::now();
                    let mut copied_ids = Vec::with_capacity(jobs.len());
                    // Every job in a batch writes into disjoint ranges of the same out_path,
                    // sequentially on this worker - reopening the destination per file was
                    // costing an extra open()/close() pair per staged file for no benefit.
                    let mut destination = fs::OpenOptions::new().write(true).open(&out_path)?;
                    for job in jobs {
                        let source = fs::File::open(&job.full_path).map_err(|error| {
                            anyhow!(
                                "could not open staged media {}: {error}",
                                job.full_path.display()
                            )
                        })?;
                        destination.seek(SeekFrom::Start(job.dest_offset))?;
                        let copied =
                            io::copy(&mut source.take(job.expected_length), &mut destination)?;
                        if copied != job.expected_length {
                            bail!(
                                "staged media changed size while saving: {}",
                                job.full_path.display()
                            );
                        }
                        copied_ids.push(job.id);
                    }
                    let copied_micros = copy_started.elapsed().as_micros() as u64;
                    copy_micros.fetch_add(copied_micros, Ordering::Relaxed);
                    max_copy_micros.fetch_max(copied_micros, Ordering::Relaxed);

                    let release_started = Instant::now();
                    if let Some((pool, save_id)) = &snapshot_tracking {
                        release_snapshot_media(pool, save_id, &copied_ids)?;
                    }
                    let released_micros = release_started.elapsed().as_micros() as u64;
                    release_micros.fetch_add(released_micros, Ordering::Relaxed);
                    max_release_micros.fetch_max(released_micros, Ordering::Relaxed);
                    tracing::debug!(
                        phase = "write_staged_media_batch",
                        batch_number,
                        files = jobs.len(),
                        bytes = batch_bytes,
                        copy_ms = copied_micros as f64 / 1000.0,
                        snapshot_release_ms = released_micros as f64 / 1000.0,
                        "staged-media batch completed"
                    );
                    let done = completed.fetch_add(jobs.len(), Ordering::Relaxed) + jobs.len();
                    on_progress(done, total);
                    Ok(())
                })?;
            tracing::info!(
                phase = "write_staged_media_batches",
                elapsed_ms = staged_started.elapsed().as_secs_f64() * 1000.0,
                files = new_jobs.len(),
                bytes = staged_bytes,
                batches = batch_count.load(Ordering::Relaxed),
                copy_worker_ms = copy_micros.load(Ordering::Relaxed) as f64 / 1000.0,
                max_copy_batch_ms = max_copy_micros.load(Ordering::Relaxed) as f64 / 1000.0,
                snapshot_release_worker_ms = release_micros.load(Ordering::Relaxed) as f64
                    / 1000.0,
                max_snapshot_release_batch_ms =
                    max_release_micros.load(Ordering::Relaxed) as f64 / 1000.0,
                "save phase completed"
            );

            let offsets_started = Instant::now();
            let embedded_file_count = embedded_offsets.len();
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            for (id, destination) in embedded_offsets {
                tx.execute(
                    "UPDATE pack_media SET \"offset\" = ? WHERE media_id = ?",
                    params![destination, id],
                )?;
            }
            for job in &new_jobs {
                tx.execute(
                    "INSERT INTO pack_media(media_id, generation_id, \"offset\", length)
                     VALUES (?, ?, ?, ?)
                     ON CONFLICT(media_id) DO UPDATE SET generation_id = excluded.generation_id,
                       \"offset\" = excluded.\"offset\", length = excluded.length",
                    params![job.id, generation_id, job.dest_offset, job.expected_length],
                )?;
            }
            tx.execute(
                "UPDATE editor_state SET archive_generation = ? WHERE singleton = 1",
                [&generation_id],
            )?;
            tx.commit()?;
            tracing::info!(
                phase = "record_snapshot_offsets",
                elapsed_ms = offsets_started.elapsed().as_secs_f64() * 1000.0,
                embedded_files = embedded_file_count,
                staged_files = new_jobs.len(),
                "save phase completed"
            );
            Ok(offset)
        })
        .await?
    }

    pub async fn save_as(
        &self,
        path: &Path,
        on_progress: impl Fn(usize, usize) + Send + Sync + 'static,
    ) -> Result<Option<Self>> {
        if self.closing.load(Ordering::SeqCst) {
            bail!("pack is closing");
        }
        if self.path.as_deref() == Some(path) {
            self.save(on_progress).await?;
            return Ok(None);
        }

        let _save_guard = self.save_serial.lock().await;
        let _handle = self.saving.write().await;

        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(path)
            .await?;

        let first_save = self.path.is_none();
        if self.saved.load(Ordering::Relaxed) && !first_save {
            tokio::fs::copy(self.path.as_ref().unwrap(), path).await?;
            let header = self.header.read().unwrap().make_clone();
            file.write_all(&header.to_buf()?).await?;
            file.sync_data().await?;
        } else {
            let on_progress = Arc::new(on_progress);
            let offset = self
                .write_files(Some(path.to_path_buf()), on_progress)
                .await?;

            let export = tempfile::Builder::new()
                .prefix("pack-editor-save-as-")
                .tempdir_in(&self.dir)?;
            let runtime_path = export.path().join("runtime.db");
            let pool = self.db_pool.clone();
            let runtime_for_export = runtime_path.clone();
            spawn_blocking(move || -> Result<()> {
                let connection = pool.get()?;
                editor_db::export_runtime(&connection, &runtime_for_export)
            })
            .await??;

            file.seek(SeekFrom::Start(offset)).await?;
            let index_length = {
                let mut dbf = File::open(&runtime_path).await?;
                tokio::io::copy(&mut dbf, &mut file).await?
            };

            let buf = self.metadata.read().unwrap().to_buf()?;
            let metadata_length = buf.len() as u64;
            file.write_all(&buf).await?;
            file.set_len(offset + metadata_length + index_length)
                .await?;

            let header = Header {
                id: if first_save {
                    self.header.read().unwrap().id
                } else {
                    Uuid::new_v4()
                },
                index_offset: offset,
                index_length,
                metadata_offset: offset + index_length,
                metadata_length,
            };

            file.seek(SeekFrom::Start(0)).await?;
            file.write_all(&header.to_buf()?).await?;
            file.sync_data().await?;
            self.mark_saved().await?;
        }

        let opened = Self::open_internal(path.to_path_buf(), &self.data_dir, false, true).await?;
        if first_save {
            self.saved.store(false, Ordering::Relaxed);
        }
        Ok(Some(opened))
    }

    pub async fn discard_changes(&self) -> Result<Metadata> {
        if self.saved.load(Ordering::Relaxed) {
            return Ok(self.metadata.read().unwrap().clone());
        }

        let _save_guard = self.save_serial.lock().await;
        let _handle = self.saving.write().await;
        let _archive_guard = self.archive_io.write().await;
        let mut file = self.open_read().await?;

        // Extract all header fields before any .await so Ref<Header> doesn't cross await points
        let (not_saved_yet, metadata_offset, metadata_length, index_offset, index_length) = {
            let h = self.header.read().unwrap();
            (
                h.is_default(),
                h.metadata_offset,
                h.metadata_length,
                h.index_offset,
                h.index_length,
            )
        };

        let metadata = if not_saved_yet {
            None
        } else {
            file.seek(SeekFrom::Start(metadata_offset)).await?;
            let mut buf = vec![0u8; metadata_length as usize];
            file.read_exact(&mut buf).await?;
            Some(Metadata::from_buf(&buf)?)
        };

        if not_saved_yet {
            bail!("cannot discard a pack that has never been saved");
        }
        file.seek(SeekFrom::Start(index_offset)).await?;
        let mut db_data = vec![0u8; index_length as usize];
        file.read_exact(&mut db_data).await?;

        // Restore through SQLite rather than overwriting index.db underneath the
        // live pool. In WAL mode the latter can leave existing connections tied to
        // stale pages or replay an old sidecar over the restored database.
        let staging = tempfile::Builder::new()
            .prefix("pack-editor-restore-")
            .tempdir_in(&self.dir)?;
        let restore_path = staging.path().join("index.db");
        tokio::fs::write(&restore_path, db_data).await?;
        let pool = self.db_pool.clone();
        let restored_metadata = metadata.clone().unwrap();
        spawn_blocking(move || -> Result<()> {
            let mut destination = pool.get()?;
            editor_db::replace_from_runtime(&mut destination, &restore_path, &restored_metadata)?;
            Ok(())
        })
        .await??;

        self.sweep_loose_files().await?;

        let final_meta = if let Some(m) = metadata {
            *self.metadata.write().unwrap() = m.clone();
            m
        } else {
            self.metadata.read().unwrap().clone()
        };

        self.mark_saved().await?;
        Ok(final_meta)
    }
}

pub(super) async fn process_pending_file_deletions(
    pool: Pool<SqliteConnectionManager>,
    media_dir: PathBuf,
) -> Result<()> {
    let started = Instant::now();
    let (deleted, failed, scan_time, unlink_time, queue_time) =
        spawn_blocking(move || -> Result<_> {
            let mut conn = pool.get()?;
            let scan_started = Instant::now();
            let paths = {
                let mut statement = conn.prepare("SELECT path FROM pending_file_deletions")?;
                let paths = statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                paths
            };
            let scan_time = scan_started.elapsed();
            let unlink_started = Instant::now();
            let mut deleted_paths = Vec::with_capacity(paths.len());
            let mut failed = 0;
            for stored in paths {
                let path = PathBuf::from(&stored);
                let full_path = if path.is_absolute() {
                    path
                } else {
                    media_dir.join(path)
                };
                match fs::remove_file(full_path) {
                    Ok(()) => deleted_paths.push(stored),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        deleted_paths.push(stored)
                    }
                    Err(error) => {
                        failed += 1;
                        tracing::error!("could not delete loose media {stored}: {error}");
                    }
                }
            }
            let unlink_time = unlink_started.elapsed();

            // Keep the durable queue entry until its file has actually gone. Clearing all successful
            // entries in one transaction avoids an fsync-backed SQLite commit for every loose file;
            // after a crash, already-unlinked paths are harmless and are cleared on the next pass.
            let queue_started = Instant::now();
            if !deleted_paths.is_empty() {
                let selected: Array =
                    Rc::new(deleted_paths.iter().cloned().map(Value::Text).collect());
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.execute(
                    "DELETE FROM pending_file_deletions
                 WHERE path IN (SELECT value FROM rarray(?))",
                    params![&selected],
                )?;
                tx.commit()?;
            }
            let queue_time = queue_started.elapsed();
            Ok((
                deleted_paths.len(),
                failed,
                scan_time,
                unlink_time,
                queue_time,
            ))
        })
        .await??;
    tracing::info!(
        phase = "delete_loose_files",
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        deleted,
        failed,
        scan_ms = scan_time.as_secs_f64() * 1000.0,
        unlink_ms = unlink_time.as_secs_f64() * 1000.0,
        queue_commit_ms = queue_time.as_secs_f64() * 1000.0,
        "save phase completed"
    );
    Ok(())
}

pub(super) fn release_snapshot_media(
    pool: &Pool<SqliteConnectionManager>,
    save_id: &str,
    ids: &[i64],
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let selected: Array = Rc::new(ids.iter().copied().map(Value::Integer).collect());
    let mut connection = pool.get()?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    release_snapshot_media_tx(&tx, save_id, &selected)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn release_snapshot_media_tx(
    tx: &rusqlite::Transaction<'_>,
    save_id: &str,
    selected: &Array,
) -> Result<()> {
    tx.execute(
        "DELETE FROM snapshot_media
         WHERE save_id = ?1 AND media_id IN (SELECT value FROM rarray(?2))",
        params![save_id, selected],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO pending_file_deletions(path)
         SELECT l.path FROM loose_media l JOIN media m ON m.id = l.media_id
         WHERE m.deleted = 1 AND m.id IN (SELECT value FROM rarray(?1))
           AND NOT EXISTS (SELECT 1 FROM snapshot_media s WHERE s.media_id = m.id)
           AND NOT EXISTS (SELECT 1 FROM history_media_refs h WHERE h.media_id = m.id)",
        params![selected],
    )?;
    tx.execute(
        "DELETE FROM media WHERE deleted = 1
         AND id IN (SELECT value FROM rarray(?1))
         AND NOT EXISTS (SELECT 1 FROM snapshot_media s WHERE s.media_id = media.id)
         AND NOT EXISTS (SELECT 1 FROM history_media_refs h WHERE h.media_id = media.id)",
        params![selected],
    )?;
    Ok(())
}

pub(super) fn commit_in_place_media(
    pool: &Pool<SqliteConnectionManager>,
    save_id: &str,
    offsets: &[(i64, u64)],
) -> Result<()> {
    if offsets.is_empty() {
        return Ok(());
    }
    let selected: Array = Rc::new(offsets.iter().map(|&(id, _)| Value::Integer(id)).collect());
    let mut connection = pool.get()?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for &(id, offset) in offsets {
        tx.execute(
            "UPDATE pack_media SET \"offset\" = ? WHERE media_id = ?",
            params![offset, id],
        )?;
    }
    release_snapshot_media_tx(&tx, save_id, &selected)?;
    tx.commit()?;
    Ok(())
}
