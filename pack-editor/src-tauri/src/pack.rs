use std::{
    fs::{self, create_dir_all},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, RwLock as StdRwLock,
    },
    thread::available_parallelism,
};

use anyhow::{anyhow, bail, Result};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{named_params, params, params_from_iter, OptionalExtension};
use serde::{Deserialize, Serialize};
use shared::{
    db::migrate,
    encode::{FileInfo, FileInfoParts, FileType},
    read_pack::{Header, Metadata, HEADER_SIZE},
};
use tokio::{
    fs::{remove_file, File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::RwLock,
    task::spawn_blocking,
};
use uuid::Uuid;

use crate::encode::EncodedFile;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MediaFile {
    pub id: u64,
    pub file_info: FileInfo,
    pub file_name: String,
    pub hash: String,
    pub tags: Vec<String>,
    pub size: u64,
}

struct Lock {
    file: fs::File,
    path: PathBuf,
}

impl Lock {
    fn new(path: PathBuf) -> Result<Self> {
        let file = fs::File::create(&path)?;
        file.try_lock()?;
        Ok(Self { file, path })
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if let Err(err) = self.file.unlock() {
            tracing::error!("{err}");
        }
        if let Err(err) = fs::remove_file(&self.path) {
            tracing::error!("{err}");
        }
    }
}

pub struct MediaPack {
    path: PathBuf,
    data_dir: PathBuf,
    saving: Arc<RwLock<()>>,
    _lock: Lock,
    header: StdRwLock<Header>,
    dir: PathBuf,
    metadata: StdRwLock<Metadata>,
    db_pool: Pool<SqliteConnectionManager>,
    db_path: PathBuf,
    saved: AtomicBool,
}

pub struct MediaPackView {
    path: PathBuf,
    saving: Arc<RwLock<()>>,
    dir: PathBuf,
    db_pool: Pool<SqliteConnectionManager>,
    _thread_pool: Arc<rayon::ThreadPool>,
}

#[derive(Debug)]
pub struct MediaData {
    pub id: i64,
    pub offset: u64,
    pub length: u64,
}

/// A single already-embedded file that needs to move from `source_offset` (its
/// still-valid position from the last save) to `dest_offset` (its new, compacted
/// position), both within the same pack file.
#[derive(Debug)]
struct ShiftJob {
    id: i64,
    source_offset: u64,
    dest_offset: u64,
    length: u64,
}

/// A newly-staged file (still a loose file under `dir/media/`) that needs
/// appending into the pack file at `dest_offset`. `expected_length` comes from
/// the DB (recorded when the file was staged) and is used to size the file
/// upfront via `set_len`, bounding this job's copy so it can never overrun into
/// a neighboring job's region even if the on-disk file were somehow larger.
#[derive(Debug)]
struct NewFileJob {
    id: i64,
    full_path: PathBuf,
    dest_offset: u64,
    expected_length: u64,
}

/// The write requested by a single completed copy job. Sent to the dedicated DB
/// writer thread rather than executed by the worker itself, so all writes for a
/// save go through one connection with zero contention.
enum DbUpdateKind {
    Shift { id: i64, offset: u64 },
    NewFile { id: i64, offset: u64, length: u64 },
    DropMissing { id: i64 },
}

struct DbUpdateRequest {
    kind: DbUpdateKind,
    ack: std::sync::mpsc::Sender<Result<()>>,
}

/// Runs on a single dedicated thread for the duration of one `write_files` call,
/// draining update requests from parallel copy workers and applying them one at a
/// time - so those workers never contend with each other for SQLite's single
/// write lock. Exits once `rx` is closed (all senders dropped) and everything
/// already sent has been drained.
fn run_db_writer(
    conn: PooledConnection<SqliteConnectionManager>,
    rx: std::sync::mpsc::Receiver<DbUpdateRequest>,
) {
    while let Ok(req) = rx.recv() {
        let result = (|| -> Result<()> {
            match req.kind {
                DbUpdateKind::Shift { id, offset } => {
                    conn.execute(
                        "UPDATE media SET offset = ? WHERE id = ?",
                        params![offset, id],
                    )?;
                }
                DbUpdateKind::NewFile { id, offset, length } => {
                    conn.execute(
                        "UPDATE media SET offset = ?, length = ?, path = NULL WHERE id = ?",
                        params![offset, length, id],
                    )?;
                }
                DbUpdateKind::DropMissing { id } => {
                    conn.execute("DELETE FROM media WHERE id = ?", params![id])?;
                }
            }
            Ok(())
        })();
        // The requester always waits for this ack, so a send failure here would
        // only mean it gave up some other way (e.g. panicked) - nothing to do.
        let _ = req.ack.send(result);
    }
}

/// Sends one update to the DB writer thread and blocks until it's been applied
/// and acknowledged - so callers only consider a file "done" once its row is
/// actually durable, matching the immediate-per-file write the sequential code
/// used to do directly.
fn send_db_update(db_tx: &std::sync::mpsc::Sender<DbUpdateRequest>, kind: DbUpdateKind) -> Result<()> {
    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
    db_tx
        .send(DbUpdateRequest { kind, ack: ack_tx })
        .map_err(|_| anyhow!("db writer thread is gone"))?;
    ack_rx.recv().map_err(|_| anyhow!("db writer thread is gone"))?
}

pub enum FileData {
    Path(PathBuf),
    Data(Vec<u8>),
}

pub struct DataRange {
    pub data: Vec<u8>,
    pub start: u64,
    pub end: u64,
    pub total_size: u64,
}

pub struct Range {
    pub start: Option<u64>,
    pub end: Option<u64>,
}

impl MediaPack {
    pub async fn new(path: PathBuf, data_dir: &Path, name: &str) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .await?;

        let lock_path = path.with_added_extension("lock");
        let lock = Lock::new(lock_path)?;
        let header = Header::new();

        file.write_all(&header.to_buf()?).await?;

        let metadata = Metadata {
            name: name.to_string(),
            ..Metadata::default()
        };

        let dir = data_dir
            .join("Lewdware Pack Editor")
            .join(header.id.to_string());

        create_dir_all(&dir)?;
        create_dir_all(&dir.join("media"))?;

        let metadata_path = dir.join("Metadata");
        File::create(&metadata_path)
            .await?
            .write_all(&metadata.to_buf()?)
            .await?;

        let db_path = dir.join("index.db");
        let manager = SqliteConnectionManager::file(&db_path);
        let db_pool = Pool::builder().build(manager)?;

        let pool = db_pool.clone();
        spawn_blocking(move || -> Result<_> {
            let conn = pool.get()?;
            migrate(&conn)
        })
        .await??;

        File::create(dir.join("UNSAVED")).await?;

        Ok(Self {
            path,
            data_dir: data_dir.to_path_buf(),
            saving: Arc::new(RwLock::new(())),
            _lock: lock,
            header: StdRwLock::new(header),
            dir,
            metadata: StdRwLock::new(metadata),
            db_pool,
            saved: AtomicBool::new(false),
            db_path,
        })
    }

    pub async fn open(path: PathBuf, data_dir: &Path) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .await?;

        let lock_path = path.with_added_extension("lock");
        let lock = Lock::new(lock_path)?;

        let mut buf = [0u8; HEADER_SIZE];
        file.read_exact(&mut buf).await?;
        let header = Header::from_buf(buf)?;

        let dir = data_dir
            .join("Lewdware Pack Editor")
            .join(header.id.to_string());

        create_dir_all(&dir)?;
        create_dir_all(&dir.join("media"))?;

        let db_path = dir.join("index.db");
        let has_unsaved = fs::exists(dir.join("UNSAVED"))? && fs::exists(&db_path)?;

        let metadata = if has_unsaved {
            let metadata_path = dir.join("Metadata");
            fs::read(metadata_path)
                .map_err(|err| anyhow!(err))
                .and_then(|buf| Metadata::from_buf(&buf).map_err(|err| err.into()))
                .unwrap_or_default()
        } else {
            file.seek(SeekFrom::Start(header.metadata_offset)).await?;
            let mut buf = vec![0u8; header.metadata_length as usize];
            file.read_exact(&mut buf).await?;
            Metadata::from_buf(&buf)?
        };

        if !has_unsaved {
            file.seek(SeekFrom::Start(header.index_offset)).await?;
            let mut db_data = vec![0u8; header.index_length as usize];
            file.read_exact(&mut db_data).await?;

            let mut db_file = File::create(&db_path).await?;
            db_file.write_all(&db_data).await?;
            db_file.flush().await?;
        }

        let manager = SqliteConnectionManager::file(&db_path);
        // Sized to comfortably cover the parallel-copy worker count in write_files,
        // so workers grabbing a connection to record their offset never queue on
        // the pool itself (they may still briefly serialize on SQLite's own write
        // lock, which rusqlite's default 5s busy_timeout already handles).
        let pool_size = available_parallelism().map(|n| n.get() as u32).unwrap_or(4);
        let db_pool = Pool::builder().max_size(pool_size).build(manager)?;

        let pool = db_pool.clone();
        spawn_blocking(move || -> Result<_> {
            let conn = pool.get()?;
            migrate(&conn)
        })
        .await??;

        Ok(Self {
            path,
            data_dir: data_dir.to_path_buf(),
            saving: Arc::new(RwLock::new(())),
            _lock: lock,
            header: StdRwLock::new(header),
            dir,
            metadata: StdRwLock::new(metadata),
            db_pool,
            saved: AtomicBool::new(!has_unsaved),
            db_path,
        })
    }

    pub fn get_view(&self) -> Result<MediaPackView> {
        let threads = (available_parallelism()?.get() / 2).max(1);
        Ok(MediaPackView {
            path: self.path.clone(),
            saving: self.saving.clone(),
            dir: self.dir.clone(),
            db_pool: self.db_pool.clone(),
            _thread_pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()?,
            ),
        })
    }

    pub fn name(&self) -> String {
        self.metadata.read().unwrap().name.clone()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub async fn is_saved(&self) -> bool {
        let _handle = self.saving.write().await;
        self.saved.load(Ordering::Relaxed)
    }

    pub async fn mark_unsaved(&self) -> Result<()> {
        if self.saved.load(Ordering::Relaxed) {
            File::create(self.dir.join("UNSAVED")).await?;
            self.saved.store(false, Ordering::Relaxed);
        }
        Ok(())
    }

    async fn mark_saved(&self) -> Result<()> {
        if !self.saved.load(Ordering::Relaxed) {
            if let Err(err) = remove_file(self.dir.join("UNSAVED")).await {
                if err.kind() != io::ErrorKind::NotFound {
                    return Err(err.into());
                }
            }
            self.saved.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    async fn db_execute<T, F>(&self, mut f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnMut(PooledConnection<SqliteConnectionManager>) -> Result<T> + Send + 'static,
    {
        let pool = self.db_pool.clone();
        spawn_blocking(move || {
            let conn = pool.get()?;
            f(conn)
        })
        .await?
    }

    async fn open_read(&self) -> io::Result<File> {
        OpenOptions::new().read(true).open(&self.path).await
    }

    async fn open_write(&self) -> io::Result<File> {
        OpenOptions::new().write(true).open(&self.path).await
    }

    pub async fn save(
        &self,
        on_progress: impl Fn(usize, usize) + Send + Sync + 'static,
    ) -> Result<()> {
        if self.saved.load(Ordering::Relaxed) {
            return Ok(());
        }
        let _handle = self.saving.write().await;
        let on_progress = Arc::new(on_progress);

        tracing::warn!("Writing files");

        let offset = self.write_files(None, on_progress).await?;

        tracing::warn!("Finished writing files");

        self.db_execute(|conn| conn.execute("VACUUM", []).map_err(|err| err.into()))
            .await?;

        let mut file = self.open_write().await?;
        file.seek(SeekFrom::Start(offset)).await?;

        let index_length = {
            let mut dbf = File::open(&self.db_path).await?;
            tokio::io::copy(&mut dbf, &mut file).await?
        };

        let buf = self.metadata.read().unwrap().to_buf()?;
        let metadata_length = buf.len() as u64;
        file.write_all(&buf).await?;
        file.set_len(offset + metadata_length + index_length)
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
        *self.header.write().unwrap() = header;
        file.sync_data().await?;

        self.clean_media()?;
        self.mark_saved().await?;
        Ok(())
    }

    async fn write_files(
        &self,
        to_path: Option<PathBuf>,
        on_progress: Arc<dyn Fn(usize, usize) + Send + Sync>,
    ) -> Result<u64> {
        let dir = self.dir.clone();
        let path = self.path.clone();
        let db_pool = self.db_pool.clone();

        self.db_execute(move |conn| {
            let out_path = to_path.clone().unwrap_or_else(|| path.clone());

            let mut num_files: usize =
                conn.query_row_and_then("SELECT COUNT(*) as files FROM media", params![], |row| {
                    row.get("files")
                })?;

            let mut offset = HEADER_SIZE as u64;

            let mut get_stmt = conn.prepare(
                "SELECT id, offset, length FROM media WHERE offset IS NOT NULL ORDER BY offset",
            )?;

            let mut media = get_stmt
                .query_map(params![], |row| {
                    Ok(MediaData {
                        id: row.get("id")?,
                        offset: row.get("offset")?,
                        length: row.get("length")?,
                    })
                })?
                .peekable();

            if to_path.is_none() {
                while media
                    .next_if(|x| {
                        x.as_ref().is_ok_and(|d| {
                            if d.offset == offset {
                                offset += d.length;
                                true
                            } else {
                                false
                            }
                        })
                    })
                    .is_some()
                {
                    num_files -= 1;
                }
            }

            // Shared across both phases below: on_progress's denominator, which can
            // still shrink in the second phase if a staged file turns out to be
            // missing on disk.
            let num_files = AtomicUsize::new(num_files);

            // Precompute each remaining file's compacted destination alongside its
            // still-valid original (source) range, before moving any bytes. This is
            // pure arithmetic over DB rows - no file I/O yet.
            let mut jobs = Vec::new();
            for media_result in media {
                let media_data = media_result?;
                jobs.push(ShiftJob {
                    id: media_data.id,
                    source_offset: media_data.offset,
                    dest_offset: offset,
                    length: media_data.length,
                });
                offset += media_data.length;
            }

            // All DB writes for both phases below go through a single dedicated
            // writer thread instead of each worker grabbing its own pooled
            // connection. SQLite only ever allows one writer at a time regardless,
            // so N workers racing for that lock via busy-timeout retries doesn't
            // buy any real parallelism - it only adds contention, and under
            // sustained load from thousands of rapid small writes that contention
            // can genuinely exceed the busy_timeout and surface as "database is
            // locked". A single writer has zero contention by construction. Workers
            // still wait for an ack before considering their file done, so a file
            // is never reported/removed until its DB row is actually durable -
            // same per-file crash-safety guarantee as before, just funneled through
            // one thread instead of one connection per worker.
            let (db_tx, db_rx) = std::sync::mpsc::channel::<DbUpdateRequest>();
            let writer_conn = db_pool.get()?;
            let writer_handle = std::thread::spawn(move || run_db_writer(writer_conn, db_rx));

            let result: Result<()> = (|| {
                // Run the shifts in parallel. For an in-place save, `path` and
                // `out_path` are the same file, so a job's write can only start once
                // no other in-flight job still needs to read from the range it's
                // about to overwrite - `in_flight` + `cvar` gate that. Checking
                // against actual in-flight jobs (rather than a precomputed "depends
                // on job N" shortcut) is deliberate: the safe set of predecessors a
                // job depends on isn't always just its immediate predecessor, so
                // anything less than a live check risks a job overwriting data
                // another job hasn't read yet.
                //
                // Registration into `in_flight` happens here, on this single
                // coordinating thread, strictly in job order, *before* the job is
                // handed to a worker - not inside the worker itself. rayon's
                // work-stealing scheduler doesn't run spawned tasks in submission
                // order, so if each worker registered itself only once it actually
                // started, a later job could start (and see an empty/incomplete
                // `in_flight`) before an earlier job it truly conflicts with had
                // even begun. Registering on the coordinator, in order, guarantees
                // that by the time job j is considered, every earlier job is
                // already accounted for in `in_flight` (still running) or has
                // already been removed from it (its read completed).
                let saved_count = AtomicUsize::new(0);
                let in_flight: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());
                let cvar = Condvar::new();
                let errors: Mutex<Vec<anyhow::Error>> = Mutex::new(Vec::new());

                rayon::scope(|scope| {
                    for job in &jobs {
                        {
                            let mut guard = in_flight.lock().unwrap();
                            loop {
                                let overlap = guard.iter().any(|&(src_off, src_len)| {
                                    job.dest_offset < src_off + src_len
                                        && job.dest_offset + job.length > src_off
                                });
                                if !overlap {
                                    guard.push((job.source_offset, job.length));
                                    break;
                                }
                                guard = cvar.wait(guard).unwrap();
                            }
                        }

                        let path = &path;
                        let out_path = &out_path;
                        let in_flight = &in_flight;
                        let cvar = &cvar;
                        let db_tx = &db_tx;
                        let saved_count = &saved_count;
                        let on_progress = &on_progress;
                        let errors = &errors;
                        let num_files = &num_files;
                        scope.spawn(move |_| {
                            match copy_shift_job(job, path, out_path, in_flight, cvar, db_tx) {
                                Ok(()) => {
                                    let n = saved_count.fetch_add(1, Ordering::SeqCst) + 1;
                                    on_progress(n, num_files.load(Ordering::SeqCst));
                                }
                                Err(err) => errors.lock().unwrap().push(err),
                            }
                        });
                    }
                });

                if let Some(err) = errors.into_inner().unwrap().into_iter().next() {
                    return Err(err);
                }

                // Newly-staged files: unlike the shift jobs above, these each read
                // from their own separate loose file under dir/media/, so there's no
                // self-overlap risk at all and no in-flight gating is needed - only
                // the destination ranges need to stay disjoint, which the
                // precomputed cumulative offsets below already guarantee.
                let mut get_stmt =
                    conn.prepare("SELECT id, path, length FROM media WHERE path IS NOT NULL")?;
                let media = get_stmt.query_map(params![], |row| {
                    Ok((
                        row.get::<_, i64>("id")?,
                        row.get::<_, String>("path")?,
                        row.get::<_, Option<u64>>("length")?,
                    ))
                })?;

                let mut new_jobs = Vec::new();
                for media_result in media {
                    let (id, media_path, length) = media_result?;
                    let full_path = dir.join("media").join(&media_path);
                    // Older staged files from before file size was recorded at
                    // upload time won't have a `length` yet - fall back to a stat.
                    let expected_length = match length {
                        Some(l) => l,
                        None => fs::metadata(&full_path)?.len(),
                    };
                    new_jobs.push(NewFileJob {
                        id,
                        full_path,
                        dest_offset: offset,
                        expected_length,
                    });
                    offset += expected_length;
                }

                // Pre-size the file to its final length in one call, rather than
                // letting each parallel write extend it individually - avoids the
                // (brief, but serializing) inode-extension locking every OS does for
                // writes that grow a file, and keeps every write below the same kind
                // of "write into an already-sized region" operation as the shifts
                // above.
                fs::OpenOptions::new()
                    .write(true)
                    .open(&out_path)?
                    .set_len(offset)?;

                let saved_count = AtomicUsize::new(0);
                let errors: Mutex<Vec<anyhow::Error>> = Mutex::new(Vec::new());

                rayon::scope(|scope| {
                    for job in &new_jobs {
                        let out_path = &out_path;
                        let db_tx = &db_tx;
                        let saved_count = &saved_count;
                        let on_progress = &on_progress;
                        let errors = &errors;
                        let num_files = &num_files;
                        scope.spawn(move |_| {
                            match copy_new_file_job(job, out_path, db_tx) {
                                Ok(true) => {
                                    let n = saved_count.fetch_add(1, Ordering::SeqCst) + 1;
                                    on_progress(n, num_files.load(Ordering::SeqCst));
                                }
                                Ok(false) => {
                                    // Staged file was missing; already dropped from
                                    // the DB and the progress denominator.
                                    num_files.fetch_sub(1, Ordering::SeqCst);
                                }
                                Err(err) => errors.lock().unwrap().push(err),
                            }
                        });
                    }
                });

                if let Some(err) = errors.into_inner().unwrap().into_iter().next() {
                    return Err(err);
                }

                Ok(())
            })();

            // Closing the channel (by dropping the sender) lets the writer thread's
            // recv loop end once it's drained everything already sent; join it
            // before returning either way so we never leave it running detached.
            drop(db_tx);
            if writer_handle.join().is_err() {
                tracing::error!("db writer thread panicked");
            }

            result?;
            Ok(offset)
        })
        .await
    }

    pub async fn save_as(
        &self,
        path: &Path,
        on_progress: impl Fn(usize, usize) + Send + Sync + 'static,
    ) -> Result<Option<Self>> {
        if path == &self.path {
            self.save(on_progress).await?;
            return Ok(None);
        }

        let _handle = self.saving.write().await;

        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(path)
            .await?;

        if self.saved.load(Ordering::Relaxed) {
            tokio::fs::copy(&self.path, path).await?;
            let header = self.header.read().unwrap().make_clone();
            file.write_all(&header.to_buf()?).await?;
            file.sync_data().await?;
        } else {
            let on_progress = Arc::new(on_progress);
            let offset = self
                .write_files(Some(path.to_path_buf()), on_progress)
                .await?;

            self.db_execute(|conn| conn.execute("VACUUM", []).map_err(|err| err.into()))
                .await?;

            file.seek(SeekFrom::Start(offset)).await?;
            let index_length = {
                let mut dbf = File::open(&self.db_path).await?;
                tokio::io::copy(&mut dbf, &mut file).await?
            };

            let buf = self.metadata.read().unwrap().to_buf()?;
            let metadata_length = buf.len() as u64;
            file.write_all(&buf).await?;
            file.set_len(offset + metadata_length + index_length)
                .await?;

            let header = Header {
                id: Uuid::new_v4(),
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

        Ok(Some(Self::open(path.to_path_buf(), &self.data_dir).await?))
    }

    pub async fn discard_changes(&self) -> Result<Metadata> {
        if self.saved.load(Ordering::Relaxed) {
            return Ok(self.metadata.read().unwrap().clone());
        }

        let _handle = self.saving.write().await;
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

        file.seek(SeekFrom::Start(index_offset)).await?;
        let mut db_data = vec![0u8; index_length as usize];
        file.read_exact(&mut db_data).await?;

        let mut db_file = File::create(&self.db_path).await?;
        db_file.write_all(&db_data).await?;
        db_file.flush().await?;

        self.clean_media()?;

        let final_meta = if let Some(m) = metadata {
            *self.metadata.write().unwrap() = m.clone();
            m
        } else {
            self.metadata.read().unwrap().clone()
        };

        self.mark_saved().await?;
        Ok(final_meta)
    }

    pub fn metadata(&self) -> Metadata {
        self.metadata.read().unwrap().clone()
    }

    pub async fn set_metadata(&self, metadata: &Metadata) -> Result<()> {
        let _handle = self.saving.read().await;
        *self.metadata.write().unwrap() = metadata.clone();
        self.mark_unsaved().await
    }

    pub async fn save_metadata(&self) -> Result<()> {
        let temp_path = self.dir.join("Metadata.copy");
        let final_path = self.dir.join("Metadata");
        let data = self.metadata.read().unwrap().to_buf()?;
        tokio::fs::File::create(&temp_path)
            .await?
            .write_all(&data)
            .await?;
        fs::rename(temp_path, final_path)?;
        Ok(())
    }

    fn clean_media(&self) -> Result<()> {
        for entry in fs::read_dir(self.dir.join("media"))? {
            if let Err(err) = entry.and_then(|e| fs::remove_file(e.path())) {
                tracing::error!("{err}");
            }
        }
        Ok(())
    }

    /// Returns `Ok(None)` if `hash` is already present - a DB-level `UNIQUE`
    /// constraint on `media.hash`, not just the caller's own pre-check, so this
    /// is safe even when two uploads with identical content race each other (the
    /// pre-check alone can't catch that: both can see "not present yet" before
    /// either has inserted its row - the constraint is what actually closes it).
    pub async fn add_file(
        &self,
        encoded_file: EncodedFile,
        path: &Path,
        hash: blake3::Hash,
    ) -> Result<Option<MediaFile>> {
        let _handle = self.saving.read().await;

        let FileInfoParts {
            file_type,
            width,
            height,
            transparent,
            duration,
            audio,
        } = encoded_file.info.to_parts();

        let file_name = file_name(path);
        let file_path = encoded_file.path.to_string_lossy().to_string();
        let file_info = encoded_file.info.clone();
        let file_name_clone = file_name.clone();
        let hash_bytes = *hash.as_bytes();
        let size = tokio::fs::metadata(&encoded_file.path).await?.len();

        let insert_result = self
            .db_execute(move |conn| {
                conn.query_row(
                    "INSERT INTO media (file_name, file_type, path, length, width, height, transparent, duration, audio, hash, thumbnail)
                    VALUES (:file_name, :file_type, :path, :length, :width, :height, :transparent, :duration, :audio, :hash, :thumbnail) RETURNING id",
                    named_params! {
                        ":file_name": file_name_clone,
                        ":file_type": file_type.as_str(),
                        ":path": file_path,
                        ":length": size,
                        ":width": width,
                        ":height": height,
                        ":transparent": transparent,
                        ":duration": duration,
                        ":audio": audio,
                        ":hash": hash_bytes,
                        ":thumbnail": encoded_file.thumbnail,
                    },
                    |row| row.get::<_, u64>("id"),
                )
                .map_err(|err| err.into())
            })
            .await;

        let id = match insert_result {
            Ok(id) => id,
            Err(err) if is_unique_violation(&err) => return Ok(None),
            Err(err) => return Err(err),
        };

        self.mark_unsaved().await?;

        Ok(Some(MediaFile {
            id,
            file_name,
            file_info,
            hash: hash.to_string(),
            tags: vec![],
            size,
        }))
    }

    pub async fn remove_files(&self, ids: Vec<u64>) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            conn.execute(
                &format!("DELETE FROM media WHERE id IN ({})", repeat_vars(ids.len())),
                params_from_iter(&ids),
            )?;
            Ok(())
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn get_files(&self) -> Result<Vec<MediaFile>> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, file_type, file_name, width, height, transparent, duration, audio, hash, length FROM media",
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
                        size: row.get::<_, Option<u64>>("length")?.unwrap_or(0),
                    })
                })?;
                rows.collect::<Result<Vec<_>>>()?
            };

            // Build id → index map then load all tag associations in one query.
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

            Ok(files)
        })
        .await
    }

    pub async fn get_all_tags(&self) -> Result<Vec<String>> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare("SELECT name FROM tags")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>("name"))?;
            rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
        })
        .await
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
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            let tag_id: u64 =
                conn.query_row("SELECT id FROM tags WHERE name = ?", params![tag], |row| {
                    row.get("id")
                })?;
            conn.execute(
                "INSERT INTO media_tags (media_id, tag_id) VALUES (?, ?)",
                params![id, tag_id],
            )?;
            Ok(())
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn create_and_add_tag(&self, id: u64, tag: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut conn| {
            let tx = conn.transaction()?;
            let tag_id: u64 = tx.query_row(
                "INSERT INTO tags (name) VALUES (?) RETURNING id",
                params![tag],
                |row| row.get("id"),
            )?;
            tx.execute(
                "INSERT INTO media_tags (media_id, tag_id) VALUES (?, ?)",
                params![id, tag_id],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn remove_tag(&self, id: u64, tag: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            conn.execute(
                "DELETE FROM media_tags WHERE media_id = ? AND tag_id IN (SELECT id FROM tags WHERE name = ?)",
                params![id, tag],
            )?;
            Ok(())
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn check_hash(&self, hash: &blake3::Hash) -> Result<bool> {
        let hash_bytes = *hash.as_bytes();
        self.db_execute(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT 1 FROM media WHERE hash = ?",
                    params![hash_bytes],
                    |_| Ok(1),
                )
                .optional()?
                .is_some())
        })
        .await
    }

    pub async fn set_title(&self, id: u64, name: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            conn.execute(
                "UPDATE media SET file_name = ? WHERE id = ?",
                params![name, id],
            )?;
            Ok(())
        })
        .await?;
        self.mark_unsaved().await
    }
}

