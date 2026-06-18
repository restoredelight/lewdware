use std::{
    fs::{self, create_dir_all},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock as StdRwLock,
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
    read_pack::{HEADER_SIZE, Header, Metadata},
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
        let db_pool = Pool::builder().build(manager)?;

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

        let offset = self.write_files(None, on_progress).await?;

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

        self.db_execute(move |conn| {
            let mut in_file = fs::File::open(&path)?;
            let mut out_file = fs::OpenOptions::new()
                .write(true)
                .open(to_path.as_ref().unwrap_or(&path))?;

            let mut num_files: usize =
                conn.query_row_and_then("SELECT COUNT(*) as files FROM media", params![], |row| {
                    row.get("files")
                })?;

            let mut offset = HEADER_SIZE as u64;

            let mut get_stmt = conn.prepare(
                "SELECT id, offset, length FROM media WHERE offset IS NOT NULL ORDER BY offset",
            )?;
            let mut edit_offset_stmt = conn.prepare("UPDATE media SET offset = ? WHERE id = ?")?;

            let mut media = get_stmt
                .query_map(params![], |row| {
                    Ok(MediaData {
                        id: row.get("id")?,
                        offset: row.get("offset")?,
                        length: row.get("length")?,
                    })
                })?
                .peekable();

            let mut saved = 0usize;

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

            out_file.seek(SeekFrom::Start(offset))?;

            for media_result in media {
                let media_data = media_result?;
                in_file.seek(SeekFrom::Start(media_data.offset))?;
                let mut file = in_file.take(media_data.length);
                io::copy(&mut file, &mut out_file)?;
                in_file = file.into_inner();
                edit_offset_stmt.execute(params![offset, media_data.id])?;
                offset += media_data.length;
                saved += 1;
                on_progress(saved, num_files);
            }

            let mut get_stmt = conn.prepare("SELECT id, path FROM media WHERE path IS NOT NULL")?;
            let mut set_offset_len =
                conn.prepare("UPDATE media SET offset = ?, length = ?, path = NULL WHERE id = ?")?;

            let media = get_stmt.query_map::<(i64, String), _, _>(params![], |row| {
                Ok((row.get("id")?, row.get("path")?))
            })?;

            for media_result in media {
                let (id, media_path) = media_result?;
                let full_path = dir.join("media").join(media_path);
                let size = {
                    let mut media_file = fs::File::open(&full_path)?;
                    io::copy(&mut media_file, &mut out_file)?
                };
                set_offset_len.execute(params![offset, size, id])?;
                offset += size;
                if let Err(err) = fs::remove_file(&full_path) {
                    tracing::error!("{err}");
                }
                saved += 1;
                on_progress(saved, num_files);
            }

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

    pub async fn add_file(
        &self,
        encoded_file: EncodedFile,
        path: &Path,
        hash: blake3::Hash,
    ) -> Result<MediaFile> {
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

        let id = self
            .db_execute(move |conn| {
                conn.query_row(
                    "INSERT INTO media (file_name, file_type, path, width, height, transparent, duration, audio, hash, thumbnail)
                    VALUES (:file_name, :file_type, :path, :width, :height, :transparent, :duration, :audio, :hash, :thumbnail) RETURNING id",
                    named_params! {
                        ":file_name": file_name_clone,
                        ":file_type": file_type.as_str(),
                        ":path": file_path,
                        ":width": width,
                        ":height": height,
                        ":transparent": transparent,
                        ":duration": duration,
                        ":audio": audio,
                        ":hash": hash_bytes,
                        ":thumbnail": encoded_file.thumbnail,
                    },
                    |row| row.get("id"),
                )
                .map_err(|err| err.into())
            })
            .await?;

        self.mark_unsaved().await?;

        Ok(MediaFile {
            id,
            file_name,
            file_info,
            hash: hash.to_string(),
            tags: vec![],
        })
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
                "SELECT id, file_type, file_name, width, height, transparent, duration, audio, hash FROM media",
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
}
