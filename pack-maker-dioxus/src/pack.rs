use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs::{self, create_dir_all},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
    thread::available_parallelism,
};

use anyhow::{anyhow, bail, Context, Result};
use dioxus::{core::consume_context, stores::Store};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{named_params, params, params_from_iter, OptionalExtension};
use serde::{Deserialize, Serialize};
use shared::{
    db::{migrate, Database},
    encode::{FileInfo, FileInfoParts, FileType},
    read_pack::HEADER_SIZE,
};
use shared::{pack_config::Metadata, read_config::MediaCategory, read_pack::Header};
use tokio::{
    fs::{remove_file, File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::RwLock,
    task::spawn_blocking,
};
use uuid::Uuid;

use crate::{
    encode::EncodedFile, image_list::Media, media_server::Range, thumbnail::generate_preview,
    utils::file_name, SaveProgress,
};

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
            eprintln!("{err}");
        }

        if let Err(err) = fs::remove_file(&self.path) {
            eprintln!("{err}");
        }
    }
}

pub struct MediaPack {
    path: PathBuf,
    saving: Arc<RwLock<()>>,
    lock: Lock,
    header: RefCell<Header>,
    dir: PathBuf,
    metadata: RefCell<Metadata>,
    db_pool: Pool<SqliteConnectionManager>,
    tag_to_id: HashMap<String, u64>,
    id_to_tag: HashMap<u64, String>,
    db_path: PathBuf,
    saved: Cell<bool>,
}

pub struct MediaPackView {
    path: PathBuf,
    saving: Arc<RwLock<()>>,
    dir: PathBuf,
    db_pool: Pool<SqliteConnectionManager>,
    thread_pool: Arc<rayon::ThreadPool>,
}

#[derive(Debug)]
pub struct MediaData {
    pub id: i64,
    pub offset: u64,
    pub length: u64,
}

pub struct PackedEntry {
    pub path: PathBuf,
    pub info: EntryInfo,
    pub tags: Vec<String>,
    pub hash: blake3::Hash,
    pub thumbnail: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct EntryInfo {
    pub file_name: String,
    pub file_info: FileInfo,
}

#[derive(Clone, Serialize, Deserialize, Store)]
pub struct MediaInfo {
    pub id: u64,
    pub file_info: FileInfo,
    pub file_name: String,
    pub category: MediaCategory,
}

pub enum FileData {
    Path(PathBuf),
    Data(Vec<u8>),
}

impl MediaPack {
    pub async fn new(path: PathBuf, name: &str) -> Result<Self> {
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

        let data_dir = dirs::data_dir().context("Couldn't find data dir")?;

        let dir = data_dir
            .join("Lewdware Pack Editor")
            .join(header.id.to_string());

        create_dir_all(&dir)?;
        create_dir_all(&dir.join("media"))?;

        let metadata_path = dir.join("Metadata");

        let data = metadata.to_buf()?;

        File::create(&metadata_path).await?.write_all(&data).await?;

        let db_path = dir.join("index.db");

        let manager = SqliteConnectionManager::file(&db_path);
        let db_pool = Pool::builder().build(manager)?;

        let pool = db_pool.clone();

        spawn_blocking(move || -> Result<_> {
            let conn = pool.get()?;

            let db = DatabasePool(&conn);
            migrate(db)
        })
        .await??;

        let tag_to_id = HashMap::new();
        let id_to_tag = HashMap::new();

        File::create(dir.join("UNSAVED")).await?;

        Ok(Self {
            path,
            saving: Arc::new(RwLock::new(())),
            lock,
            header: RefCell::new(header),
            dir,
            metadata: RefCell::new(metadata),
            db_pool,
            tag_to_id,
            id_to_tag,
            saved: Cell::new(false),
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
            thread_pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()?,
            ),
        })
    }

