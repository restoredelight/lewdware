mod behaviour;
mod labels;
mod media;
mod save;
mod view;

use save::process_pending_file_deletions;
#[cfg(test)]
use view::resolve_range;

use std::{
    fs::{self, create_dir_all},
    io::{self, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, RwLock as StdRwLock,
    },
    thread::available_parallelism,
    time::Instant,
};

use anyhow::{anyhow, bail, Result};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::vtab::array;
use serde::{Deserialize, Serialize};
use shared::{
    behaviour::Behaviour,
    encode::FileInfo,
    read_pack::{Header, Metadata, HEADER_SIZE},
    tags,
};
use tokio::{
    fs::{remove_file, File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{mpsc, oneshot, Mutex as AsyncMutex, RwLock},
    task::{spawn_blocking, JoinHandle},
};
use uuid::Uuid;

#[cfg(test)]
use rusqlite::{types::Value, vtab::array::Array};
/// Only `media_id_array` needs these, and it is test-only.
#[cfg(test)]
use std::rc::Rc;

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

/// Every table where a behaviour document references a tag. Merging a tag re-points each of them;
/// deleting one relies on their `ON DELETE CASCADE` instead.
const TAG_JOIN_TABLES: &[(&str, &str)] = &[
    ("behaviour_stage_tag", "stage_id"),
    ("behaviour_text_item_tag", "item_id"),
    ("behaviour_web_link_tag", "link_id"),
    ("behaviour_content_group_tag", "group_id"),
];

/// What one behaviour edit produced -- see [`MediaPack::edit_behaviour`].
#[derive(Serialize, Clone, Debug)]
pub struct BehaviourEdit {
    pub behaviour: Behaviour,
    /// Media the edit retired that turned out to be scenery nothing else referenced, so it left
    /// the pack with the edit. The front end drops these from its media grid.
    pub deleted_ids: Vec<u64>,
    /// Tags the edit's [`TagAction`]s took out of the pack, so the grid can drop them from the
    /// files that carried them without refetching. Reported rather than assumed: a retirement is
    /// conditional on nothing claiming the tag, and that is decided here.
    pub removed_tags: Vec<String>,
    /// `[from, to]` for each rename that actually happened, for the same reason: a rename onto a
    /// name the pack already has is skipped rather than turned into a merge.
    pub renamed_tags: Vec<(String, String)>,
}

/// A tag edit that belongs to the same author action as a behaviour patch.
///
/// The tag half of `retiring`, and there for the same reason: renaming a stage renames the tag it
/// owns, and dropping a stage retires it, so the two halves have to be one transaction or undo
/// would leave a stage renamed and its tag not (or the reverse). See
/// `behaviour-design/default-mode-v2.md`, "Stage tags".
///
/// Ordering is by *phase*, not by position in the list, because the two groups ask their questions
/// of different documents: [`TagAction::Apply`], [`TagAction::Remove`] and [`TagAction::Rename`]
/// run before the patch is written, so media membership and the stage association change in one
/// transaction (and a rename afterwards would be undone on the spot, since writing a tag list
/// re-creates any name it does not find). Retirement and deletion run after it, because "does
/// anything still claim this tag?" is a question about the document the edit produced.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TagAction {
    /// Puts `tag` on media, creating the tag if the pack does not have it.
    ///
    /// `media` of `None` means every file in the pack -- the seeding rule, which keeps a stage
    /// showing what it showed a moment before its tags were switched on. A list rather than ids for
    /// that case would mean shipping every id in the pack across the IPC boundary to describe one
    /// checkbox.
    Apply {
        tag: String,
        media: Option<Vec<u64>>,
    },
    /// Removes `tag` from the named media.
    ///
    /// This is part of a behaviour edit (rather than the ordinary tag command) when leaving one
    /// stage first has to give neighbouring stages replacement tags. Keeping the additions, the
    /// stage patch and the removal in one transaction makes the whole toggle one undo entry.
    Remove { tag: String, media: Vec<u64> },
    /// Renames `from` to `to`, and does nothing at all if the pack already has a tag called `to`.
    ///
    /// Skipped rather than refused: this rides along with a stage rename, and failing the whole
    /// edit would mean the author's stage cannot be renamed because a *tag* name collides. The
    /// stage keeps the tag it had, under the name it had, which is the honest outcome.
    Rename { from: String, to: String },
    /// Deletes `tag` if nothing claims it: no media carries it, and no stage, text pool, web link
    /// or content group names it in the patched document. Otherwise leaves it entirely alone.
    ///
    /// The condition is re-checked here rather than trusted from the front end, because the front
    /// end asked it of the document *before* the edit.
    RetireIfUnclaimed { tag: String },
    /// Deletes `tag` outright, associations and all -- the author having been told what it is on
    /// and having asked for it anyway.
    Delete { tag: String },
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
    Duplicate(u64),
}