impl Drop for MediaPack {
    fn drop(&mut self) {
        if self.saved.load(Ordering::Relaxed) {
            if let Err(err) = fs::remove_dir_all(&self.dir) {
                tracing::error!("{err}");
            }
        }
    }
}

impl MediaPackView {
    async fn db_execute<T, F>(&self, mut f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnMut(PooledConnection<SqliteConnectionManager>) -> Result<T> + Send + 'static,
    {
        let pool = self.db_pool.clone();
        spawn_blocking(move || {
            let conn = pool.get()?;
            f(conn)
        })
        .await?
    }

    async fn open_read(&self) -> io::Result<File> {
        OpenOptions::new().read(true).open(&self.path).await
    }

    async fn get_raw_file(&self, id: u64) -> Result<(FileData, FileType, bool)> {
        let (offset, length, path, file_type, transparent) = self
            .db_execute(move |conn| {
                conn.query_row_and_then(
                    "SELECT offset, length, path, file_type, transparent FROM media WHERE id = ?",
                    params![id],
                    |row| -> Result<_> {
                        Ok((
                            row.get::<_, Option<u64>>("offset")?,
                            row.get::<_, Option<usize>>("length")?,
                            row.get::<_, Option<String>>("path")?,
                            row.get::<_, String>("file_type")?.parse()?,
                            row.get::<_, Option<bool>>("transparent")?.unwrap_or(false),
                        ))
                    },
                )
                .map_err(Into::into)
            })
            .await?;

        let mut file = self.open_read().await?;

        let file_data = match (offset, length, path) {
            (Some(offset), Some(length), _) => {
                file.seek(SeekFrom::Start(offset)).await?;
                let mut buf = vec![0u8; length];
                file.read_exact(&mut buf).await?;
                FileData::Data(buf)
            }
            (_, _, Some(path)) => FileData::Path(self.dir.join("media").join(path)),
            _ => bail!("No offset, length or path"),
        };

        Ok((file_data, file_type, transparent))
    }

    pub async fn get_thumbnail(&self, id: u64) -> Result<Vec<u8>> {
        self.db_execute(move |conn| {
            conn.query_row("SELECT thumbnail FROM media WHERE id = ?", [id], |row| {
                row.get("thumbnail")
            })
            .map_err(Into::into)
        })
        .await
    }

    pub async fn get_preview(&self, id: u64) -> Result<Vec<u8>> {
        let _handle = self.saving.read().await;
        let (file_data, file_type, transparent) = self.get_raw_file(id).await?;
        crate::thumbnail::generate_preview(file_data, file_type == FileType::Image, transparent)
            .await
    }

    pub async fn get_display(&self, id: u64) -> Result<Vec<u8>> {
        let _handle = self.saving.read().await;
        let (file_data, _, _) = self.get_raw_file(id).await?;
        crate::thumbnail::generate_display_image(file_data).await
    }

    pub async fn get_file_data(&self, id: u64) -> Result<(Vec<u8>, FileType)> {
        let _handle = self.saving.read().await;
        let (file_data, file_type, _) = self.get_raw_file(id).await?;
        let data = match file_data {
            FileData::Path(path) => tokio::fs::read(path).await?,
            FileData::Data(data) => data,
        };
        Ok((data, file_type))
    }

    pub async fn get_file_range(&self, id: u64, range: Range) -> Result<(DataRange, FileType)> {
        let _handle = self.saving.read().await;

        let (offset, length, path, file_type) = self
            .db_execute(move |conn| {
                conn.query_row_and_then(
                    "SELECT offset, length, path, file_type FROM media WHERE id = ?",
                    params![id],
                    |row| -> Result<_> {
                        Ok((
                            row.get::<_, Option<u64>>("offset")?,
                            row.get::<_, Option<u64>>("length")?,
                            row.get::<_, Option<String>>("path")?,
                            row.get::<_, String>("file_type")?.parse()?,
                        ))
                    },
                )
                .map_err(Into::into)
            })
            .await?;

        let mut file = self.open_read().await?;

        let data_range = match (offset, length, path) {
            (Some(offset), Some(length), _) => {
                let (start, end) = resolve_range(range, length)?;
                file.seek(SeekFrom::Start(offset + start)).await?;
                let mut buf = vec![0u8; (end - start) as usize];
                file.read_exact(&mut buf).await?;
                DataRange {
                    data: buf,
                    start,
                    end,
                    total_size: length,
                }
            }
            (_, _, Some(path)) => {
                let path = self.dir.join("media").join(path);
                let mut file = tokio::fs::File::open(&path).await?;
                let size = file.metadata().await?.len();
                let (start, end) = resolve_range(range, size)?;
                file.seek(SeekFrom::Start(start)).await?;
                let mut buf = vec![0u8; (end - start) as usize];
                file.read_exact(&mut buf).await?;
                DataRange {
                    data: buf,
                    start,
                    end,
                    total_size: size,
                }
            }
            _ => bail!("No offset, length or path"),
        };

        Ok((data_range, file_type))
    }
}

/// Copies one already-embedded file from `job.source_offset` to `job.dest_offset`
/// within `path`/`out_path` (the same file for an in-place save), then records the
/// new offset in the DB immediately - so a crash mid-save never leaves the DB
/// pointing at a range whose original bytes may have already been overwritten by
/// another job's write.
///
/// Callers must have already registered `(job.source_offset, job.length)` in
/// `in_flight` (see the dispatch loop in `write_files`) before spawning this -
/// registration must happen on a single thread, strictly in job order, to be a
/// valid safety gate. This function only removes the entry once the read is done.
fn copy_shift_job(
    job: &ShiftJob,
    path: &Path,
    out_path: &Path,
    in_flight: &Mutex<Vec<(u64, u64)>>,
    cvar: &Condvar,
    db_tx: &std::sync::mpsc::Sender<DbUpdateRequest>,
) -> Result<()> {
    let copy_result = (|| -> Result<()> {
        let mut in_file = fs::File::open(path)?;
        in_file.seek(SeekFrom::Start(job.source_offset))?;
        let mut bounded = in_file.take(job.length);
        let mut out_file = fs::OpenOptions::new().write(true).open(out_path)?;
        out_file.seek(SeekFrom::Start(job.dest_offset))?;
        io::copy(&mut bounded, &mut out_file)?;
        Ok(())
    })();

    // The read is done (or failed) either way, so this job's source range no
    // longer needs protecting from concurrent writers.
    {
        let mut guard = in_flight.lock().unwrap();
        guard.retain(|&(src_off, _)| src_off != job.source_offset);
        cvar.notify_all();
    }

    copy_result?;

    send_db_update(
        db_tx,
        DbUpdateKind::Shift {
            id: job.id,
            offset: job.dest_offset,
        },
    )
}

/// Copies one newly-staged loose file into `out_path` at `job.dest_offset`, then
/// records its offset/length in the DB and only then removes the staging file -
/// in that order, so a crash never leaves a row with neither a valid pack offset
/// nor its staging copy. No overlap gating is needed here: unlike
/// `copy_shift_job`, the source is always a separate file from `out_path`, so
/// concurrent jobs can never race on the same bytes.
///
/// Returns `Ok(false)` (not an error) if the staged file was missing on disk -
/// its DB row has already been dropped in that case, and the caller is expected
/// to adjust the progress denominator accordingly.
fn copy_new_file_job(
    job: &NewFileJob,
    out_path: &Path,
    db_tx: &std::sync::mpsc::Sender<DbUpdateRequest>,
) -> Result<bool> {
    let media_file = match fs::File::open(&job.full_path) {
        Ok(f) => f,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            tracing::error!(
                "Staged file missing, dropping from pack: {}",
                job.full_path.display()
            );
            send_db_update(db_tx, DbUpdateKind::DropMissing { id: job.id })?;
            return Ok(false);
        }
        Err(err) => return Err(err.into()),
    };

