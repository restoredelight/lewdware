use std::{
    fs::{self, create_dir_all},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, RwLock as StdRwLock,
    },
    thread::{available_parallelism, sleep},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Result};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rayon::prelude::*;
use rusqlite::{
    backup::{Backup, StepResult},
    named_params, params,
    types::Value,
    vtab::array::{self, Array},
    OpenFlags, OptionalExtension, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use shared::{
    behaviour::v3::Behaviour,
    encode::{FileInfo, FileInfoParts, FileType},
    read_pack::{Header, Metadata, HEADER_SIZE},
};
use tokio::{
    fs::{remove_file, File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{mpsc, oneshot, Mutex as AsyncMutex, RwLock},
    task::{spawn_blocking, JoinHandle},
};
use uuid::Uuid;

use shared::encode::EncodedFile;

use crate::editor_db;
use crate::history;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MediaFile {
    pub id: u64,
    pub file_info: FileInfo,
    pub file_name: String,
    pub hash: String,
    pub tags: Vec<String>,
    pub artists: Vec<String>,
    pub source_url: Option<String>,
    pub size: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct TagSummary {
    pub name: String,
    pub media_count: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct ArtistSummary {
    pub name: String,
    pub media_count: u64,
}

enum AddFileResult {
    Inserted(u64),
    Restored(u64),
    Duplicate,
}

fn read_behaviour(connection: &rusqlite::Connection) -> Result<Behaviour> {
    let blob = connection
        .query_row(
            "SELECT blob FROM pack_data WHERE name = 'behaviour'",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()?;
    blob.as_deref()
        .map(Behaviour::from_json_bytes)
        .transpose()
        .map(|behaviour| behaviour.unwrap_or_default())
        .map_err(Into::into)
}

fn tag_media_ids(tx: &rusqlite::Transaction<'_>, tag: &str) -> Result<Vec<u64>> {
    let mut statement = tx.prepare(
        "SELECT mt.media_id FROM media_tags mt
         JOIN tags t ON t.id = mt.tag_id WHERE t.name = ?",
    )?;
    let ids = statement
        .query_map([tag], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

fn artist_media_ids(tx: &rusqlite::Transaction<'_>, artist: &str) -> Result<Vec<u64>> {
    let mut statement = tx.prepare(
        "SELECT ma.media_id FROM media_artists ma
         JOIN artists a ON a.id = ma.artist_id WHERE a.name = ?",
    )?;
    let ids = statement
        .query_map([artist], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
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
    path: Option<PathBuf>,
    data_dir: PathBuf,
    saving: Arc<RwLock<()>>,
    archive_io: Arc<RwLock<()>>,
    save_serial: Arc<AsyncMutex<()>>,
    garbage_collection: GarbageCollector,
    _lock: Option<Lock>,
    header: StdRwLock<Header>,
    dir: PathBuf,
    metadata: StdRwLock<Metadata>,
    db_pool: Pool<SqliteConnectionManager>,
    db_path: PathBuf,
    saved: AtomicBool,
    revision: AtomicU64,
    closing: AtomicBool,
    delete_on_drop: AtomicBool,
}

pub struct MediaPackView {
    path: Option<PathBuf>,
    archive_io: Arc<RwLock<()>>,
    dir: PathBuf,
    db_pool: Pool<SqliteConnectionManager>,
}

struct SaveSnapshot {
    save_id: String,
    revision: u64,
    generation_id: String,
    editor_state_id: String,
    pending_history: bool,
    metadata: Metadata,
    runtime_path: PathBuf,
    db_path: PathBuf,
    db_pool: Pool<SqliteConnectionManager>,
    _staging: tempfile::TempDir,
}

struct PreparedGeneration {
    file: tempfile::NamedTempFile,
    header: Header,
}

enum GenerationPersistence {
    Durable,
    ReplacedButNotSynced(anyhow::Error),
}

enum GarbageCollectorCommand {
    Collect,
    CollectAndWait(oneshot::Sender<Result<()>>),
    Shutdown(oneshot::Sender<()>),
}

struct GarbageCollector {
    sender: mpsc::Sender<GarbageCollectorCommand>,
    handle: AsyncMutex<Option<JoinHandle<()>>>,
}

impl GarbageCollector {
    fn start(pool: Pool<SqliteConnectionManager>, media_dir: PathBuf) -> Self {
        let (sender, mut receiver) = mpsc::channel(1);
        let handle = tokio::spawn(async move {
            while let Some(command) = receiver.recv().await {
                match command {
                    GarbageCollectorCommand::Collect => {
                        if let Err(error) =
                            process_pending_file_deletions(pool.clone(), media_dir.clone()).await
                        {
                            tracing::error!("loose-media garbage collection failed: {error}");
                        }
                    }
                    GarbageCollectorCommand::CollectAndWait(reply) => {
                        let result =
                            process_pending_file_deletions(pool.clone(), media_dir.clone()).await;
                        let _ = reply.send(result);
                    }
                    GarbageCollectorCommand::Shutdown(reply) => {
                        let _ = reply.send(());
                        break;
                    }
                }
            }
        });
        Self {
            sender,
            handle: AsyncMutex::new(Some(handle)),
        }
    }

    fn wake(&self) {
        match self.sender.try_send(GarbageCollectorCommand::Collect) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::debug!("garbage collector is already shut down");
            }
        }
    }

    async fn collect(&self) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(GarbageCollectorCommand::CollectAndWait(reply))
            .await
            .map_err(|_| anyhow!("garbage collector is shut down"))?;
        response
            .await
            .map_err(|_| anyhow!("garbage collector stopped without replying"))?
    }

    async fn shutdown(&self) {
        let (reply, response) = oneshot::channel();
        if self
            .sender
            .send(GarbageCollectorCommand::Shutdown(reply))
            .await
            .is_ok()
        {
            let _ = response.await;
        }
        if let Some(handle) = self.handle.lock().await.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for GarbageCollector {
    fn drop(&mut self) {
        if let Ok(mut handle) = self.handle.try_lock() {
            if let Some(handle) = handle.take() {
                handle.abort();
            }
        }
    }
}

fn log_save_phase(phase: &'static str, started: Instant) {
    tracing::info!(
        phase,
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "save phase completed"
    );
}

#[derive(Debug)]
pub struct MediaData {
    pub id: i64,
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug)]
struct CopyRun {
    source_offset: u64,
    dest_offset: u64,
    length: u64,
    media_ids: Vec<i64>,
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
    pub async fn new_unsaved(data_dir: &Path, name: &str) -> Result<Self> {
        let header = Header::new();
        let metadata = Metadata {
            name: name.to_string(),
            ..Metadata::default()
        };
        Self::initialize(None, data_dir, header, metadata, None).await
    }

    pub async fn recover_unsaved(data_dir: &Path, id: Uuid) -> Result<Self> {
        let dir = data_dir.join("Lewdware Pack Editor").join(id.to_string());
        if !dir.join("UNSAVED").is_file() {
            bail!("draft no longer exists");
        }
        let metadata = Metadata::from_buf(&fs::read(dir.join("Metadata"))?)?;
        let db_path = dir.join("index.db");
        if !db_path.is_file() {
            bail!("draft database is missing");
        }
        let manager = sqlite_connection_manager(&db_path);
        let db_pool = Pool::builder().build(manager)?;
        let pool = db_pool.clone();
        spawn_blocking(move || -> Result<_> {
            let conn = pool.get()?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            editor_db::initialize(&conn)
        })
        .await??;
        let mut header = Header::new();
        header.id = id;
        let pack = Self {
            path: None,
            data_dir: data_dir.to_path_buf(),
            saving: Arc::new(RwLock::new(())),
            archive_io: Arc::new(RwLock::new(())),
            save_serial: Arc::new(AsyncMutex::new(())),
            garbage_collection: GarbageCollector::start(db_pool.clone(), dir.join("media")),
            _lock: None,
            header: StdRwLock::new(header),
            dir,
            metadata: StdRwLock::new(metadata),
            db_pool,
            db_path,
            saved: AtomicBool::new(false),
            revision: AtomicU64::new(0),
            closing: AtomicBool::new(false),
            delete_on_drop: AtomicBool::new(false),
        };
        pack.sweep_loose_files().await?;
        Ok(pack)
    }

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

        Self::initialize(Some(path), data_dir, header, metadata, Some(lock)).await
    }

    async fn initialize(
        path: Option<PathBuf>,
        data_dir: &Path,
        header: Header,
        metadata: Metadata,
        lock: Option<Lock>,
    ) -> Result<Self> {
        let dir = data_dir
            .join("Lewdware Pack Editor")
            .join(header.id.to_string());

        create_dir_all(&dir)?;
        create_dir_all(dir.join("media"))?;

        let metadata_path = dir.join("Metadata");
        File::create(&metadata_path)
            .await?
            .write_all(&metadata.to_buf()?)
            .await?;

        let db_path = dir.join("index.db");
        let manager = sqlite_connection_manager(&db_path);
        let db_pool = Pool::builder().build(manager)?;

        let pool = db_pool.clone();
        let metadata_for_db = metadata.clone();
        spawn_blocking(move || -> Result<_> {
            let conn = pool.get()?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            editor_db::initialize_empty(&conn, &metadata_for_db)
        })
        .await??;

        File::create(dir.join("UNSAVED")).await?;

        let pack = Self {
            path,
            data_dir: data_dir.to_path_buf(),
            saving: Arc::new(RwLock::new(())),
            archive_io: Arc::new(RwLock::new(())),
            save_serial: Arc::new(AsyncMutex::new(())),
            garbage_collection: GarbageCollector::start(db_pool.clone(), dir.join("media")),
            _lock: lock,
            header: StdRwLock::new(header),
            dir,
            metadata: StdRwLock::new(metadata),
            db_pool,
            saved: AtomicBool::new(false),
            revision: AtomicU64::new(0),
            closing: AtomicBool::new(false),
            delete_on_drop: AtomicBool::new(false),
            db_path,
        };
        pack.sweep_loose_files().await?;
        Ok(pack)
    }

    pub async fn open(path: PathBuf, data_dir: &Path) -> Result<Self> {
        Self::open_internal(path, data_dir, true, false).await
    }

    async fn open_internal(
        path: PathBuf,
        data_dir: &Path,
        sweep_orphans: bool,
        reuse_working_db: bool,
    ) -> Result<Self> {
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
        create_dir_all(dir.join("media"))?;

        let db_path = dir.join("index.db");
        let has_unsaved = fs::exists(dir.join("UNSAVED"))? && fs::exists(&db_path)?;
        let has_working_db = has_unsaved || (reuse_working_db && fs::exists(&db_path)?);

        let mut metadata = if has_working_db {
            let metadata_path = dir.join("Metadata");
            match fs::read(&metadata_path)
                .map_err(|err| anyhow!(err))
                .and_then(|buf| Metadata::from_buf(&buf).map_err(|err| err.into()))
            {
                Ok(metadata) => metadata,
                Err(err) => {
                    // Never replace damaged recovery metadata with an all-default value. The
                    // last saved metadata embedded in the pack is a safer recovery baseline.
                    tracing::error!("could not read working metadata, using saved metadata: {err}");
                    file.seek(SeekFrom::Start(header.metadata_offset)).await?;
                    let mut buf = vec![0u8; header.metadata_length as usize];
                    file.read_exact(&mut buf).await?;
                    let metadata = Metadata::from_buf(&buf)?;
                    fs::write(&metadata_path, metadata.to_buf()?)?;
                    metadata
                }
            }
        } else {
            file.seek(SeekFrom::Start(header.metadata_offset)).await?;
            let mut buf = vec![0u8; header.metadata_length as usize];
            file.read_exact(&mut buf).await?;
            Metadata::from_buf(&buf)?
        };

        let runtime_path = dir.join("saved-index.db");
        let has_archive = !header.is_default() && header.index_length > 0;
        let db_data = if has_archive {
            let archive_db = async {
                file.seek(SeekFrom::Start(header.index_offset)).await?;
                let mut db_data = vec![0u8; header.index_length as usize];
                file.read_exact(&mut db_data).await?;
                Ok::<_, io::Error>(db_data)
            }
            .await;
            match archive_db {
                Ok(db_data) => Some(db_data),
                Err(error) if has_unsaved && error.kind() == io::ErrorKind::UnexpectedEof => {
                    // An interrupted in-place save can truncate the old embedded index after its
                    // recoverable media-offset batches were committed to the working database.
                    // The UNSAVED database is authoritative in this case.
                    tracing::warn!(
                        "could not read the saved pack index; recovering from the working database: {error}"
                    );
                    None
                }
                Err(error) => return Err(error.into()),
            }
        } else {
            None
        };
        let has_archive_db = db_data.is_some();

        if !has_working_db {
            // The working database uses WAL. Never place a freshly extracted main
            // database beside sidecars belonging to the previous working copy:
            // SQLite could otherwise replay stale pages into the extracted index.
            remove_sqlite_sidecars(&db_path)?;
            let _ = fs::remove_file(&db_path);
        }
        if let Some(db_data) = db_data {
            let mut db_file = File::create(&runtime_path).await?;
            db_file.write_all(&db_data).await?;
            db_file.flush().await?;
        }

        let manager = sqlite_connection_manager(&db_path);
        // Sized to comfortably cover the parallel-copy worker count in write_files,
        // so workers grabbing a connection to record their offset never queue on
        // the pool itself (they may still briefly serialize on SQLite's own write
        // lock, which rusqlite's default 5s busy_timeout already handles).
        let pool_size = available_parallelism().map(|n| n.get() as u32).unwrap_or(4);
        let db_pool = Pool::builder().max_size(pool_size).build(manager)?;

        let pool = db_pool.clone();
        let runtime = has_archive_db.then_some(runtime_path.clone());
        let metadata_for_db = metadata.clone();
        spawn_blocking(move || -> Result<_> {
            let mut conn = pool.get()?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            if !has_working_db {
                let runtime = runtime
                    .as_ref()
                    .ok_or_else(|| anyhow!("pack has no index"))?;
                editor_db::import_runtime(&mut conn, &runtime, &metadata_for_db)?;
            } else {
                editor_db::initialize(&conn)?;
                if let Some(runtime) = &runtime {
                    if let Err(error) = editor_db::reconcile_archive(&mut conn, runtime) {
                        if has_unsaved && editor_db::is_corrupt_database_error(&error) {
                            tracing::warn!(
                                "saved pack index is corrupt after an interrupted in-place save; \
                                 recovering from the working database: {error}"
                            );
                            conn.execute("DELETE FROM save_sessions", [])?;
                        } else {
                            return Err(error);
                        }
                    }
                } else {
                    conn.execute("DELETE FROM save_sessions", [])?;
                }
                editor_db::reset_session(&mut conn, has_unsaved)?;
            }
            if let Some(runtime) = runtime {
                let _ = fs::remove_file(runtime);
            }
            Ok(())
        })
        .await??;
        if has_working_db {
            let pool = db_pool.clone();
            let stored = spawn_blocking(move || -> Result<Vec<u8>> {
                Ok(pool.get()?.query_row(
                    "SELECT metadata FROM pack_metadata WHERE singleton = 1",
                    [],
                    |row| row.get(0),
                )?)
            })
            .await??;
            metadata = Metadata::from_buf(&stored)?;
            fs::write(dir.join("Metadata"), &stored)?;
        }

        let pack = Self {
            path: Some(path),
            data_dir: data_dir.to_path_buf(),
            saving: Arc::new(RwLock::new(())),
            archive_io: Arc::new(RwLock::new(())),
            save_serial: Arc::new(AsyncMutex::new(())),
            garbage_collection: GarbageCollector::start(db_pool.clone(), dir.join("media")),
            _lock: Some(lock),
            header: StdRwLock::new(header),
            dir,
            metadata: StdRwLock::new(metadata),
            db_pool,
            saved: AtomicBool::new(!has_unsaved),
            revision: AtomicU64::new(0),
            closing: AtomicBool::new(false),
            delete_on_drop: AtomicBool::new(false),
            db_path,
        };
        if sweep_orphans {
            pack.sweep_loose_files().await?;
        }
        pack.process_pending_file_deletions().await?;
        Ok(pack)
    }

    pub fn get_view(&self) -> Result<MediaPackView> {
        Ok(MediaPackView {
            path: self.path.clone(),
            archive_io: self.archive_io.clone(),
            dir: self.dir.clone(),
            db_pool: self.db_pool.clone(),
        })
    }

    pub fn name(&self) -> String {
        self.metadata.read().unwrap().name.clone()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn id(&self) -> Uuid {
        self.header.read().unwrap().id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Remove physical media files that have no durable loose-source row.
    async fn sweep_loose_files(&self) -> Result<()> {
        let pool = self.db_pool.clone();
        let referenced = spawn_blocking(move || -> Result<_> {
            let conn = pool.get()?;
            let mut stmt = conn.prepare("SELECT path FROM loose_media")?;
            let paths = stmt
                .query_map([], |row| row.get::<_, String>(0).map(PathBuf::from))?
                .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
            Ok(paths)
        })
        .await??;
        let media_dir = self.dir.join("media");
        spawn_blocking(move || -> Result<()> {
            for entry in fs::read_dir(&media_dir)? {
                let entry = entry?;
                let relative = PathBuf::from(entry.file_name());
                if entry.file_type()?.is_file() && !referenced.contains(&relative) {
                    fs::remove_file(entry.path())?;
                }
            }
            if let Some(pack_dir) = media_dir.parent() {
                for entry in fs::read_dir(pack_dir)? {
                    let entry = entry?;
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if entry.file_type()?.is_dir()
                        && (name.starts_with("pack-editor-upload-")
                            || name.starts_with("pack-editor-import-")
                            || name.starts_with("pack-editor-save-")
                            || name.starts_with("pack-editor-save-as-")
                            || name.starts_with("pack-editor-restore-"))
                    {
                        fs::remove_dir_all(entry.path())?;
                    }
                }
            }
            Ok(())
        })
        .await??;
        if let Some(path) = &self.path {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            let prefix = format!(".pack-editor-generation-{}-", self.id());
            for entry in fs::read_dir(parent)? {
                let entry = entry?;
                if entry.file_type()?.is_file()
                    && entry.file_name().to_string_lossy().starts_with(&prefix)
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }

    async fn process_pending_file_deletions(&self) -> Result<()> {
        self.garbage_collection.collect().await
    }

    fn schedule_pending_file_deletions(&self) {
        self.garbage_collection.wake();
    }

    pub fn is_untitled(&self) -> bool {
        self.path.is_none()
    }

    pub async fn is_saved(&self) -> bool {
        let _handle = self.saving.write().await;
        self.saved.load(Ordering::Relaxed)
    }

    pub async fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let _save = self.save_serial.lock().await;
        let _mutations = self.saving.write().await;
        self.garbage_collection.shutdown().await;
        self.delete_on_drop.store(true, Ordering::SeqCst);
    }

    pub async fn mark_unsaved(&self) -> Result<()> {
        self.mark_unsaved_now()?;
        self.schedule_pending_file_deletions();
        Ok(())
    }

    fn mark_unsaved_now(&self) -> Result<()> {
        self.revision.fetch_add(1, Ordering::SeqCst);
        if self.saved.load(Ordering::Relaxed) {
            fs::File::create(self.dir.join("UNSAVED"))?;
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
        let path = self.path.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "pack has not been saved yet")
        })?;
        OpenOptions::new().read(true).open(path).await
    }

    async fn create_save_snapshot(&self) -> Result<SaveSnapshot> {
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

    async fn reconcile_save_snapshot(
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

    async fn clear_save_session(&self, save_id: &str) -> Result<()> {
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

    async fn write_snapshot_tail(
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

    async fn prepare_generation(
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

    async fn materialize_deleted_snapshot_media(&self, snapshot: &SaveSnapshot) -> Result<()> {
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

    async fn commit_generation(
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

    async fn validate_generation_commit(&self, snapshot: &SaveSnapshot) -> Result<()> {
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

    async fn save_in_place(
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

    async fn write_files(
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

    async fn write_files_from(
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

    pub fn metadata(&self) -> Metadata {
        self.metadata.read().unwrap().clone()
    }

    pub async fn set_metadata(&self, metadata: &Metadata) -> Result<()> {
        let _handle = self.saving.read().await;
        let data = metadata.to_buf()?;
        self.db_execute(move |mut connection| {
            history::record(&mut connection, "Edit pack metadata", 0, &[], |tx| {
                tx.execute(
                    "UPDATE pack_metadata SET metadata = ? WHERE singleton = 1",
                    [&data],
                )?;
                Ok(())
            })
        })
        .await?;
        *self.metadata.write().unwrap() = metadata.clone();
        self.mark_unsaved().await
    }

    pub async fn save_metadata(&self) -> Result<()> {
        let final_path = self.dir.join("Metadata");
        let data = self.metadata.read().unwrap().to_buf()?;
        let dir = self.dir.clone();
        spawn_blocking(move || -> Result<()> {
            let mut temp = tempfile::NamedTempFile::new_in(dir)?;
            std::io::Write::write_all(&mut temp, &data)?;
            temp.as_file().sync_all()?;
            temp.persist(final_path).map_err(|e| e.error)?;
            Ok(())
        })
        .await??;
        Ok(())
    }

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
    async fn add_file(
        &self,
        encoded_file: EncodedFile,
        path: &Path,
        hash: blake3::Hash,
    ) -> Result<Option<MediaFile>> {
        self.add_file_to_import(encoded_file, path, hash, None)
            .await
    }

    pub async fn begin_media_import(&self) -> Result<i64> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| history::begin_media_import(&mut connection))
            .await
    }

    pub async fn finish_media_import(&self, history_id: i64) -> Result<history::Status> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| {
            history::finish_media_import(&mut connection, history_id)
        })
        .await
    }

    pub async fn add_file_to_import(
        &self,
        encoded_file: EncodedFile,
        path: &Path,
        hash: blake3::Hash,
        history_id: Option<i64>,
    ) -> Result<Option<MediaFile>> {
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
                            return Ok(AddFileResult::Duplicate);
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
                    .map(Some)
                    .ok_or_else(|| anyhow!("restored media {id} is not active"));
            }
            AddFileResult::Duplicate => {
                match tokio::fs::remove_file(&encoded_path).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                return Ok(None);
            }
        };
        // Do this synchronously immediately after the database commit. Upload cancellation can
        // drop the async task at its next await; the recovery marker must already exist by then.
        self.mark_unsaved_now()?;
        if !encoded_file.artists.is_empty() {
            self.add_artists(id, encoded_file.artists.clone()).await?;
        }

        Ok(Some(MediaFile {
            id,
            file_name,
            file_info,
            hash: hash.to_string(),
            tags: vec![],
            artists: encoded_file.artists,
            source_url: encoded_file.source_url,
            size,
        }))
    }

    pub async fn remove_files(&self, ids: Vec<u64>) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        let label = if ids.len() == 1 {
            "Remove media item".to_string()
        } else {
            format!("Remove {} media items", ids.len())
        };
        self.db_execute(move |mut conn| {
            history::record_media_visibility(&mut conn, &label, &ids, true)
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

    async fn reload_metadata_from_db(&self) -> Result<()> {
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
    async fn purge_history_files(&self, ids: Vec<u64>) -> Result<()> {
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

    pub async fn get_all_tags(&self) -> Result<Vec<String>> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare("SELECT name FROM tags")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>("name"))?;
            rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
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

    pub async fn get_tag_summaries(&self) -> Result<Vec<TagSummary>> {
        self.db_execute(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT tags.name, COUNT(media.id) FROM tags
                 LEFT JOIN media_tags ON media_tags.tag_id = tags.id
                 LEFT JOIN media ON media.id = media_tags.media_id AND media.deleted = 0
                 GROUP BY tags.id ORDER BY tags.name COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(TagSummary {
                    name: row.get(0)?,
                    media_count: row.get(1)?,
                })
            })?;
            rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
        })
        .await
    }

    pub async fn rename_tag(&self, from: String, to: String) -> Result<Behaviour> {
        let _handle = self.saving.read().await;
        let behaviour = self
            .db_execute(move |mut conn| {
                history::record_with_media_refs(&mut conn, "Rename tag", 0, |tx| {
                    let media_ids = tag_media_ids(tx, &from)?;
                    let target_exists: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM tags WHERE name = ?)",
                        params![to],
                        |row| row.get(0),
                    )?;
                    if target_exists {
                        bail!("A tag named \"{to}\" already exists. Merge the tags instead.");
                    }
                    let changed =
                        tx.execute("UPDATE tags SET name = ? WHERE name = ?", params![to, from])?;
                    if changed == 0 {
                        tx.execute("INSERT INTO tags (name) VALUES (?)", params![to])?;
                    }
                    let mut behaviour = read_behaviour(tx)?;
                    behaviour.rewrite_tag(&from, Some(&to));
                    tx.execute(
                        "INSERT OR REPLACE INTO pack_data (name, blob) VALUES ('behaviour', ?)",
                        params![behaviour.to_json_bytes()?],
                    )?;
                    Ok((behaviour, media_ids))
                })
            })
            .await?;
        self.mark_unsaved().await?;
        Ok(behaviour)
    }

    pub async fn merge_tag(&self, from: String, to: String) -> Result<Behaviour> {
        let _handle = self.saving.read().await;
        let behaviour = self.db_execute(move |mut conn| {
            history::record_with_media_refs(&mut conn, "Merge tags", 0, |tx| {
            let media_ids = tag_media_ids(tx, &from)?;
            tx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?)", params![to])?;
            tx.execute("INSERT OR IGNORE INTO media_tags (media_id, tag_id) SELECT media_tags.media_id, target.id FROM media_tags JOIN tags source ON source.id = media_tags.tag_id JOIN tags target ON target.name = ? WHERE source.name = ?", params![to, from])?;
            tx.execute("DELETE FROM media_tags WHERE tag_id IN (SELECT id FROM tags WHERE name = ?)", params![from])?;
            tx.execute("DELETE FROM tags WHERE name = ?", params![from])?;
            let mut behaviour = read_behaviour(tx)?;
            behaviour.rewrite_tag(&from, Some(&to));
            tx.execute("INSERT OR REPLACE INTO pack_data (name, blob) VALUES ('behaviour', ?)", params![behaviour.to_json_bytes()?])?;
            Ok((behaviour, media_ids))
            })
        }).await?;
        self.mark_unsaved().await?;
        Ok(behaviour)
    }

    pub async fn delete_tag(&self, tag: String) -> Result<Behaviour> {
        let _handle = self.saving.read().await;
        let behaviour = self
            .db_execute(move |mut conn| {
                history::record_with_media_refs(&mut conn, "Delete tag", 0, |tx| {
                    let media_ids = tag_media_ids(tx, &tag)?;
                    tx.execute(
                    "DELETE FROM media_tags WHERE tag_id IN (SELECT id FROM tags WHERE name = ?)",
                    params![tag],
                )?;
                    tx.execute("DELETE FROM tags WHERE name = ?", params![tag])?;
                    let mut behaviour = read_behaviour(tx)?;
                    behaviour.rewrite_tag(&tag, None);
                    tx.execute(
                        "INSERT OR REPLACE INTO pack_data (name, blob) VALUES ('behaviour', ?)",
                        params![behaviour.to_json_bytes()?],
                    )?;
                    Ok((behaviour, media_ids))
                })
            })
            .await?;
        self.mark_unsaved().await?;
        Ok(behaviour)
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
        self.db_execute(move |mut connection| {
            history::record(&mut connection, "Add media tag", 0, &[id], |tx| {
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
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| {
            history::record(
                &mut connection,
                "Create and add media tag",
                0,
                &[id],
                |tx| {
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
                },
            )
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

    /// Writes a named blob into the pack's generic `pack_data` table (e.g. `"behaviour"` for the
    /// Edgeware importer's converted behaviour.json) -- `name` is the table's primary key, so a
    /// repeat write for the same name replaces it.
    pub async fn set_pack_data(&self, name: &str, blob: Vec<u8>) -> Result<()> {
        let _handle = self.saving.read().await;
        let name = name.to_string();
        let label = if name == "behaviour" {
            "Edit pack behaviour".to_string()
        } else {
            format!("Edit pack data “{name}”")
        };
        self.db_execute(move |mut connection| {
            history::record(&mut connection, &label, 0, &[], |tx| {
                tx.execute(
                    "INSERT OR REPLACE INTO pack_data (name, blob) VALUES (?, ?)",
                    params![name, blob],
                )?;
                Ok(())
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    /// Reads a named blob from the pack's `pack_data` table (e.g. `"behaviour"`). `None` if the
    /// pack doesn't carry an entry with this name -- a pack predating this key, or never written
    /// by a tool that populates it, is expected, not an error. Mirrors the engine's own
    /// `lewdware::media::pack::MediaPack::get_pack_data`.
    pub async fn get_pack_data(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let _handle = self.saving.read().await;
        let name = name.to_string();
        self.db_execute(move |conn| {
            conn.query_row(
                "SELECT blob FROM pack_data WHERE name = ?",
                params![name],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })
        .await
    }

    pub async fn remove_tag(&self, id: u64, tag: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| {
            history::record(&mut connection, "Remove media tag", 0, &[id], |tx| {
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
        if ids.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| {
            history::record(&mut connection, "Add tag to media", 0, &ids, |tx| {
                tx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?)", params![tag])?;
                let tag_id: u64 =
                    tx.query_row("SELECT id FROM tags WHERE name = ?", params![tag], |row| {
                        row.get("id")
                    })?;
                for id in &ids {
                    tx.execute(
                        "INSERT OR IGNORE INTO media_tags (media_id, tag_id) VALUES (?, ?)",
                        params![id, tag_id],
                    )?;
                }
                Ok(())
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn remove_tag_from_files(&self, ids: Vec<u64>, tag: String) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| {
            history::record(&mut connection, "Remove tag from media", 0, &ids, |tx| {
                for id in &ids {
                    tx.execute("DELETE FROM media_tags WHERE media_id = ? AND tag_id IN (SELECT id FROM tags WHERE name = ?)", params![id, tag])?;
                }
                Ok(())
            })
        }).await?;
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
        self.db_execute(move |mut connection| {
            history::record(&mut connection, "Add media artist", 0, &[id], |tx| {
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
        self.db_execute(move |mut connection| {
            history::record(
                &mut connection,
                "Create and add media artist",
                0,
                &[id],
                |tx| {
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
                },
            )
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn remove_artist(&self, id: u64, artist: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| {
            history::record(&mut connection, "Remove media artist", 0, &[id], |tx| {
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
                 LEFT JOIN media ON media.id = media_artists.media_id AND media.deleted = 0
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
        self.db_execute(move |mut conn| {
            history::record_with_media_refs(&mut conn, "Rename artist", 0, |tx| {
                let media_ids = artist_media_ids(tx, &from)?;
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
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn merge_artist(&self, from: String, to: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut conn| {
            history::record_with_media_refs(&mut conn, "Merge artists", 0, |tx| {
            let media_ids = artist_media_ids(tx, &from)?;
            tx.execute("INSERT OR IGNORE INTO artists (name) VALUES (?)", params![to])?;
            tx.execute("INSERT OR IGNORE INTO media_artists (media_id, artist_id) SELECT media_artists.media_id, target.id FROM media_artists JOIN artists source ON source.id = media_artists.artist_id JOIN artists target ON target.name = ? WHERE source.name = ?", params![to, from])?;
            tx.execute("DELETE FROM media_artists WHERE artist_id IN (SELECT id FROM artists WHERE name = ?)", params![from])?;
            tx.execute("DELETE FROM artists WHERE name = ?", params![from])?;
            Ok(((), media_ids))
            })
        }).await?;
        self.mark_unsaved().await
    }

    pub async fn delete_artist(&self, artist: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut conn| {
            history::record_with_media_refs(&mut conn, "Delete artist", 0, |tx| {
            let media_ids = artist_media_ids(tx, &artist)?;
            tx.execute(
                "DELETE FROM media_artists WHERE artist_id IN (SELECT id FROM artists WHERE name = ?)",
                params![artist],
            )?;
            tx.execute("DELETE FROM artists WHERE name = ?", params![artist])?;
            Ok(((), media_ids))
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn add_artist_to_files(&self, ids: Vec<u64>, artist: String) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        self.db_execute(move |mut conn| {
            history::record(&mut conn, "Add artist to media", 0, &ids, |tx| {
                tx.execute(
                    "INSERT OR IGNORE INTO artists (name) VALUES (?)",
                    params![artist],
                )?;
                let artist_id: u64 = tx.query_row(
                    "SELECT id FROM artists WHERE name = ?",
                    params![artist],
                    |row| row.get("id"),
                )?;
                for id in &ids {
                    tx.execute(
                        "INSERT OR IGNORE INTO media_artists (media_id, artist_id) VALUES (?, ?)",
                        params![id, artist_id],
                    )?;
                }
                Ok(())
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    pub async fn remove_artist_from_files(&self, ids: Vec<u64>, artist: String) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let _handle = self.saving.read().await;
        self.db_execute(move |mut conn| {
            history::record(&mut conn, "Remove artist from media", 0, &ids, |tx| {
            for id in &ids {
                tx.execute("DELETE FROM media_artists WHERE media_id = ? AND artist_id IN (SELECT id FROM artists WHERE name = ?)", params![id, artist])?;
            }
            Ok(())
            })
        }).await?;
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

        self.mark_unsaved().await
    }
}

impl Drop for MediaPack {
    fn drop(&mut self) {
        if self.saved.load(Ordering::Relaxed) || self.delete_on_drop.load(Ordering::Relaxed) {
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
        let path = self.path.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "pack has not been saved yet")
        })?;
        OpenOptions::new().read(true).open(path).await
    }

    async fn get_raw_file(&self, id: u64) -> Result<(FileData, FileType, bool)> {
        let (offset, length, path, file_type, transparent) = self
            .db_execute(move |conn| {
                conn.query_row_and_then(
                    "SELECT p.\"offset\", p.length, l.path, m.file_type, m.transparent
                     FROM media m
                     LEFT JOIN pack_media p ON p.media_id = m.id
                     LEFT JOIN loose_media l ON l.media_id = m.id
                     WHERE m.id = ?",
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
            })
            .await?;

        let file_data = match (offset, length, path) {
            (Some(offset), Some(length), _) => {
                let mut file = self.open_read().await?;
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
        let _handle = self.archive_io.read().await;
        let (file_data, file_type, transparent) = self.get_raw_file(id).await?;
        crate::thumbnail::generate_preview(file_data, file_type == FileType::Image, transparent)
            .await
    }

    pub async fn get_display(&self, id: u64) -> Result<Vec<u8>> {
        let _handle = self.archive_io.read().await;
        let (file_data, _, _) = self.get_raw_file(id).await?;
        crate::thumbnail::generate_display_image(file_data).await
    }

    pub async fn get_file_data(&self, id: u64) -> Result<(Vec<u8>, FileType)> {
        let _handle = self.archive_io.read().await;
        let (file_data, file_type, _) = self.get_raw_file(id).await?;
        let data = match file_data {
            FileData::Path(path) => tokio::fs::read(path).await?,
            FileData::Data(data) => data,
        };
        Ok((data, file_type))
    }

    pub async fn get_file_range(&self, id: u64, range: Range) -> Result<(DataRange, FileType)> {
        let _handle = self.archive_io.read().await;

        let (offset, length, path, file_type) = self
            .db_execute(move |conn| {
                conn.query_row_and_then(
                    "SELECT p.\"offset\", COALESCE(p.length, l.length) AS length,
                            l.path, m.file_type
                     FROM media m
                     LEFT JOIN pack_media p ON p.media_id = m.id
                     LEFT JOIN loose_media l ON l.media_id = m.id
                     WHERE m.id = ?",
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
            })
            .await?;

        let data_range = match (offset, length, path) {
            (Some(offset), Some(length), _) => {
                let mut file = self.open_read().await?;
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

async fn process_pending_file_deletions(
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

fn copy_range_forward(
    source: &mut fs::File,
    destination: &mut fs::File,
    run: &CopyRun,
    buffer: &mut [u8],
) -> Result<()> {
    source.seek(SeekFrom::Start(run.source_offset))?;
    destination.seek(SeekFrom::Start(run.dest_offset))?;
    let mut remaining = run.length;
    while remaining > 0 {
        let amount = remaining.min(buffer.len() as u64) as usize;
        source.read_exact(&mut buffer[..amount])?;
        destination.write_all(&buffer[..amount])?;
        remaining -= amount as u64;
    }
    Ok(())
}

/// Whether `err` (as returned by a query through `db_execute`, which wraps the
/// underlying `rusqlite::Error` in an `anyhow::Error`) is a `UNIQUE` constraint
/// violation on `media.{column}` specifically -- `media` has two unique columns
/// (`hash`, `file_name`), which need different handling (see `add_file`/`set_title`), so
/// the column has to be pulled out of SQLite's own failure message rather than just
/// checking for "some constraint or other failed".
fn violates_unique_constraint(err: &anyhow::Error, column: &str) -> bool {
    match err.downcast_ref::<rusqlite::Error>() {
        Some(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::ConstraintViolation,
                ..
            },
            Some(message),
        )) => message.contains(&format!("media.{column}")),
        _ => false,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .to_string()
}

fn remove_sqlite_sidecars(path: &Path) -> io::Result<()> {
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        match fs::remove_file(PathBuf::from(sidecar)) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn clone_archive(source: &Path, pack_id: Uuid) -> Result<tempfile::NamedTempFile> {
    let parent = source
        .parent()
        .ok_or_else(|| anyhow!("pack destination has no parent directory"))?;
    let generation = tempfile::Builder::new()
        .prefix(&format!(".pack-editor-generation-{pack_id}-"))
        .tempfile_in(parent)?;

    // Reflink implementations such as macOS clonefile require the destination
    // not to exist. Keep TempPath's delete-on-drop guard while removing the
    // placeholder created by NamedTempFile, then reopen the resulting file.
    let (placeholder, generation_path) = generation.into_parts();
    drop(placeholder);
    fs::remove_file(&generation_path)?;
    if reflink_copy::reflink(source, &generation_path).is_err() {
        fs::copy(source, &generation_path)?;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&generation_path)?;
    let generation = tempfile::NamedTempFile::from_parts(file, generation_path);
    generation
        .as_file()
        .set_permissions(fs::metadata(source)?.permissions())?;
    Ok(generation)
}

fn sync_parent_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()?;
    }
    Ok(())
}

fn is_storage_full(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .and_then(io::Error::raw_os_error)
            .is_some_and(|code| code == libc::ENOSPC || code == libc::EDQUOT || code == 112)
    })
}

/// Produces the next name to try after `name` collided with an existing file: appends
/// " (1)" before the extension, or bumps an existing " (n)" suffix to " (n+1)" so repeated
/// collisions count up instead of piling up " (1) (1) (1)"-style.
fn next_candidate_name(name: &str) -> String {
    let (base, ext) = match name.rsplit_once('.') {
        Some((base, ext)) if !base.is_empty() => (base, Some(ext)),
        _ => (name, None),
    };

    let (stem, next_n) = match base.rfind(" (") {
        Some(open)
            if base.ends_with(')')
                && let Ok(n) = base[open + 2..base.len() - 1].parse::<u32>() =>
        {
            (&base[..open], n + 1)
        }
        _ => (base, 1),
    };

    match ext {
        Some(ext) => format!("{stem} ({next_n}).{ext}"),
        None => format!("{stem} ({next_n})"),
    }
}

#[cfg(test)]
fn media_id_array(ids: &[u64]) -> Result<Array> {
    let values = ids
        .iter()
        .map(|&id| Ok(Value::Integer(i64::try_from(id)?)))
        .collect::<Result<Vec<_>>>()?;
    Ok(Rc::new(values))
}

fn release_snapshot_media(
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

fn release_snapshot_media_tx(
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

fn commit_in_place_media(
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

fn sqlite_connection_manager(path: &Path) -> SqliteConnectionManager {
    SqliteConnectionManager::file(path).with_init(|conn| {
        array::load_module(conn)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
    })
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
    use std::{
        collections::HashMap,
        sync::{Condvar, Mutex},
    };

    use proptest::prelude::*;
    use rusqlite::params;
    use shared::{
        behaviour::{
            Behaviour, ContentGroup, DesignValues, Experience, FrequencyAnchors, Level, TextItem,
            Timeline, WebLink,
        },
        read_pack::Metadata,
    };
    use tempfile::tempdir;

    use super::*;

    async fn new_test_pack(pack_path: &Path, data_dir: &Path, name: &str) -> MediaPack {
        MediaPack::new(pack_path.to_path_buf(), data_dir, name)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn destinationless_pack_uses_its_generated_id_on_first_save() {
        let output = tempdir().unwrap();
        let data = tempdir().unwrap();
        let pack = MediaPack::new_unsaved(data.path(), "Untitled Pack")
            .await
            .unwrap();
        let id = pack.header.read().unwrap().id;
        assert!(pack.path().is_none());
        assert!(pack.dir().ends_with(id.to_string()));

        let destination = output.path().join("first-save.lwpack");
        let opened = pack
            .save_as(&destination, |_, _| {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(opened.header.read().unwrap().id, id);
        assert_eq!(opened.path(), Some(destination.as_path()));
        assert!(destination.is_file());
    }

    #[tokio::test]
    async fn embedded_modes_roundtrip_through_database_history() {
        let output = tempdir().unwrap();
        let data = tempdir().unwrap();
        let pack = new_test_pack(&output.path().join("modes.lwpack"), data.path(), "Modes").await;
        let bytes = include_bytes!("../../../default-modes/sandbox/build/Sandbox.lwmode").to_vec();

        let id = pack.add_mode(bytes.clone()).await.unwrap();
        assert_eq!(pack.get_modes().await.unwrap(), vec![(id, bytes.clone())]);

        pack.remove_mode(id).await.unwrap();
        assert!(pack.get_modes().await.unwrap().is_empty());
        pack.undo().await.unwrap();
        assert_eq!(pack.get_modes().await.unwrap().len(), 1);
        pack.redo().await.unwrap();
        assert!(pack.get_modes().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn imported_media_undo_keeps_its_loose_source_for_redo() {
        let output = tempdir().unwrap();
        let data = tempdir().unwrap();
        let pack = new_test_pack(
            &output.path().join("import-history.lwpack"),
            data.path(),
            "Import history",
        )
        .await;
        let bytes = b"backend-owned import history";
        let staged = pack.dir.join("media").join("import-history.opus");
        tokio::fs::write(&staged, bytes).await.unwrap();
        let history_id = pack.begin_media_import().await.unwrap();
        let media = pack
            .add_file_to_import(
                EncodedFile {
                    info: FileInfo::Audio { duration: 1.0 },
                    thumbnail: None,
                    path: staged.clone(),
                    artists: vec![],
                    source_url: None,
                },
                Path::new("import-history.wav"),
                blake3::hash(bytes),
                Some(history_id),
            )
            .await
            .unwrap()
            .unwrap();
        let status = pack.finish_media_import(history_id).await.unwrap();
        assert!(status.can_undo);

        pack.undo().await.unwrap();
        assert!(pack.get_files().await.unwrap().is_empty());
        assert!(staged.exists());
        pack.redo().await.unwrap();
        let restored = pack.get_files().await.unwrap();
        assert_eq!(restored[0].id, media.id);
        assert_eq!(
            pack.get_view()
                .unwrap()
                .get_file_data(media.id)
                .await
                .unwrap()
                .0,
            bytes
        );
    }

    #[tokio::test]
    async fn reimporting_soft_deleted_content_restores_its_existing_media_row() {
        let output = tempdir().unwrap();
        let data = tempdir().unwrap();
        let pack = new_test_pack(
            &output.path().join("restore-duplicate.lwpack"),
            data.path(),
            "Restore duplicate",
        )
        .await;
        let bytes = b"same encoded media";
        let first_path = pack.dir.join("media").join("first.opus");
        tokio::fs::write(&first_path, bytes).await.unwrap();
        let first_history = pack.begin_media_import().await.unwrap();
        let first = pack
            .add_file_to_import(
                EncodedFile {
                    info: FileInfo::Audio { duration: 1.0 },
                    thumbnail: None,
                    path: first_path,
                    artists: vec![],
                    source_url: None,
                },
                Path::new("first.wav"),
                blake3::hash(bytes),
                Some(first_history),
            )
            .await
            .unwrap()
            .unwrap();
        pack.finish_media_import(first_history).await.unwrap();
        pack.remove_files(vec![first.id]).await.unwrap();

        let redundant_path = pack.dir.join("media").join("second.opus");
        tokio::fs::write(&redundant_path, bytes).await.unwrap();
        let second_history = pack.begin_media_import().await.unwrap();
        let restored = pack
            .add_file_to_import(
                EncodedFile {
                    info: FileInfo::Audio { duration: 1.0 },
                    thumbnail: None,
                    path: redundant_path.clone(),
                    artists: vec![],
                    source_url: None,
                },
                Path::new("second.wav"),
                blake3::hash(bytes),
                Some(second_history),
            )
            .await
            .unwrap()
            .unwrap();
        pack.finish_media_import(second_history).await.unwrap();

        assert_eq!(restored.id, first.id);
        assert!(!redundant_path.exists());
        pack.undo().await.unwrap();
        assert!(pack.get_files().await.unwrap().is_empty());
        pack.redo().await.unwrap();
        assert_eq!(pack.get_files().await.unwrap()[0].id, first.id);
    }

    #[tokio::test]
    async fn pending_import_is_not_trimmed_by_concurrent_history_entries() {
        let output = tempdir().unwrap();
        let data = tempdir().unwrap();
        let pack = new_test_pack(
            &output.path().join("pending-trim.lwpack"),
            data.path(),
            "Pending trim",
        )
        .await;
        let history_id = pack.begin_media_import().await.unwrap();
        for index in 0u64..105 {
            pack.set_pack_data("test", index.to_le_bytes().to_vec())
                .await
                .unwrap();
        }
        let pending_exists = pack
            .db_execute(move |connection| {
                connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM history_entries
                         WHERE id = ? AND status = 'pending')",
                        [history_id],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .unwrap();
        assert!(pending_exists);
        pack.finish_media_import(history_id).await.unwrap();
    }

    #[tokio::test]
    async fn undo_does_not_resurrect_garbage_from_a_discarded_redo_branch() {
        let output = tempdir().unwrap();
        let data = tempdir().unwrap();
        let pack = new_test_pack(
            &output.path().join("redo-garbage.lwpack"),
            data.path(),
            "Redo garbage",
        )
        .await;
        let bytes = b"discarded redo bytes";
        let staged = pack.dir.join("media").join("redo-garbage.opus");
        tokio::fs::write(&staged, bytes).await.unwrap();
        let history_id = pack.begin_media_import().await.unwrap();
        pack.add_file_to_import(
            EncodedFile {
                info: FileInfo::Audio { duration: 1.0 },
                thumbnail: None,
                path: staged,
                artists: vec![],
                source_url: None,
            },
            Path::new("redo-garbage.wav"),
            blake3::hash(bytes),
            Some(history_id),
        )
        .await
        .unwrap();
        pack.finish_media_import(history_id).await.unwrap();
        pack.undo().await.unwrap();

        pack.set_metadata(&Metadata {
            name: "A new branch".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        pack.undo().await.unwrap();
        assert!(pack.get_files().await.unwrap().is_empty());
        assert_eq!(
            pack.history_status().await.unwrap().redo_label.as_deref(),
            Some("Edit pack metadata")
        );
    }

    #[tokio::test]
    async fn reopening_reconciles_a_generation_committed_before_the_working_db() {
        let output = tempdir().unwrap();
        let data = tempdir().unwrap();
        let path = output.path().join("generation-recovery.lwpack");
        let pack = new_test_pack(&path, data.path(), "Generation recovery").await;
        let bytes = b"generation recovery bytes";
        let id = insert_staged_audio(&pack, bytes).await;
        pack.save(|_, _| {}).await.unwrap();
        pack.set_metadata(&Metadata {
            name: "Unsaved metadata".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        pack.db_execute(move |conn| {
            conn.execute(
                "UPDATE pack_media SET generation_id = 'stale', \"offset\" = 999999 WHERE media_id = ?",
                [id],
            )?;
            conn.execute(
                "UPDATE editor_state SET archive_generation = 'stale' WHERE singleton = 1",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        drop(pack);

        let reopened = MediaPack::open(path, data.path()).await.unwrap();
        assert_eq!(reopened.name(), "Unsaved metadata");
        assert_eq!(
            reopened
                .get_view()
                .unwrap()
                .get_file_data(id)
                .await
                .unwrap()
                .0,
            bytes
        );
        let generation: (String, String) = reopened
            .db_execute(move |conn| {
                conn.query_row(
                    "SELECT p.generation_id, e.archive_generation FROM pack_media p
                     CROSS JOIN editor_state e WHERE p.media_id = ? AND e.singleton = 1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(Into::into)
            })
            .await
            .unwrap();
        assert_eq!(generation.0, generation.1);
        assert_ne!(generation.0, "stale");
    }

    #[tokio::test]
    async fn destinationless_pack_can_be_recovered_from_its_working_directory() {
        let data = tempdir().unwrap();
        let pack = MediaPack::new_unsaved(data.path(), "Crash draft")
            .await
            .unwrap();
        let id = pack.id();
        drop(pack);

        let recovered = MediaPack::recover_unsaved(data.path(), id).await.unwrap();
        assert_eq!(recovered.name(), "Crash draft");
        assert_eq!(recovered.id(), id);
        assert!(recovered.path().is_none());
        assert!(!recovered.is_saved().await);
    }

    // Insert a minimal audio row backed by a staging file in dir/media/. Reuses the (already
    // unique, UUID-based) staged file name as the DB file_name too, since callers routinely
    // insert several of these against the same pack and `media.file_name` is UNIQUE.
    // Returns the assigned media id.
    async fn insert_staged_audio(pack: &MediaPack, content: &[u8]) -> u64 {
        let filename = Uuid::new_v4().to_string();
        tokio::fs::write(pack.dir.join("media").join(&filename), content)
            .await
            .unwrap();

        let hash_bytes = *blake3::hash(content).as_bytes();
        let filename_clone = filename.clone();
        let length = content.len() as u64;
        pack.db_execute(move |mut conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let id: u64 = tx.query_row(
                "INSERT INTO media (file_name, file_type, duration, hash) \
                 VALUES (?, 'audio', 1.0, ?) RETURNING id",
                params![filename_clone, hash_bytes],
                |row| row.get("id"),
            )?;
            tx.execute(
                "INSERT INTO loose_media(media_id, path, length) VALUES (?, ?, ?)",
                params![id, filename, length],
            )?;
            tx.commit()?;
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
    async fn edits_during_snapshot_save_remain_dirty_and_are_not_written_into_that_snapshot() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("snapshot.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "Before snapshot").await;
        insert_staged_audio(&pack, &vec![7; 2 * 1024 * 1024]).await;

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started_tx = Arc::new(Mutex::new(Some(started_tx)));
        let save = pack.save({
            let started_tx = started_tx.clone();
            move |_, _| {
                if let Some(tx) = started_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        });
        let edit = async {
            started_rx.await.unwrap();
            pack.set_metadata(&Metadata {
                name: "Edited while saving".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        };
        let (save_result, ()) = tokio::join!(save, edit);
        save_result.unwrap();

        assert!(
            !pack.is_saved().await,
            "a post-snapshot edit must remain dirty"
        );
        assert_eq!(pack.name(), "Edited while saving");

        let mut file = File::open(&pack_path).await.unwrap();
        let mut header_buf = [0; HEADER_SIZE];
        file.read_exact(&mut header_buf).await.unwrap();
        let header = Header::from_buf(header_buf).unwrap();
        file.seek(SeekFrom::Start(header.metadata_offset))
            .await
            .unwrap();
        let mut metadata_buf = vec![0; header.metadata_length as usize];
        file.read_exact(&mut metadata_buf).await.unwrap();
        assert_eq!(
            Metadata::from_buf(&metadata_buf).unwrap().name,
            "Before snapshot"
        );
    }

    #[tokio::test]
    async fn discard_waits_for_an_in_flight_generation_save() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("discard-save-race.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "Discard race").await;
        insert_staged_audio(&pack, b"initial").await;
        pack.save(|_, _| {}).await.unwrap();
        insert_staged_audio(&pack, &vec![3; 4 * 1024 * 1024]).await;
        pack.mark_unsaved().await.unwrap();

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started_tx = Arc::new(Mutex::new(Some(started_tx)));
        let save = pack.save({
            let started_tx = started_tx.clone();
            move |_, _| {
                if let Some(tx) = started_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        });
        let discard = async {
            started_rx.await.unwrap();
            pack.discard_changes().await.unwrap();
        };
        let (save_result, ()) = tokio::join!(save, discard);
        save_result.unwrap();

        assert!(pack.is_saved().await);
        assert_eq!(
            pack.db_execute(move |connection| {
                connection
                    .query_row("SELECT COUNT(*) FROM save_sessions", [], |row| {
                        row.get::<_, u64>(0)
                    })
                    .map_err(Into::into)
            })
            .await
            .unwrap(),
            0
        );
        for file in pack.get_files().await.unwrap() {
            pack.get_view()
                .unwrap()
                .get_file_data(file.id)
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn removing_loose_media_during_save_keeps_undo_bytes_alive() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("snapshot-remove.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "Snapshot remove").await;
        let bytes = vec![9; 2 * 1024 * 1024];
        let path = pack.dir.join("media").join("snapshot-remove.opus");
        tokio::fs::write(&path, &bytes).await.unwrap();
        let media = pack
            .add_file(
                EncodedFile {
                    info: FileInfo::Audio { duration: 1.0 },
                    thumbnail: None,
                    path: path.clone(),
                    artists: vec![],
                    source_url: None,
                },
                Path::new("snapshot-remove.wav"),
                blake3::hash(&bytes),
            )
            .await
            .unwrap()
            .unwrap();

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started_tx = Arc::new(Mutex::new(Some(started_tx)));
        let save = pack.save({
            let started_tx = started_tx.clone();
            move |_, _| {
                if let Some(tx) = started_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        });
        let remove = async {
            started_rx.await.unwrap();
            pack.remove_files(vec![media.id]).await.unwrap();
        };
        let (save_result, ()) = tokio::join!(save, remove);
        save_result.unwrap();

        assert!(!pack.is_saved().await);
        pack.undo().await.unwrap();
        let (restored, _) = pack
            .get_view()
            .unwrap()
            .get_file_data(media.id)
            .await
            .unwrap();
        assert_eq!(restored, bytes);
    }

    #[tokio::test]
    async fn generation_save_does_not_block_archive_previews_while_copying() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("generation-preview.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "Generation preview").await;
        let first = insert_staged_audio(&pack, &[1; 1024]).await;
        let second_bytes = vec![2; 2 * 1024 * 1024];
        let second = insert_staged_audio(&pack, &second_bytes).await;
        pack.save(|_, _| {}).await.unwrap();
        pack.remove_files(vec![first]).await.unwrap();

        let view = pack.get_view().unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started_tx = Arc::new(Mutex::new(Some(started_tx)));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let save = pack.save({
            let started_tx = started_tx.clone();
            let gate = gate.clone();
            move |_, _| {
                if let Some(tx) = started_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                    let (released, wake) = &*gate;
                    let mut released = released.lock().unwrap();
                    while !*released {
                        released = wake.wait(released).unwrap();
                    }
                }
            }
        });
        let preview = async {
            started_rx.await.unwrap();
            let result =
                tokio::time::timeout(Duration::from_secs(1), view.get_file_data(second)).await;
            let (released, wake) = &*gate;
            *released.lock().unwrap() = true;
            wake.notify_all();
            let data = result
                .expect("preview should read the old generation while the new one is written")
                .unwrap()
                .0;
            assert_eq!(data, second_bytes);
        };
        let (saved, ()) = tokio::join!(save, preview);
        saved.unwrap();
    }

    #[tokio::test]
    async fn synchronous_in_place_fallback_compacts_in_source_order() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("in-place-fallback.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "In-place fallback").await;
        let contents: Vec<Vec<u8>> = (0..8u8)
            .map(|value| vec![value; 1024 + value as usize * 127])
            .collect();
        let mut ids = Vec::new();
        for content in &contents {
            ids.push(insert_staged_audio(&pack, content).await);
        }
        pack.save(|_, _| {}).await.unwrap();
        pack.remove_files(vec![ids[1], ids[4]]).await.unwrap();

        let snapshot = pack.create_save_snapshot().await.unwrap();
        pack.save_in_place(&snapshot, Arc::new(|_, _| {}))
            .await
            .unwrap();
        drop(pack);

        let reopened = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        let view = reopened.get_view().unwrap();
        for (index, expected) in contents.iter().enumerate() {
            if index == 1 || index == 4 {
                continue;
            }
            assert_eq!(view.get_file_data(ids[index]).await.unwrap().0, *expected);
        }
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
    async fn unsaved_recovery_tolerates_an_interrupted_in_place_archive_tail() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("interrupted-in-place.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "Interrupted in place").await;
        let bytes = b"media remains before the damaged database tail";
        let media_id = insert_staged_audio(&pack, bytes).await;
        pack.save(|_, _| {}).await.unwrap();
        pack.set_pack_data("unsaved", vec![1]).await.unwrap();

        let index_offset = pack.header.read().unwrap().index_offset;
        fs::OpenOptions::new()
            .write(true)
            .open(&pack_path)
            .unwrap()
            .set_len(index_offset)
            .unwrap();
        drop(pack);

        let reopened = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        assert!(!reopened.is_saved().await);
        assert_eq!(
            reopened
                .get_view()
                .unwrap()
                .get_file_data(media_id)
                .await
                .unwrap()
                .0,
            bytes
        );
    }

    #[tokio::test]
    async fn unsaved_recovery_tolerates_a_readable_but_corrupt_archive_index() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("corrupt-in-place-index.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "Corrupt in-place index").await;
        let bytes = b"media remains valid before the overwritten database";
        let media_id = insert_staged_audio(&pack, bytes).await;
        pack.save(|_, _| {}).await.unwrap();
        pack.set_pack_data("unsaved", vec![1]).await.unwrap();

        let (index_offset, index_length) = {
            let header = pack.header.read().unwrap();
            (header.index_offset, header.index_length)
        };
        let mut archive = fs::OpenOptions::new().write(true).open(&pack_path).unwrap();
        archive.seek(SeekFrom::Start(index_offset)).unwrap();
        archive
            .write_all(&vec![0xa5; index_length as usize])
            .unwrap();
        archive.sync_data().unwrap();
        drop(pack);

        let reopened = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        assert!(!reopened.is_saved().await);
        assert_eq!(
            reopened
                .get_view()
                .unwrap()
                .get_file_data(media_id)
                .await
                .unwrap()
                .0,
            bytes
        );
    }

    #[tokio::test]
    async fn damaged_metadata_backup_uses_the_authoritative_editor_database() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "Original Name").await;
        pack.save(|_, _| {}).await.unwrap();
        pack.set_metadata(&Metadata {
            name: "Modified Name".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();
        tokio::fs::write(pack.dir().join("Metadata"), b"damaged")
            .await
            .unwrap();
        drop(pack);

        let recovered = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        assert_eq!(recovered.name(), "Modified Name");
        assert!(!recovered.is_saved().await);
    }

    #[tokio::test]
    async fn add_tags_creates_new_tags_and_associates_them() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
        let file_id = insert_staged_audio(&pack, b"audio").await;

        pack.add_tags(file_id, vec!["kinky".to_string(), "hypno".to_string()])
            .await
            .unwrap();

        let mut tags = pack.get_tags(file_id).await.unwrap();
        tags.sort();
        assert_eq!(tags, vec!["hypno".to_string(), "kinky".to_string()]);

        let mut all_tags = pack.get_all_tags().await.unwrap();
        all_tags.sort();
        assert_eq!(all_tags, vec!["hypno".to_string(), "kinky".to_string()]);

        // Re-adding an already-associated tag (e.g. a second imported file sharing a mood) is a
        // no-op, not a constraint-violation error.
        pack.add_tags(file_id, vec!["kinky".to_string()])
            .await
            .unwrap();
        let mut tags = pack.get_tags(file_id).await.unwrap();
        tags.sort();
        assert_eq!(tags, vec!["hypno".to_string(), "kinky".to_string()]);
    }

    #[tokio::test]
    async fn bulk_tag_edit_updates_every_selected_file() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
        let first = insert_staged_audio(&pack, b"first").await;
        let second = insert_staged_audio(&pack, b"second").await;

        pack.add_tag_to_files(vec![first, second], "shared".to_string())
            .await
            .unwrap();
        assert_eq!(pack.get_tags(first).await.unwrap(), vec!["shared"]);
        assert_eq!(pack.get_tags(second).await.unwrap(), vec!["shared"]);

        pack.remove_tag_from_files(vec![first, second], "shared".to_string())
            .await
            .unwrap();
        assert!(pack.get_tags(first).await.unwrap().is_empty());
        assert!(pack.get_tags(second).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn bulk_remove_and_restore_exceeds_sqlite_variable_limit() {
        const COUNT: u64 = 35_000;
        let output = tempdir().unwrap();
        let data = tempdir().unwrap();
        let pack = new_test_pack(
            &output.path().join("large-removal.lwpack"),
            data.path(),
            "Large removal",
        )
        .await;
        pack.db_execute(|mut conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            {
                let mut insert = tx.prepare(
                    "INSERT INTO media
                     (id, file_name, file_type, duration, hash)
                     VALUES (?, ?, 'audio', 1.0, ?)",
                )?;
                for id in 1..=COUNT {
                    insert.execute(params![
                        id,
                        format!("bulk-{id}.opus"),
                        blake3::hash(&id.to_le_bytes()).as_bytes().to_vec(),
                    ])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();

        let ids = (1..=COUNT).collect::<Vec<_>>();
        pack.remove_files(ids.clone()).await.unwrap();
        assert!(pack.get_files().await.unwrap().is_empty());
        assert!(pack.history_status().await.unwrap().can_undo);
        pack.undo().await.unwrap();
        assert_eq!(pack.get_files().await.unwrap().len() as u64, COUNT);
    }

    #[tokio::test]
    async fn tag_management_updates_associations_and_behaviour_together() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
        let media = insert_staged_audio(&pack, b"tagged").await;
        pack.add_tags(media, vec!["old".to_string()]).await.unwrap();

        let mut behaviour = Behaviour::default();
        behaviour.content.wallpaper_tags = vec!["old".to_string()];
        pack.set_pack_data("behaviour", behaviour.to_json_bytes().unwrap())
            .await
            .unwrap();
        let behaviour = pack
            .rename_tag("old".to_string(), "new".to_string())
            .await
            .unwrap();

        assert_eq!(pack.get_tags(media).await.unwrap(), vec!["new"]);
        assert_eq!(behaviour.content.wallpaper_tags, vec!["new"]);
        let stored =
            Behaviour::from_json_bytes(&pack.get_pack_data("behaviour").await.unwrap().unwrap())
                .unwrap();
        assert_eq!(stored.content.wallpaper_tags, vec!["new"]);

        let behaviour = pack.delete_tag("new".to_string()).await.unwrap();
        assert!(behaviour.content.wallpaper_tags.is_empty());
        assert!(pack.get_tags(media).await.unwrap().is_empty());
        assert!(pack.get_tag_summaries().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn removed_media_can_be_restored_with_its_row_tags_and_bytes_after_save() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("history.lwpack");
        let bytes = b"undoable media bytes";
        let pack = new_test_pack(&pack_path, data_dir.path(), "History").await;
        let id = insert_staged_audio(&pack, bytes).await;
        pack.add_tags(id, vec!["one".into(), "two".into()])
            .await
            .unwrap();
        let original = pack.get_files().await.unwrap().remove(0);

        pack.remove_files(vec![id]).await.unwrap();
        assert!(pack.get_files().await.unwrap().is_empty());
        pack.save(|_, _| {}).await.unwrap();

        let history_tables_in_pack_index: u64 = pack.db_execute(|conn| {
            conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('history_media', 'history_media_tags')", [], |row| row.get(0)).map_err(Into::into)
        }).await.unwrap();
        assert_eq!(history_tables_in_pack_index, 0);

        pack.undo().await.unwrap();
        let restored = pack.get_files().await.unwrap().remove(0);
        assert_eq!(restored.id, original.id);
        assert_eq!(restored.file_name, original.file_name);
        assert_eq!(restored.hash, original.hash);
        let mut tags = pack.get_tags(id).await.unwrap();
        tags.sort();
        assert_eq!(tags, vec!["one", "two"]);
        let (restored_bytes, _) = pack.get_view().unwrap().get_file_data(id).await.unwrap();
        assert_eq!(restored_bytes, bytes);

        pack.remove_files(vec![id]).await.unwrap();
        assert!(pack.get_files().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn purging_evicted_media_history_removes_only_staged_rows_and_bytes() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack = new_test_pack(&tmp.path().join("purge.lwpack"), data_dir.path(), "Purge").await;
        let staged = insert_staged_audio(&pack, b"staged").await;
        let active = insert_staged_audio(&pack, b"active").await;
        let staged_path: String = pack
            .db_execute(move |conn| {
                conn.query_row(
                    "SELECT path FROM loose_media WHERE media_id = ?",
                    params![staged],
                    |row| row.get(0),
                )
                .map_err(Into::into)
            })
            .await
            .unwrap();
        pack.remove_files(vec![staged]).await.unwrap();
        pack.db_execute(|conn| {
            conn.execute("DELETE FROM history_entries", [])?;
            Ok(())
        })
        .await
        .unwrap();
        pack.purge_history_files(vec![staged]).await.unwrap();

        assert!(!pack.dir().join("media").join(staged_path).exists());
        assert_eq!(
            pack.get_files()
                .await
                .unwrap()
                .iter()
                .map(|file| file.id)
                .collect::<Vec<_>>(),
            vec![active]
        );
        assert_eq!(
            pack.get_files()
                .await
                .unwrap()
                .iter()
                .map(|file| file.id)
                .collect::<Vec<_>>(),
            vec![active]
        );
    }

    #[tokio::test]
    async fn undoing_a_merge_recovers_source_target_and_dual_associations_atomically() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack = new_test_pack(&tmp.path().join("tags.lwpack"), data_dir.path(), "Tags").await;
        let source_only = insert_staged_audio(&pack, b"source").await;
        let target_only = insert_staged_audio(&pack, b"target").await;
        let both = insert_staged_audio(&pack, b"both").await;
        pack.add_tags(source_only, vec!["source".into()])
            .await
            .unwrap();
        pack.add_tags(target_only, vec!["target".into()])
            .await
            .unwrap();
        pack.add_tags(both, vec!["source".into(), "target".into()])
            .await
            .unwrap();
        let mut before = Behaviour::default();
        before.content.wallpaper_tags = vec!["source".into()];
        pack.set_pack_data("behaviour", serde_json::to_vec_pretty(&before).unwrap())
            .await
            .unwrap();

        pack.merge_tag("source".into(), "target".into())
            .await
            .unwrap();
        let retained_media: u64 = pack
            .db_execute(move |connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM history_media_refs
                         WHERE history_id = (SELECT id FROM history_entries
                                             WHERE label = 'Merge tags'
                                             ORDER BY sequence DESC LIMIT 1)",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(Into::into)
            })
            .await
            .unwrap();
        assert_eq!(retained_media, 2);
        pack.undo().await.unwrap();

        assert_eq!(pack.get_tags(source_only).await.unwrap(), vec!["source"]);
        assert_eq!(pack.get_tags(target_only).await.unwrap(), vec!["target"]);
        let mut both_tags = pack.get_tags(both).await.unwrap();
        both_tags.sort();
        assert_eq!(both_tags, vec!["source", "target"]);
        let stored =
            Behaviour::from_json_bytes(&pack.get_pack_data("behaviour").await.unwrap().unwrap())
                .unwrap();
        assert_eq!(stored.content.wallpaper_tags, vec!["source"]);
    }

    #[tokio::test]
    async fn undoing_a_deleted_tag_recovers_associations_and_behaviour() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack = new_test_pack(
            &tmp.path().join("delete-tag.lwpack"),
            data_dir.path(),
            "Tags",
        )
        .await;
        let media = insert_staged_audio(&pack, b"tagged").await;
        pack.add_tags(media, vec!["restore-me".into()])
            .await
            .unwrap();
        let mut before = Behaviour::default();
        before.content.splash_tags = vec!["restore-me".into()];
        pack.set_pack_data("behaviour", serde_json::to_vec_pretty(&before).unwrap())
            .await
            .unwrap();
        pack.delete_tag("restore-me".into()).await.unwrap();
        pack.undo().await.unwrap();
        assert_eq!(pack.get_tags(media).await.unwrap(), vec!["restore-me"]);
        let stored =
            Behaviour::from_json_bytes(&pack.get_pack_data("behaviour").await.unwrap().unwrap())
                .unwrap();
        assert_eq!(stored.content.splash_tags, vec!["restore-me"]);
    }

    #[tokio::test]
    async fn closing_a_saved_pack_removes_staged_history_files() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack = new_test_pack(
            &tmp.path().join("cleanup.lwpack"),
            data_dir.path(),
            "Cleanup",
        )
        .await;
        let id = insert_staged_audio(&pack, b"temporary").await;
        pack.remove_files(vec![id]).await.unwrap();
        pack.save(|_, _| {}).await.unwrap();
        let working_dir = pack.dir().to_path_buf();
        assert!(working_dir.exists());
        drop(pack);
        assert!(!working_dir.exists());
    }

    #[tokio::test]
    async fn set_pack_data_round_trips_a_named_blob() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
        pack.set_pack_data("behaviour", b"first".to_vec())
            .await
            .unwrap();
        // A repeat write for the same name replaces it, rather than erroring or duplicating.
        pack.set_pack_data("behaviour", b"second".to_vec())
            .await
            .unwrap();

        let blob: Vec<u8> = pack
            .db_execute(|conn| {
                conn.query_row(
                    "SELECT blob FROM pack_data WHERE name = 'behaviour'",
                    [],
                    |row| row.get(0),
                )
                .map_err(Into::into)
            })
            .await
            .unwrap();
        assert_eq!(blob, b"second");
        assert!(!pack.is_saved().await);
    }

    /// Exercises the exact path the pack editor's `get_behaviour`/`set_behaviour` Tauri commands
    /// use (`Behaviour::to_json_bytes`/`from_json_bytes` over `set_pack_data`/`get_pack_data`),
    /// with a document touching every new Content/Experience tab section -- content groups,
    /// captions, web links, and an Experience section with a timeline level -- through a real
    /// save + reopen, not just an in-memory round-trip.
    #[tokio::test]
    async fn content_and_experience_behaviour_survives_save_and_reopen() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let mut behaviour = Behaviour::new();
        behaviour.content.content_groups.push(ContentGroup {
            id: "kinky".to_string(),
            label: "Kinky".to_string(),
            description: Some("Kinky-tagged content".to_string()),
            tags: vec!["kinky".to_string()],
            enabled_by_default: true,
        });
        behaviour.content.captions.push(TextItem {
            text: "Obey.".to_string(),
            tags: vec!["kinky".to_string()],
        });
        behaviour.content.web_links.push(WebLink {
            url: "https://duckduckgo.com/?q=".to_string(),
            args: vec!["edgeware packs".to_string()],
            tags: vec![],
        });
        behaviour.content.wallpaper_tags = vec!["bg".to_string()];
        behaviour.experience = Some(Experience {
            timeline: Timeline {
                levels: vec![
                    Level {
                        at_seconds: 0.0,
                        at_popups: None,
                        anchors: FrequencyAnchors {
                            popup: Some(30.0),
                            web: None,
                            notification: None,
                            prompt: None,
                            subliminal: None,
                        },
                        design: DesignValues::default(),
                        tags: None,
                        wallpaper_tags: None,
                    },
                    Level {
                        at_seconds: 300.0,
                        at_popups: None,
                        anchors: FrequencyAnchors {
                            popup: Some(45.0),
                            web: None,
                            notification: None,
                            prompt: None,
                            subliminal: None,
                        },
                        design: DesignValues::default(),
                        tags: Some(vec!["kinky".to_string()]),
                        wallpaper_tags: None,
                    },
                ],
            },
        });

        let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
        assert_eq!(pack.get_pack_data("behaviour").await.unwrap(), None);

        pack.set_pack_data("behaviour", behaviour.to_json_bytes().unwrap())
            .await
            .unwrap();
        assert!(!pack.is_saved().await);
        pack.save(|_, _| {}).await.unwrap();
        drop(pack);

        let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        let blob = pack2.get_pack_data("behaviour").await.unwrap().unwrap();
        let decoded = Behaviour::from_json_bytes(&blob).unwrap();
        assert_eq!(decoded, behaviour);
    }

    #[tokio::test]
    async fn get_pack_data_returns_none_when_absent_and_the_written_blob_otherwise() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
        assert_eq!(pack.get_pack_data("behaviour").await.unwrap(), None);

        pack.set_pack_data("behaviour", b"the-blob".to_vec())
            .await
            .unwrap();
        assert_eq!(
            pack.get_pack_data("behaviour").await.unwrap(),
            Some(b"the-blob".to_vec())
        );
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

    #[tokio::test]
    async fn a_missing_loose_source_is_dropped_without_aborting_other_media() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("missing-loose.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "Missing loose").await;
        let missing = insert_staged_audio(&pack, b"will disappear").await;
        pack.set_source_url(missing, Some("https://example.com/missing".into()))
            .await
            .unwrap();
        let survivor_bytes = b"must still save";
        let survivor = insert_staged_audio(&pack, survivor_bytes).await;
        let missing_path: String = pack
            .db_execute(move |conn| {
                conn.query_row(
                    "SELECT path FROM loose_media WHERE media_id = ?",
                    [missing],
                    |row| row.get(0),
                )
                .map_err(Into::into)
            })
            .await
            .unwrap();
        tokio::fs::remove_file(pack.dir().join("media").join(missing_path))
            .await
            .unwrap();

        pack.save(|_, _| {}).await.unwrap();
        let files = pack.get_files().await.unwrap();
        assert_eq!(
            files.iter().map(|file| file.id).collect::<Vec<_>>(),
            vec![survivor]
        );
        assert_eq!(
            pack.get_view()
                .unwrap()
                .get_file_data(survivor)
                .await
                .unwrap()
                .0,
            survivor_bytes
        );
        let retained: (bool, bool) = pack
            .db_execute(move |connection| {
                connection
                    .query_row(
                        "SELECT m.deleted,
                                EXISTS(SELECT 1 FROM history_media_refs h WHERE h.media_id = m.id)
                         FROM media m WHERE m.id = ?",
                        [missing],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(Into::into)
            })
            .await
            .unwrap();
        assert_eq!(retained, (true, true));
    }

    // Exercises compact generation layout after scattered deletions. Varying
    // content and lengths expose a bad run boundary, offset, or truncation.
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
        // writer has to shift several contiguous runs by different amounts.
        let deleted_indices = [2usize, 3, 9, 15, 20];
        let deleted_ids: Vec<u64> = deleted_indices.iter().map(|&i| ids[i]).collect();
        pack.remove_files(deleted_ids).await.unwrap();

        // Second save creates a gapless replacement generation.
        pack.save(|_, _| {}).await.unwrap();
        let expected_index_offset = HEADER_SIZE as u64
            + contents
                .iter()
                .enumerate()
                .filter(|(index, _)| !deleted_indices.contains(index))
                .map(|(_, content)| content.len() as u64)
                .sum::<u64>();
        assert_eq!(
            pack.header.read().unwrap().index_offset,
            expected_index_offset,
            "generation saves must leave no gaps between embedded media"
        );
        drop(pack);

        let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        let view = pack2.get_view().unwrap();

        for (i, content) in contents.iter().enumerate() {
            if deleted_indices.contains(&i) {
                continue;
            }
            let (data, _) = view.get_file_data(ids[i]).await.unwrap();
            assert_eq!(
                &data, content,
                "content mismatch for surviving file index {i}"
            );
        }
    }

    // Exercises the sequential newly-staged-file layout: varying lengths expose
    // any bad cumulative offset or truncation before the snapshot tail is written.
    #[tokio::test]
    async fn many_new_files_survive_sequential_first_save() {
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

    /// Manual reproduction for high-file-count generation-save performance without running the
    /// encoder/import pipeline. Run with:
    /// `cargo test -p lewdware-pack-editor profile_generation_save_many_staged_files -- --ignored --nocapture`
    /// Override the defaults with `PACK_EDITOR_PERF_MEDIA_COUNT`,
    /// `PACK_EDITOR_PERF_MEDIA_BYTES`, and `PACK_EDITOR_PERF_ROOT`.
    #[tokio::test]
    #[ignore = "manual save performance profile"]
    async fn profile_generation_save_many_staged_files() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_test_writer()
            .try_init();
        let count = std::env::var("PACK_EDITOR_PERF_MEDIA_COUNT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(25_000);
        let bytes_per_file = std::env::var("PACK_EDITOR_PERF_MEDIA_BYTES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(64 * 1024)
            .max(std::mem::size_of::<u64>());
        let total_media_bytes = count as u64 * bytes_per_file as u64;
        let perf_root = std::env::var_os("PACK_EDITOR_PERF_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap().join("target"));
        fs::create_dir_all(&perf_root).unwrap();
        // Do not use tempfile's default /tmp: it is commonly a tmpfs and would make the archive
        // sync timings and filesystem cleanup unrealistically cheap.
        let tmp = tempfile::tempdir_in(&perf_root).unwrap();
        let data_dir = tempfile::tempdir_in(&perf_root).unwrap();
        let pack_path = tmp.path().join("many-staged-files.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "Many staged files").await;
        // Establish a normal existing archive before adding the profiled files.
        pack.save(|_, _| {}).await.unwrap();

        let seed_started = Instant::now();
        let media_dir = pack.dir.join("media");
        pack.db_execute(move |mut connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut bytes = vec![0xa5; bytes_per_file];
            {
                let mut insert_media = tx.prepare(
                    "INSERT INTO media (file_name, file_type, duration, hash)
                     VALUES (?, 'audio', 1.0, ?)",
                )?;
                let mut insert_loose =
                    tx.prepare("INSERT INTO loose_media(media_id, path, length) VALUES (?, ?, ?)")?;
                for index in 0..count {
                    let filename = format!("perf-{index:08}.opus");
                    bytes[..std::mem::size_of::<u64>()]
                        .copy_from_slice(&(index as u64).to_le_bytes());
                    fs::write(media_dir.join(&filename), &bytes)?;
                    let mut hash = [0u8; 32];
                    hash[..8].copy_from_slice(&(index as u64).to_le_bytes());
                    insert_media.execute(params![filename, hash.as_slice()])?;
                    let id = tx.last_insert_rowid();
                    insert_loose.execute(params![id, filename, bytes.len() as u64])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
        pack.mark_unsaved().await.unwrap();
        eprintln!(
            "seeded {count} staged files ({:.2} GiB total, {bytes_per_file} bytes each) in {:.3}s",
            total_media_bytes as f64 / 1024.0_f64.powi(3),
            seed_started.elapsed().as_secs_f64()
        );

        let progress_finished = Arc::new(Mutex::new(None::<Instant>));
        let progress_for_callback = progress_finished.clone();
        let save_started = Instant::now();
        pack.save(move |completed, total| {
            if total > 0 && completed == total {
                progress_for_callback
                    .lock()
                    .unwrap()
                    .get_or_insert_with(Instant::now);
            }
        })
        .await
        .unwrap();
        let elapsed = save_started.elapsed();
        let after_progress = progress_finished
            .lock()
            .unwrap()
            .map(|finished| finished.elapsed())
            .unwrap_or_default();
        eprintln!(
            "saved {count} staged files ({:.2} GiB) in {:.3}s; {:.3}s elapsed after file progress completed",
            total_media_bytes as f64 / 1024.0_f64.powi(3),
            elapsed.as_secs_f64(),
            after_progress.as_secs_f64()
        );
        let garbage_collection_started = Instant::now();
        pack.process_pending_file_deletions().await.unwrap();
        eprintln!(
            "background loose-file garbage collection completed in {:.3}s",
            garbage_collection_started.elapsed().as_secs_f64()
        );
        pack.close().await;
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
            let remaining =
                tokio::time::timeout(std::time::Duration::from_secs(10), pack.get_files())
                    .await
                    .unwrap()
                    .unwrap();
            if remaining.len() < 20 {
                break;
            }
            let to_delete: Vec<u64> = remaining.iter().step_by(9).map(|f| f.id).collect();
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
            artists: vec![],
            source_url: None,
            path: encoded_path_1,
        };

        let encoded_path_2 = pack.dir.join("media").join("upload-2");
        tokio::fs::write(&encoded_path_2, content).await.unwrap();
        let encoded_2 = EncodedFile {
            info: FileInfo::Audio { duration: 1.0 },
            thumbnail: None,
            artists: vec![],
            source_url: None,
            path: encoded_path_2,
        };

        let first = pack
            .add_file(encoded_1, Path::new("a.wav"), hash)
            .await
            .unwrap();
        assert!(
            first.is_some(),
            "first upload of new content should succeed"
        );

        let second = pack
            .add_file(encoded_2, Path::new("b.wav"), hash)
            .await
            .unwrap();
        assert!(
            second.is_none(),
            "duplicate hash should be rejected, not inserted"
        );
        assert!(
            !pack.dir.join("media").join("upload-2").exists(),
            "an unreferenced encoded file should be deleted on final drop"
        );

        let files = pack.get_files().await.unwrap();
        assert_eq!(
            files.len(),
            1,
            "only one row should exist for the duplicate hash"
        );
    }

    #[tokio::test]
    async fn referenced_loose_file_survives_pack_drop_and_orphans_are_swept_on_reopen() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("leases.lwpack");
        let pack = new_test_pack(&pack_path, data_dir.path(), "Leases").await;

        let encoded_path = pack.dir.join("media").join("referenced.opus");
        tokio::fs::write(&encoded_path, b"referenced")
            .await
            .unwrap();
        let media = pack
            .add_file(
                EncodedFile {
                    info: FileInfo::Audio { duration: 1.0 },
                    thumbnail: None,
                    path: encoded_path.clone(),
                    artists: vec![],
                    source_url: None,
                },
                Path::new("referenced.wav"),
                blake3::hash(b"referenced"),
            )
            .await
            .unwrap()
            .unwrap();

        let orphan_path = pack.dir.join("media").join("orphan.opus");
        tokio::fs::write(&orphan_path, b"orphan").await.unwrap();
        let abandoned_staging = pack.dir.join("pack-editor-upload-abandoned");
        tokio::fs::create_dir(&abandoned_staging).await.unwrap();
        tokio::fs::write(abandoned_staging.join("partial.opus"), b"partial")
            .await
            .unwrap();

        drop(pack);
        assert!(
            encoded_path.exists(),
            "a durable live row must survive application teardown"
        );

        let reopened = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
        assert!(encoded_path.exists());
        assert!(!orphan_path.exists());
        assert!(!abandoned_staging.exists());
        assert_eq!(reopened.get_files().await.unwrap()[0].id, media.id);
        reopened.remove_files(vec![media.id]).await.unwrap();
        assert!(
            encoded_path.exists(),
            "undo history must retain the loose file"
        );
        reopened
            .db_execute(|conn| {
                conn.execute("DELETE FROM history_entries", [])?;
                Ok(())
            })
            .await
            .unwrap();
        reopened.purge_history_files(vec![media.id]).await.unwrap();
        assert!(
            !encoded_path.exists(),
            "history eviction must release the final durable owner"
        );
    }

    // Two distinct uploads that happen to share a requested file name -- unlike a hash collision
    // (rejected outright), this should still add the file, just under an adjusted name.
    #[tokio::test]
    async fn colliding_file_name_on_add_is_renamed_not_rejected() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "Names").await;

        let encoded_path_1 = pack.dir.join("media").join("upload-1");
        tokio::fs::write(&encoded_path_1, b"first").await.unwrap();
        let encoded_1 = EncodedFile {
            info: FileInfo::Audio { duration: 1.0 },
            thumbnail: None,
            artists: vec![],
            source_url: None,
            path: encoded_path_1,
        };

        let encoded_path_2 = pack.dir.join("media").join("upload-2");
        tokio::fs::write(&encoded_path_2, b"second").await.unwrap();
        let encoded_2 = EncodedFile {
            info: FileInfo::Audio { duration: 1.0 },
            thumbnail: None,
            artists: vec![],
            source_url: None,
            path: encoded_path_2,
        };

        let encoded_path_3 = pack.dir.join("media").join("upload-3");
        tokio::fs::write(&encoded_path_3, b"third").await.unwrap();
        let encoded_3 = EncodedFile {
            info: FileInfo::Audio { duration: 1.0 },
            thumbnail: None,
            artists: vec![],
            source_url: None,
            path: encoded_path_3,
        };

        let first = pack
            .add_file(encoded_1, Path::new("clip.wav"), blake3::hash(b"first"))
            .await
            .unwrap()
            .unwrap();
        let second = pack
            .add_file(encoded_2, Path::new("clip.wav"), blake3::hash(b"second"))
            .await
            .unwrap()
            .unwrap();
        let third = pack
            .add_file(encoded_3, Path::new("clip.wav"), blake3::hash(b"third"))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(first.file_name, "clip.wav");
        assert_eq!(second.file_name, "clip (1).wav");
        assert_eq!(third.file_name, "clip (2).wav");

        let files = pack.get_files().await.unwrap();
        assert_eq!(files.len(), 3, "all three distinct files should be added");
    }

    // Unlike add_file's auto-rename, an explicit rename to an existing name is a deliberate user
    // action and should be rejected with a clear error, not silently adjusted.
    #[tokio::test]
    async fn set_title_rejects_a_colliding_name() {
        let tmp = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let pack_path = tmp.path().join("test.lwpack");

        let pack = new_test_pack(&pack_path, data_dir.path(), "Rename").await;

        let encoded_path_1 = pack.dir.join("media").join("upload-1");
        tokio::fs::write(&encoded_path_1, b"first").await.unwrap();
        let encoded_1 = EncodedFile {
            info: FileInfo::Audio { duration: 1.0 },
            thumbnail: None,
            artists: vec![],
            source_url: None,
            path: encoded_path_1,
        };
        let first = pack
            .add_file(encoded_1, Path::new("a.wav"), blake3::hash(b"first"))
            .await
            .unwrap()
            .unwrap();

        let encoded_path_2 = pack.dir.join("media").join("upload-2");
        tokio::fs::write(&encoded_path_2, b"second").await.unwrap();
        let encoded_2 = EncodedFile {
            info: FileInfo::Audio { duration: 1.0 },
            thumbnail: None,
            artists: vec![],
            source_url: None,
            path: encoded_path_2,
        };
        let second = pack
            .add_file(encoded_2, Path::new("b.wav"), blake3::hash(b"second"))
            .await
            .unwrap()
            .unwrap();

        let err = pack
            .set_title(second.id, first.file_name.clone())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already exists"));

        // The rejected rename must not have applied.
        let files = pack.get_files().await.unwrap();
        let renamed = files.iter().find(|f| f.id == second.id).unwrap();
        assert_eq!(renamed.file_name, "b.wav");
    }

    /// One step of a random add/delete/save sequence exercised by
    /// `save_preserves_every_surviving_files_bytes` below.
    #[derive(Debug, Clone)]
    enum Op {
        /// Stage a new file with this content.
        Add(Vec<u8>),
        /// Delete the file at this index (mod the current number of alive files) among
        /// currently-alive files, in the order they were added. A no-op if nothing is alive.
        Delete(usize),
        /// Save (compact) the pack.
        Save,
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            prop::collection::vec(any::<u8>(), 0..64).prop_map(Op::Add),
            any::<usize>().prop_map(Op::Delete),
            Just(Op::Save),
        ]
    }

    /// Like `insert_staged_audio`, but every call is guaranteed a unique hash and file name (the
    /// `media` table has a UNIQUE constraint on both) by appending a never-repeated counter to
    /// the content, and reusing the already-unique on-disk staged file name (a fresh UUID) as the
    /// DB `file_name` too, regardless of what content the caller passes in -- proptest-generated
    /// byte vectors are short and likely to collide otherwise.
    async fn add_staged_file(pack: &MediaPack, content: &[u8], unique: u64) -> (u64, Vec<u8>) {
        let mut content = content.to_vec();
        content.extend_from_slice(&unique.to_le_bytes());

        let filename = Uuid::new_v4().to_string();
        tokio::fs::write(pack.dir.join("media").join(&filename), &content)
            .await
            .unwrap();

        let hash_bytes = *blake3::hash(&content).as_bytes();
        let length = content.len() as u64;
        let filename_clone = filename.clone();
        let id = pack
            .db_execute(move |mut conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let id: u64 = tx.query_row(
                    "INSERT INTO media (file_name, file_type, duration, hash) \
                     VALUES (?, 'audio', 1.0, ?) RETURNING id",
                    params![filename_clone, hash_bytes],
                    |row| row.get("id"),
                )?;
                tx.execute(
                    "INSERT INTO loose_media(media_id, path, length) VALUES (?, ?, ?)",
                    params![id, filename, length],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .unwrap();

        (id, content)
    }

    /// Reads back a surviving file's current on-disk bytes -- whether it's still a loose staged
    /// file (`path` set) or already compacted into the pack file (`offset`/`length` set) -- via
    /// the same code path the app's file preview/download uses.
    async fn read_back(pack: &MediaPack, id: u64) -> Vec<u8> {
        pack.get_view().unwrap().get_file_data(id).await.unwrap().0
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 32, .. ProptestConfig::default() })]

        /// The in-place save/compaction algorithm (`write_files`) is the one place this crate
        /// does concurrent, order-sensitive byte-level file surgery -- the README documents a
        /// real (if narrow) corruption window around it. Drives a random sequence of
        /// add/delete/save operations against a real `MediaPack`, mirrors the expected state in a
        /// plain in-memory model, and after every save (and once more at the end) checks that
        /// every surviving file's bytes on disk still match exactly what was staged for it.
        #[test]
        fn save_preserves_every_surviving_files_bytes(ops in prop::collection::vec(op_strategy(), 1..40)) {
            let rt = tokio::runtime::Runtime::new().unwrap();

            let result: Result<(), TestCaseError> = rt.block_on(async {
                let tmp = tempdir().unwrap();
                let data_dir = tempdir().unwrap();
                let pack_path = tmp.path().join("test.lwpack");
                let pack = new_test_pack(&pack_path, data_dir.path(), "Fuzz").await;

                let mut model: HashMap<u64, Vec<u8>> = HashMap::new();
                let mut alive: Vec<u64> = Vec::new();
                let mut unique = 0u64;

                for op in ops {
                    match op {
                        Op::Add(content) => {
                            let (id, content) = add_staged_file(&pack, &content, unique).await;
                            unique += 1;
                            model.insert(id, content);
                            alive.push(id);
                        }
                        Op::Delete(idx) => {
                            if !alive.is_empty() {
                                let id = alive.remove(idx % alive.len());
                                pack.remove_files(vec![id]).await.unwrap();
                                model.remove(&id);
                            }
                        }
                        Op::Save => {
                            pack.save(|_, _| {}).await.unwrap();

                            for (&id, expected) in &model {
                                let actual = read_back(&pack, id).await;
                                prop_assert_eq!(&actual, expected, "mismatch for media id {} after save", id);
                            }
                        }
                    }
                }

                // The sequence doesn't necessarily end with a `Save` -- do one final save/check so
                // every run actually exercises compaction at least once.
                pack.save(|_, _| {}).await.unwrap();
                for (&id, expected) in &model {
                    let actual = read_back(&pack, id).await;
                    prop_assert_eq!(&actual, expected, "mismatch for media id {} after final save", id);
                }

                Ok(())
            });

            result?;
        }
    }
}