/// What adding a file actually did.
///
/// A bulk import treats `Duplicate` as a skip, but the media slots need the file it collided
/// with: "put this image in the wallpaper slot" is answered just as well by a copy the pack
/// already has as by a fresh import, and importing it twice would be the wrong answer.
pub enum AddOutcome {
    Added(MediaFile),
    /// The pack already held these bytes, as this file.
    Duplicate(MediaFile),
}

/// One file joining a media slot.
///
/// Carries `new_to_pack` because that is the whole of what decides `__lewdware-non-popup`, and it
/// is not a fact the transaction can recover for itself: a file imported a second ago and one the
/// author imported last week look identical in the database.
#[derive(Debug, Clone, Copy)]
pub struct MediaAddition {
    pub id: u64,
    /// Whether this operation is what brought the file into the pack.
    ///
    /// Only then is the file marked as scenery. A file the pack already held is ordinary media the
    /// author put there on purpose: marking it would take it out of the media grid (which lists
    /// only popup media) and, worse, hand it to the scenery lifecycle -- emptying the slot
    /// deletes whatever is still marked, so a wallpaper picked from a file already in the
    /// pack would take that file with it. See `clear_media_slot`.
    pub new_to_pack: bool,
}

impl From<&AddOutcome> for MediaAddition {
    fn from(outcome: &AddOutcome) -> Self {
        Self {
            id: outcome.file().id,
            new_to_pack: matches!(outcome, AddOutcome::Added(_)),
        }
    }
}

impl AddOutcome {
    pub fn file(&self) -> &MediaFile {
        match self {
            Self::Added(file) | Self::Duplicate(file) => file,
        }
    }

    pub fn into_added(self) -> Option<MediaFile> {
        match self {
            Self::Added(file) => Some(file),
            Self::Duplicate(_) => None,
        }
    }
}

struct Lock {
    file: fs::File,
    path: PathBuf,
}

impl Lock {
    fn new(path: PathBuf) -> Result<Self> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
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
            if err.kind() != io::ErrorKind::NotFound {
                tracing::error!("{err}");
            }
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

/// A byte range of one media file, opened and positioned but not yet read -- the body of a
/// preview response, handed over as a stream rather than a buffer.
///
/// Buffering was the old shape, and it forced a cap on how much of a file one response could
/// carry (memory), which in turn made every response to an open-ended request a short one. Media
/// clients do not treat a short response as an invitation to ask for the rest: GStreamer -- what
/// WebKitGTK plays `<video>` through -- issues a range-less GET, reads the body to its end, and
/// stops there. A capped response therefore *was* the whole video as far as the player was
/// concerned: it would show the full duration (`+faststart` puts the header first) and stop dead
/// part-way through. Streaming lets a response be as long as the range asked for while the memory
/// it costs stays one chunk.
pub struct FileRangeStream {
    file: File,
    pub start: u64,
    pub end: u64,
    pub total_size: u64,
    /// The media's content hash, hex-encoded, for the response's `ETag`.
    ///
    /// A client that has part of a resource and asks for the rest needs a validator to know the
    /// partial response belongs to the same entity it already holds (RFC 9110 §8.8). Without one,
    /// a range response is unverifiable -- and this server hands out exactly that pattern: a full
    /// `200` for the first read, then `206`s as the player seeks around.
    pub entity_tag: String,
}

impl FileRangeStream {
    pub fn len(&self) -> u64 {
        self.end - self.start
    }