    pub async fn open(path: PathBuf) -> Result<Self> {
        println!("{}", path.display());
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

        let data_dir = dirs::data_dir().context("Couldn't find data dir")?;

        let dir = data_dir
            .join("Lewdware Pack Editor")
            .join(header.id.to_string());

        println!("{}", dir.display());

        create_dir_all(&dir)?;
        create_dir_all(&dir.join("media"))?;

        let db_path = dir.join("index.db");

        let has_unsaved = fs::exists(dir.join("UNSAVED"))? && fs::exists(&db_path)?;

        println!("Unsaved: {has_unsaved}");

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

        println!("Read metadata");

        if !has_unsaved {
            println!("Extracting db data");
            // Extract the SQLite database to a temporary location
            file.seek(SeekFrom::Start(header.index_offset)).await?;
            let mut db_data = vec![0u8; header.index_length as usize];
            file.read_exact(&mut db_data).await?;

            println!("Read db data");
            println!("{}", db_data.len());

            let mut db_file = File::create(&db_path).await?;

            db_file.write_all(&db_data).await?;

            db_file.flush().await?;
        }

        println!("Wrote db data");

        let manager = SqliteConnectionManager::file(&db_path);
        let db_pool = Pool::builder().build(manager)?;

        println!("Built connection pool");

        let pool = db_pool.clone();

        let (tag_to_id, id_to_tag) = spawn_blocking(move || -> Result<_> {
            println!("Thread spawned");
            let mut tag_to_id = HashMap::new();
            let mut id_to_tag = HashMap::new();

            let conn = pool.get()?;

            let db = DatabasePool(&conn);
            migrate(db)?;

            let mut stmt = conn.prepare("SELECT id, name FROM tags")?;

            stmt.query_map(params![], |row| {
                tag_to_id.insert(row.get("name")?, row.get("id")?);
                id_to_tag.insert(row.get("id")?, row.get("name")?);

                Ok(())
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

            Ok((tag_to_id, id_to_tag))
        })
        .await??;

        println!("Built tag map");

        Ok(Self {
            path,
            saving: Arc::new(RwLock::new(())),
            lock,
            header: RefCell::new(header),
            dir,
            metadata: RefCell::new(metadata),
            db_pool,
            saved: Cell::new(!has_unsaved),
            tag_to_id,
            id_to_tag,
            db_path,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn id(&self) -> Uuid {
        self.header.borrow().id.clone()
    }

    pub async fn is_saved(&self) -> bool {
        let _handle = self.saving.write().await;

        self.saved.get()
    }

    pub async fn mark_unsaved(&self) -> Result<()> {
        if self.saved.get() {
            File::create(self.dir.join("UNSAVED")).await?;
            self.saved.set(false);
        }

        Ok(())
    }

    async fn mark_saved(&self) -> Result<()> {
        if !self.saved.get() {
            if let Err(err) = remove_file(self.dir.join("UNSAVED")).await {
                if err.kind() != io::ErrorKind::NotFound {
                    return Err(err.into());
                }
            }
            self.saved.set(true);
        }
        Ok(())
    }

    async fn db_execute<T, F>(&self, mut f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnMut(PooledConnection<SqliteConnectionManager>) -> Result<T> + Send + 'static,
    {
        let pool = self.db_pool.clone();

        let res = spawn_blocking(move || -> Result<_> {
            let conn = pool.get()?;

            f(conn)
        })
        .await??;

        Ok(res)
    }

    async fn open_read(&self) -> io::Result<File> {
        OpenOptions::new().read(true).open(&self.path).await
    }

    async fn open_write(&self) -> io::Result<File> {
        OpenOptions::new().write(true).open(&self.path).await
    }

    pub async fn save(&self) -> Result<()> {
        if self.saved.get() {
            return Ok(());
        }

        let _handle = self.saving.write().await;

        let offset = self.write_files(None).await?;

        // Compress database
        self.db_execute(|conn| conn.execute("VACUUM", []).map_err(|err| err.into()))
            .await?;

        let mut file = self.open_write().await?;

        file.seek(SeekFrom::Start(offset)).await?;

        let index_length = {
            let mut dbf = File::open(&self.db_path).await?;
            tokio::io::copy(&mut dbf, &mut file).await?
        };

        let buf = self.metadata.borrow().to_buf()?;
        let metadata_length = buf.len() as u64;

        file.write_all(&buf).await?;

        file.set_len(offset + metadata_length + index_length)
            .await?;

        let header = Header {
            id: self.header.borrow().id,
            index_offset: offset,
            index_length,
            metadata_offset: offset + index_length,
            metadata_length,
        };

        println!("{:?}", self.header);

        file.seek(SeekFrom::Start(0)).await?;
        file.write_all(&header.to_buf()?).await?;
        *self.header.borrow_mut() = header;
        file.sync_data().await?;

        self.clean_media()?;
        self.mark_saved().await?;

        Ok(())
    }

    async fn write_files(&self, to_path: Option<PathBuf>) -> Result<u64> {
        println!("Writing files");
        let dir = self.dir.clone();
        let path = self.path.clone();
        let mut progress = consume_context::<SaveProgress>();

        let result = self.db_execute(move |conn| {
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

            // If we are writing to the same file, skip all entries which are already in the
            // correct place.
            if to_path.is_none() {
                while media
                    .next_if(|x| {
                        x.as_ref().is_ok_and(|media_data| {
                            if media_data.offset == offset {
                                offset += media_data.length;
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

            progress.start_saving(num_files);

            out_file.seek(SeekFrom::Start(offset))?;

            for media_result in media {
                let media_data = media_result?;

                in_file.seek(SeekFrom::Start(media_data.offset))?;
                let mut file = in_file.take(media_data.length);

                io::copy(&mut file, &mut out_file)?;

                in_file = file.into_inner();

                edit_offset_stmt.execute(params![offset, media_data.id])?;

                offset += media_data.length;

                progress.increment_saved();

                println!("File saved");
            }

            let mut get_stmt = conn.prepare("SELECT id, path FROM media WHERE path IS NOT NULL")?;
            let mut set_offset_len =
                conn.prepare("UPDATE media SET offset = ?, length = ?, path = NULL WHERE id = ?")?;

            let media = get_stmt.query_map::<(i64, String), _, _>(params![], |row| {
                Ok((row.get("id")?, row.get("path")?))
            })?;

            for media_result in media {
                println!("Saving media");
                let (id, path) = media_result?;

                let full_path = dir.join("media").join(path);

                let size = {
                    let mut media_file = fs::File::open(&full_path)?;

                    io::copy(&mut media_file, &mut out_file)?
                };

                set_offset_len.execute(params![offset, size, id])?;

                offset += size;

                if let Err(err) = fs::remove_file(&full_path) {
                    eprintln!("{err}");
                }

                progress.increment_saved();

                println!("File saved");
            }

            Ok(offset)
        })
        .await;

        progress.reset();

        result
    }

    pub async fn save_as(&self, path: &Path) -> Result<Option<Self>> {
        if path == &self.path {
            self.save().await?;
            return Ok(None);
        }

        let _handle = self.saving.write().await;

        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(path)
            .await?;

        if self.saved.get() {
            tokio::fs::copy(&self.path, path).await?;

            let header = self.header.borrow().make_clone();
            file.write_all(&header.to_buf()?).await?;
            file.sync_data().await?;
        } else {
            println!("Writing files");
            let offset = self.write_files(Some(path.to_path_buf())).await?;

            println!("Compressing db");
            // Compress database
            self.db_execute(|conn| conn.execute("VACUUM", []).map_err(|err| err.into()))
                .await?;

            println!("Seeking");
            file.seek(SeekFrom::Start(offset)).await?;

            println!("Copying db");
            let index_length = {
                let mut dbf = File::open(&self.db_path).await?;
                println!("Performing copy");
                tokio::io::copy(&mut dbf, &mut file).await?
            };

            println!("Writing metadata");
            let buf = self.metadata.borrow().to_buf()?;
            let metadata_length = buf.len() as u64;

            file.write_all(&buf).await?;

            println!("Truncating");
            file.set_len(offset + metadata_length + index_length)
                .await?;

            let header = Header {
                id: Uuid::new_v4(),
                index_offset: offset,
                index_length,
                metadata_offset: offset + index_length,
                metadata_length,
            };

            println!("{:?}", self.header);

            println!("Writing header");
            file.seek(SeekFrom::Start(0)).await?;
            file.write_all(&header.to_buf()?).await?;
            file.sync_data().await?;

            println!("Marking as saved");
            self.mark_saved().await?;
        }

        Ok(Some(Self::open(path.to_path_buf()).await?))
    }

    pub async fn discard_changes(&self) -> Result<()> {
        if self.saved.get() {
            return Ok(());
        }

        let _handle = self.saving.write().await;

        let mut file = self.open_read().await?;

        let not_saved_yet = self.header.borrow().is_default();

        let metadata = if not_saved_yet {
            None
        } else {
            file.seek(SeekFrom::Start(self.header.borrow().metadata_offset))
                .await?;

            let mut buf = vec![0u8; self.header.borrow().metadata_length as usize];
            file.read_exact(&mut buf).await?;

            Some(Metadata::from_buf(&buf)?)
        };

        file.seek(SeekFrom::Start(self.header.borrow().index_offset))
            .await?;
        let mut db_data = vec![0u8; self.header.borrow().index_length as usize];
        file.read_exact(&mut db_data).await?;

        println!("Read db data");
        println!("{}", db_data.len());

        let mut db_file = File::create(&self.db_path).await?;

        db_file.write_all(&db_data).await?;

        db_file.flush().await?;

        self.clean_media()?;
        if let Some(metadata) = metadata {
            *self.metadata.borrow_mut() = metadata;
        }
        self.mark_saved().await?;

        Ok(())
    }

    pub fn metadata(&self) -> Metadata {
        self.metadata.borrow().clone()
    }

    pub async fn set_metadata(&self, metadata: &Metadata) -> Result<()> {
        let _handle = self.saving.read().await;

        *self.metadata.borrow_mut() = metadata.clone();

        self.mark_unsaved().await
    }

    fn clean_media(&self) -> Result<()> {
        for entry in fs::read_dir(self.dir.join("media"))? {
            if let Err(err) = entry.and_then(|entry| fs::remove_file(entry.path())) {
                eprintln!("{err}");
            }
        }

        Ok(())
    }

    pub async fn save_metadata(&self) -> Result<()> {
        let temp_path = self.dir.join("Metadata.copy");
        let final_path = self.dir.join("Metadata");

        let data = self.metadata.borrow().to_buf()?;

        tokio::fs::File::create(&temp_path)
            .await?
            .write_all(&data)
            .await?;

        fs::rename(temp_path, final_path)?;

        Ok(())
    }

    async fn edit_tags_of(&self, name: &str, id: u64, tags: &[String]) -> Result<()> {
        let _handle = self.saving.read().await;

        let tag_ids: Vec<_> = tags
            .iter()
            .filter_map(|x| self.tag_to_id.get(x))
            .cloned()
            .collect();

        let table_name = format!("{}_tags", name);
        let id_name = format!("{}_id", name);

        self.db_execute(move |mut conn| {
            let tx = conn.transaction()?;

            tx.execute(
                &format!("DELETE FROM {table_name} WHERE {id_name} = ?"),
                params![id],
            )?;

            for tag_id in &tag_ids {
                tx.execute(
                    &format!("INSERT INTO {table_name} ({id_name}, tag_id) VALUES (?1, ?2)"),
                    params![id, tag_id],
                )?;
            }

            tx.commit()?;

            Ok(())
        })
        .await?;

        self.mark_unsaved().await?;

        Ok(())
    }

    pub async fn add_file(
        &self,
        encoded_file: EncodedFile,
        path: &Path,
        hash: blake3::Hash,
    ) -> Result<Media> {
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
        let path = encoded_file.path.to_string_lossy().to_string();

        let file_name_clone = file_name.clone();
        let hash_clone = hash.clone();
        let id = self.db_execute(move |conn| {
            conn.query_row(
                "INSERT INTO media (file_name, file_type, path, width, height, transparent, duration, audio, hash, thumbnail)
                VALUES (:file_name, :file_type, :path, :width, :height, :transparent, :duration, :audio, :hash, :thumbnail) RETURNING id",
                named_params! {
                    ":file_name": file_name_clone,
                    ":file_type": file_type.as_str(),
                    ":path": path,
                    ":width": width,
                    ":height": height,
                    ":transparent": transparent,
                    ":duration": duration,
                    ":audio": audio,
                    ":hash": hash.as_bytes(),
                    ":thumbnail": encoded_file.thumbnail,
                },
                |row| row.get("id")
            ).map_err(|err| err.into())
        }).await?;

        let media = Media {
            id,
            file_name,
            file_info: encoded_file.info,
            selected: false,
            hash: hash.to_string(),
        };

        self.mark_unsaved().await?;

        Ok(media)
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

        self.mark_unsaved().await?;

        Ok(())
    }

    pub async fn get_file_info(&self, id: u64) -> Result<EntryInfo> {
        let _handle = self.saving.read().await;

        self.db_execute(move |conn| {
            conn.query_row_and_then(
                "SELECT file_name, file_type, width, height, transparent, duration, audio FROM media WHERE id = ?",
                params![id],
                |row| -> Result<_> {
                    Ok(EntryInfo {
                        file_name: row.get("file_name")?,
                        file_info: FileInfo::try_from_parts(&FileInfoParts {
                            file_type: row.get::<_, String>("file_type")?.parse()?,
                            width: row.get("width")?,
                            height: row.get("height")?,
                            transparent: row.get("transparent")?,
                            duration: row.get("duration")?,
                            audio: row.get("audio")?,
                        })?
                    })
                },
            )
        })
        .await
    }

    pub async fn get_file(&self, id: u64) -> Result<(FileData, FileType)> {
        let _handle = self.saving.read().await;

        let (offset, length, path, file_type): (
            Option<u64>,
            Option<usize>,
            Option<String>,
            FileType,
        ) = self
            .db_execute(move |conn| {
                conn.query_row_and_then(
                    "SELECT offset, length, path, file_type FROM media WHERE id = ?",
                    params![id],
                    |row| -> Result<_> {
                        Ok((
                            row.get("offset")?,
                            row.get("length")?,
                            row.get("path")?,
                            row.get::<_, String>("file_type")?.parse()?,
                        ))
                    },
                )
                .map_err(|err| err.into())
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
            (_, _, Some(path)) => {
                let path = self.dir.join("media").join(path);

                FileData::Path(path)
            }
            _ => bail!("No offset, length or path"),
        };

        Ok((file_data, file_type))
    }

    pub async fn get_files(&self) -> Result<Vec<Media>> {
        let _handle = self.saving.read().await;

        self.db_execute(move |conn| {
            let mut stmt = conn
                .prepare("SELECT id, file_type, file_name, file_type, width, height, transparent, duration, audio, hash FROM media")?;

            let result = stmt
                .query_and_then([], |row| -> Result<_> {
                    Ok(Media {
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
                        selected: false,
                        hash: blake3::Hash::from_bytes(row.get("hash")?).to_string()
                    })
                })?
                .collect();

            result
        })
        .await
    }

    pub async fn get_tags(&self, id: u64) -> Result<Vec<String>> {
        let _handle = self.saving.read().await;

        let ids: Vec<u64> = self
            .db_execute(move |conn| {
                let mut stmt = conn.prepare("SELECT tag_id FROM media_tags WHERE media_id = ?")?;

                let result = stmt
                    .query_map(params![id], |row| row.get("tag_id"))?
                    .collect::<rusqlite::Result<Vec<u64>>>()
                    .map_err(|err| err.into());

                result
            })
            .await?;

        Ok(ids
            .iter()
            .filter_map(|x| self.id_to_tag.get(x).cloned())
            .collect())
    }

    async fn edit_file_tags(&self, id: u64, tags: &Vec<String>) -> Result<()> {
        self.edit_tags_of("media", id, tags).await
    }

    pub async fn check_hash(&self, hash: &blake3::Hash) -> anyhow::Result<bool> {
        let hash = hash.clone();
        self.db_execute(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT 1 from media WHERE hash = ?",
                    params![hash.as_bytes()],
                    |_| Ok(1),
                )
                .optional()?
                .is_some())
        })
        .await
    }
}

impl Drop for MediaPack {
    fn drop(&mut self) {
        if self.saved.get() {
            if let Err(err) = fs::remove_dir_all(&self.dir) {
                eprintln!("{err}");
            }
        } else {
            eprintln!("Dropping an unsaved pack");
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

        let res = spawn_blocking(move || -> Result<_> {
            let conn = pool.get()?;

            f(conn)
        })
        .await??;

        Ok(res)
    }

    async fn open_read(&self) -> io::Result<File> {
        OpenOptions::new().read(true).open(&self.path).await
    }

    async fn get_file(&self, id: u64) -> Result<(FileData, FileType)> {
        let (offset, length, path, file_type): (
            Option<u64>,
            Option<usize>,
            Option<String>,
            FileType,
        ) = self
            .db_execute(move |conn| {
                conn.query_row_and_then(
                    "SELECT offset, length, path, file_type FROM media WHERE id = ?",
                    params![id],
                    |row| -> Result<_> {
                        Ok((
                            row.get("offset")?,
                            row.get("length")?,
                            row.get("path")?,
                            row.get::<_, String>("file_type")?.parse()?,
                        ))
                    },
                )
                .map_err(|err| err.into())
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
            (_, _, Some(path)) => {
                let path = self.dir.join("media").join(path);

                FileData::Path(path)
            }
            _ => bail!("No offset, length or path"),
        };

        Ok((file_data, file_type))
    }

    pub async fn get_thumbnail(&self, id: u64) -> Result<Vec<u8>> {
        self.db_execute(move |conn| {
            conn.query_row("SELECT thumbnail FROM media WHERE id = ?", [id], |row| {
                row.get("thumbnail")
            })
            .map_err(|err| err.into())
        })
        .await
    }

    pub async fn get_preview(&self, id: u64) -> Result<Vec<u8>> {
        let _handle = self.saving.read().await;

        let (file_data, file_type) = self.get_file(id).await?;

        generate_preview(file_data, file_type == FileType::Image).await
    }

    pub async fn get_file_data(&self, id: u64) -> Result<(Vec<u8>, FileType)> {
        let _handle = self.saving.read().await;

        let (file_data, file_type) = self.get_file(id).await?;

        let data = match file_data {
            FileData::Path(path) => tokio::fs::read(path).await?,
            FileData::Data(data) => data,
        };

        Ok((data, file_type))
    }

    pub async fn get_file_range(&self, id: u64, range: Range) -> Result<(DataRange, FileType)> {
        let _handle = self.saving.read().await;

        let (offset, length, path, file_type): (
            Option<u64>,
            Option<u64>,
            Option<String>,
            FileType,
        ) = self
            .db_execute(move |conn| {
                conn.query_row_and_then(
                    "SELECT offset, length, path, file_type FROM media WHERE id = ?",
                    params![id],
                    |row| {
                        Ok((
                            row.get("offset")?,
                            row.get("length")?,
                            row.get("path")?,
                            row.get::<_, String>("file_type")?.parse()?,
                        ))
                    },
                )
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

                let mut file = tokio::fs::File::open(path).await?;
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

/// A simple utility to repeat variables n times in a SQLite query (i.e. returns "?,?,?,?..." n
/// times).
fn repeat_vars(count: usize) -> String {
    assert_ne!(count, 0);
    let mut s = "?,".repeat(count);
    // Remove trailing comma
    s.pop();
    s
}

pub struct DataRange {
    pub data: Vec<u8>,
    pub start: u64,
    pub end: u64,
    pub total_size: u64,
}

struct DatabasePool<'a>(&'a PooledConnection<SqliteConnectionManager>);

impl<'a> Database for DatabasePool<'a> {
    fn exec(&self, sql: &str) -> Result<()> {
        self.0.execute_batch(sql)?;
        Ok(())
    }

    fn get_value(&self, sql: &str, field: &str) -> Result<Option<usize>> {
        self.0
            .query_row(sql, params![], |row| row.get(field))
            .optional()
            .map_err(|err| err.into())
    }
}

fn resolve_range(range: Range, size: u64) -> Result<(u64, u64)> {
    match (range.start, range.end) {
        (Some(start), Some(end)) => Ok((start, (end + 1).min(size))),
        (Some(start), None) => Ok((start, size)),
        _ => bail!("Invalid range"),
    }
}