    let mut out_file = fs::OpenOptions::new().write(true).open(out_path)?;
    out_file.seek(SeekFrom::Start(job.dest_offset))?;
    // Bounded to expected_length so an unexpectedly-larger on-disk file can never
    // overrun into the next job's precomputed (and, since we pre-sized the file,
    // already-allocated) region.
    let mut bounded = media_file.take(job.expected_length);
    let size = io::copy(&mut bounded, &mut out_file)?;

    send_db_update(
        db_tx,
        DbUpdateKind::NewFile {
            id: job.id,
            offset: job.dest_offset,
            length: size,
        },
    )?;

    if let Err(err) = fs::remove_file(&job.full_path) {
        tracing::error!("{err}");
    }

    Ok(true)
}

/// Whether `err` (as returned by a query through `db_execute`, which wraps the
/// underlying `rusqlite::Error` in an `anyhow::Error`) is a `UNIQUE` constraint
/// violation - in this schema, that only ever means `media.hash` already exists.
fn is_unique_violation(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::ConstraintViolation,
                ..
            },
            _
        ))
    )
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .to_string()
}

fn repeat_vars(count: usize) -> String {
    assert_ne!(count, 0);
    let mut s = "?,".repeat(count);
    s.pop();
    s
}

fn resolve_range(range: Range, size: u64) -> Result<(u64, u64)> {
    match (range.start, range.end) {
        (Some(start), Some(end)) => Ok((start, (end + 1).min(size))),
        (Some(start), None) => Ok((start, size)),
        _ => bail!("Invalid range"),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use shared::read_pack::Metadata;
    use tempfile::tempdir;

    use super::*;

    async fn new_test_pack(pack_path: &Path, data_dir: &Path, name: &str) -> MediaPack {
        MediaPack::new(pack_path.to_path_buf(), data_dir, name)
            .await
            .unwrap()
    }

    // Insert a minimal audio row backed by a staging file in dir/media/.
    // Returns the assigned media id.
    async fn insert_staged_audio(pack: &MediaPack, content: &[u8]) -> u64 {
        let filename = Uuid::new_v4().to_string();
        tokio::fs::write(pack.dir.join("media").join(&filename), content)
            .await
            .unwrap();

        let hash_bytes = *blake3::hash(content).as_bytes();
        pack.db_execute(move |conn| {
            let id: u64 = conn.query_row(
                "INSERT INTO media (file_name, file_type, path, duration, hash) \
                 VALUES ('test.wav', 'audio', ?, 1.0, ?) RETURNING id",
                params![filename, hash_bytes],
                |row| row.get("id"),
            )?;
            Ok(id)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn save_and_reopen_preserves_metadata() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "My Pack").await;
        pack.save(|_, _| {}).await.unwrap();
        drop(pack);

        let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        assert_eq!(pack2.name(), "My Pack");
        assert!(pack2.is_saved().await);
    }

    #[tokio::test]
    async fn file_content_survives_save_and_reopen() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let content = b"raw audio bytes for testing";

        let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
        let file_id = insert_staged_audio(&pack, content).await;
        pack.save(|_, _| {}).await.unwrap();
        drop(pack);

        let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        let view = pack2.get_view().unwrap();
        let (data, _) = view.get_file_data(file_id).await.unwrap();
        assert_eq!(data.as_slice(), content.as_slice());
    }

    #[tokio::test]
    async fn unsaved_recovery_prefers_dir_metadata() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "Original Name").await;
        pack.set_metadata(&Metadata {
            name: "Modified Name".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();
        pack.save_metadata().await.unwrap();
        // Drop without calling save() — UNSAVED marker remains and dir is not cleaned up.
        drop(pack);

        let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        assert_eq!(pack2.name(), "Modified Name");
        assert!(!pack2.is_saved().await);
    }

    #[tokio::test]
    async fn staged_file_recoverable_before_save() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let content = b"critical data that must survive a crash";

        let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
        let file_id = insert_staged_audio(&pack, content).await;
        // Drop without saving — simulates crash before or during save.
        // The staging file and DB row (path-based) are intact.
        drop(pack);

        let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        let view = pack2.get_view().unwrap();
        let (data, _) = view.get_file_data(file_id).await.unwrap();
        assert_eq!(data.as_slice(), content.as_slice());
    }

    #[tokio::test]
    async fn all_files_survive_multi_file_save_and_reopen() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let payloads: &[&[u8]] = &[b"file one", b"file two", b"file three"];

        let pack = new_test_pack(&pack_path, data_dir.path(), "Multi").await;
        let mut ids = Vec::new();
        for payload in payloads {
            ids.push(insert_staged_audio(&pack, payload).await);
        }
        pack.save(|_, _| {}).await.unwrap();
        drop(pack);

        let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        let view = pack2.get_view().unwrap();
        for (i, expected) in payloads.iter().enumerate() {
            let (data, _) = view.get_file_data(ids[i]).await.unwrap();
            assert_eq!(data.as_slice(), *expected);
        }
    }

    // Exercises the parallel gap-closing path in write_files: files embedded by a
    // first save, then a scattered subset deleted, then a second save that must
    // shift every surviving file after each deletion point to a new, compacted
    // offset. Varying content and lengths per file mean any job overwriting a
    // range another job hasn't read yet shows up as a content mismatch, not a
    // silent pass.
    #[tokio::test]
    async fn deleting_files_then_resaving_preserves_surviving_content() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "Compaction").await;

        let contents: Vec<Vec<u8>> = (0..24u8)
            .map(|i| {
                let len = 200 + (i as usize) * 53;
                (0..len).map(|b| i.wrapping_add(b as u8)).collect()
            })
            .collect();

        let mut ids = Vec::new();
        for content in &contents {
            ids.push(insert_staged_audio(&pack, content).await);
        }

        // First save: embeds everything contiguously, no gaps yet.
        pack.save(|_, _| {}).await.unwrap();

        // Delete a scattered subset (near the start, middle, and end) so the
        // gap-closing loop has to shift a large tail of survivors across several
        // separate deletion points with varying cumulative offsets.
        let deleted_indices = [2usize, 3, 9, 15, 20];
        let deleted_ids: Vec<u64> = deleted_indices.iter().map(|&i| ids[i]).collect();
        pack.remove_files(deleted_ids).await.unwrap();

        // Second save: triggers the parallel shift/compaction logic under test.
        pack.save(|_, _| {}).await.unwrap();
        drop(pack);

        let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        let view = pack2.get_view().unwrap();

        for (i, content) in contents.iter().enumerate() {
            if deleted_indices.contains(&i) {
                continue;
            }
            let (data, _) = view.get_file_data(ids[i]).await.unwrap();
            assert_eq!(&data, content, "content mismatch for surviving file index {i}");
        }
    }

    // Exercises the parallel newly-staged-file path in write_files: many loose
    // files, varying lengths, all copied concurrently into a pre-sized (set_len)
    // region of the pack file. Any job writing into the wrong precomputed slot,
    // or a mis-sized set_len leaving slots overlapping, shows up as a content
    // mismatch here.
    #[tokio::test]
    async fn many_new_files_survive_parallel_first_save() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "NewFiles").await;

        let contents: Vec<Vec<u8>> = (0..40u8)
            .map(|i| {
                let len = 150 + (i as usize) * 41;
                (0..len).map(|b| i.wrapping_add(b as u8)).collect()
            })
            .collect();

        let mut ids = Vec::new();
        for content in &contents {
            ids.push(insert_staged_audio(&pack, content).await);
        }

        pack.save(|_, _| {}).await.unwrap();
        drop(pack);

        let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        let view = pack2.get_view().unwrap();

        for (i, content) in contents.iter().enumerate() {
            let (data, _) = view.get_file_data(ids[i]).await.unwrap();
            assert_eq!(&data, content, "content mismatch for new file index {i}");
        }
    }

    // Diagnostic stress test: many files, then a large scattered deletion, then a
    // re-save - matching the scale where a "save spinner freezes near the end"
    // report came from. Wrapped in a timeout so a hang fails the test instead of
    // hanging the suite.
    #[tokio::test]
    async fn stress_large_pack_resave_does_not_hang() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "Stress").await;

        let n = 3065usize;
        let mut ids = Vec::new();
        for i in 0..n {
            let content = vec![(i % 256) as u8; 40 + (i % 50)];
            ids.push(insert_staged_audio(&pack, &content).await);
        }

        tokio::time::timeout(std::time::Duration::from_secs(30), pack.save(|_, _| {}))
            .await
            .expect("first save timed out (hang)")
            .unwrap();

        // Scatter deletions every 7th file, matching the "delete scattered files"
        // scenario the parallel gap-closing loop is meant for.
        let deleted: Vec<u64> = (0..n).step_by(7).map(|i| ids[i]).collect();
        pack.remove_files(deleted).await.unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(30), pack.save(|_, _| {}))
            .await
            .expect("second save timed out (hang)")
            .unwrap();
    }

    // Same idea but with bigger per-file content (to stretch out per-job I/O
    // duration and widen any race window) and repeated save/delete cycles (to
    // surface anything that only shows up after the parallel machinery has run
    // several times in the same process).
    #[tokio::test]
    async fn stress_repeated_cycles_with_larger_files_does_not_hang() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "StressBig").await;

        let n = 600usize;
        let mut ids = Vec::new();
        for i in 0..n {
            // 1KB to ~1.5MB, varying, to widen per-job I/O duration.
            let len = 1024 + (i * 2503) % (1536 * 1024);
            let content = vec![(i % 256) as u8; len];
            ids.push(insert_staged_audio(&pack, &content).await);
        }

        for cycle in 0..5 {
            tokio::time::timeout(std::time::Duration::from_secs(60), pack.save(|_, _| {}))
                .await
                .unwrap_or_else(|_| panic!("save timed out (hang) on cycle {cycle}"))
                .unwrap();

            // Delete a scattered ~10% of whatever remains.
            let remaining = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                pack.get_files(),
            )
            .await
            .unwrap()
            .unwrap();
            if remaining.len() < 20 {
                break;
            }
            let to_delete: Vec<u64> = remaining
                .iter()
                .step_by(9)
                .map(|f| f.id)
                .collect();
            pack.remove_files(to_delete).await.unwrap();
        }
        let _ = ids;
    }

    // Simulates the upload race: two uploads for identical content, both
    // reaching add_file with the same hash before either had inserted its row
    // (encode.rs's own pre-check can't prevent this). The DB's UNIQUE
    // constraint on hash - not just the pre-check - is what has to catch it.
    #[tokio::test]
    async fn duplicate_hash_upload_is_rejected_by_constraint() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "Dedup").await;

        let content = b"identical content, uploaded twice";
        let hash = blake3::hash(content);

        let encoded_path_1 = pack.dir.join("media").join("upload-1");
        tokio::fs::write(&encoded_path_1, content).await.unwrap();
        let encoded_1 = EncodedFile {
            info: FileInfo::Audio { duration: 1.0 },
            thumbnail: None,
            path: encoded_path_1,
        };

        let encoded_path_2 = pack.dir.join("media").join("upload-2");
        tokio::fs::write(&encoded_path_2, content).await.unwrap();
        let encoded_2 = EncodedFile {
            info: FileInfo::Audio { duration: 1.0 },
            thumbnail: None,
            path: encoded_path_2,
        };

        let first = pack
            .add_file(encoded_1, Path::new("a.wav"), hash)
            .await
            .unwrap();
        assert!(first.is_some(), "first upload of new content should succeed");

        let second = pack
            .add_file(encoded_2, Path::new("b.wav"), hash)
            .await
            .unwrap();
        assert!(
            second.is_none(),
            "duplicate hash should be rejected, not inserted"
        );

        let files = pack.get_files().await.unwrap();
        assert_eq!(
            files.len(),
            1,
            "only one row should exist for the duplicate hash"
        );
    }

    // Existing packs saved before the UNIQUE constraint existed may already have
    // duplicate-hash rows; the migration that adds the constraint has to clean
    // those up first (SQLite refuses to build a UNIQUE index over non-unique
    // data) rather than failing to open the pack at all.
    #[tokio::test]
    async fn migration_dedupes_existing_duplicate_hash_rows() {
        let tmp = tempdir().unwrap();
        let db_path = tmp.path().join("index.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();

        // Minimal pre-0002 schema: matches migration 0001 (a plain, non-unique
        // index on hash), with `migrations` already at index 1 - simulating a
        // pack saved before the UNIQUE constraint migration existed.
        conn.execute_batch(
            "CREATE TABLE media (
                id INTEGER PRIMARY KEY,
                file_name TEXT NOT NULL,
                file_type TEXT NOT NULL,
                \"offset\" INTEGER,
                length INTEGER,
                path TEXT,
                width INTEGER,
                height INTEGER,
                transparent INTEGER,
                duration REAL,
                audio INTEGER,
                hash BLOB NOT NULL,
                thumbnail BLOB
            ) STRICT;
            CREATE INDEX media_hash_index ON media (hash);
            CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL) STRICT;
            CREATE TABLE media_tags (
                media_id INTEGER NOT NULL,
                tag_id INTEGER NOT NULL,
                PRIMARY KEY (media_id, tag_id)
            ) STRICT;
            CREATE TABLE migrations (migration_index INTEGER NOT NULL);
            INSERT INTO migrations (migration_index) VALUES (1);",
        )
        .unwrap();

        let hash_bytes = [7u8; 32];
        conn.execute(
            "INSERT INTO media (file_name, file_type, hash) VALUES ('a.wav', 'audio', ?)",
            params![hash_bytes],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO media (file_name, file_type, hash) VALUES ('b.wav', 'audio', ?)",
            params![hash_bytes],
        )
        .unwrap();

        let count: usize = conn
            .query_row("SELECT COUNT(*) FROM media", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2, "both duplicate rows should exist pre-migration");

        // Applying the rest of the migrations (as happens on reopen) must dedupe
        // rather than fail when it tries to build the UNIQUE index.
        shared::db::migrate(&conn).unwrap();

        let count: usize = conn
            .query_row("SELECT COUNT(*) FROM media", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "migration should have deduped down to one row");
    }
}