    /// The range's bytes, read a chunk at a time.
    ///
    /// The file is already positioned at `start` and is read from that point regardless of what
    /// happens to the pack file underneath: an ordinary save writes a new generation and renames
    /// it over the old one, which leaves this handle reading the generation the response began
    /// with -- self-consistent, and no reason to make a save wait for a video someone is still
    /// watching. (The in-place save fallback, which only runs when the disk is full, does rewrite
    /// those bytes; a preview open across one can glitch, which is what the editor's "Saving pack
    /// -- playback may pause briefly" banner is for.)
    pub fn into_stream(self) -> impl futures::Stream<Item = io::Result<Vec<u8>>> + Send {
        const CHUNK: u64 = 256 * 1024;
        let length = self.len();
        futures::stream::try_unfold((self.file, length), |(mut file, remaining)| async move {
            if remaining == 0 {
                return Ok(None);
            }
            let mut buffer = vec![0u8; remaining.min(CHUNK) as usize];
            let read = file.read(&mut buffer).await?;
            if read == 0 {
                // Short of the length the range promised -- the file was replaced under us.
                // Ending here beats spinning on a reader that will never produce again.
                return Ok(None);
            }
            buffer.truncate(read);
            Ok(Some((buffer, (file, remaining - read as u64))))
        })
    }
}

pub struct Range {
    pub start: Option<u64>,
    pub end: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct InvalidRange;

impl std::fmt::Display for InvalidRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid range")
    }
}

impl std::error::Error for InvalidRange {}

fn draft_lock_path(data_dir: &Path, id: Uuid) -> PathBuf {
    data_dir
        .join("Lewdware Pack Editor")
        .join(format!("{id}.lock"))
}

impl MediaPack {
    pub(crate) fn remove_recoverable_draft(data_dir: &Path, id: Uuid) -> Result<()> {
        let dir = data_dir.join("Lewdware Pack Editor").join(id.to_string());
        if !dir.exists() {
            return Ok(());
        }
        let _lock = Lock::new(draft_lock_path(data_dir, id))?;
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

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
        let lock = Lock::new(draft_lock_path(data_dir, id))?;
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
            _lock: Some(lock),
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
        // Saved packs are protected by their adjacent pack-file lock. Drafts have no pack
        // path, so use a sibling lock keyed by their working directory identity.
        let lock = match lock {
            Some(lock) => Some(lock),
            None => Some(Lock::new(draft_lock_path(data_dir, header.id))?),
        };

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
                editor_db::import_runtime(&mut conn, runtime, &metadata_for_db)?;
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
    }

    pub async fn close_and_discard(&self) {
        self.close().await;
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
}

impl Drop for MediaPack {
    fn drop(&mut self) {
        let explicitly_discarded = self.delete_on_drop.load(Ordering::Relaxed);
        let safely_saved =
            self.saved.load(Ordering::Relaxed) && !self.dir.join("UNSAVED").is_file();
        if explicitly_discarded || safely_saved {
            if let Err(err) = fs::remove_dir_all(&self.dir) {
                tracing::error!("{err}");
            }
        }
    }
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

/// Refuses a tag in the reserved namespace on every command an *author* can reach.
///
/// Guarding the writes rather than the UI is what makes the namespace an invariant: the editor
/// applies these tags itself (through `add_tags`, which is deliberately not guarded -- it's how
/// media import and the media slots write the marker), and nothing else can. See `shared::tags`.
fn reject_managed_tag(tag: &str) -> Result<()> {
    if tags::is_managed(tag) {
        bail!("\"{tag}\" is reserved for Lewdware's own use. Pick a different name.");
    }
    Ok(())
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
    // There is no directory handle to fsync off Unix, so the argument goes unread there.
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn is_storage_full(error: &anyhow::Error) -> bool {
    // ENOSPC is out of space and EDQUOT is over quota -- the same thing as far as a save is
    // concerned. 112 is Windows' ERROR_DISK_FULL, which `raw_os_error` reports directly and
    // libc has no name for. EDQUOT exists only on Unix, so it cannot be named unconditionally.
    #[cfg(unix)]
    const FULL: &[i32] = &[libc::ENOSPC, libc::EDQUOT, 112];
    #[cfg(not(unix))]
    const FULL: &[i32] = &[libc::ENOSPC, 112];

    error.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .and_then(io::Error::raw_os_error)
            .is_some_and(|code| FULL.contains(&code))
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

/// Binds media ids for a `rarray(?)` subquery -- the only form of `IN (...)` that survives a set
/// larger than SQLite's variable limit (see `bulk_remove_and_restore_exceeds_sqlite_variable_limit`).
#[cfg(test)]
fn media_id_array(ids: &[u64]) -> Result<Array> {
    let values = ids
        .iter()
        .map(|&id| Ok(Value::Integer(i64::try_from(id)?)))
        .collect::<Result<Vec<_>>>()?;
    Ok(Rc::new(values))
}

fn sqlite_connection_manager(path: &Path) -> SqliteConnectionManager {
    SqliteConnectionManager::file(path).with_init(|conn| {
        array::load_module(conn)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
    })
}

#[cfg(test)]
mod tests;
