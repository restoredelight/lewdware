use std::{
    collections::HashMap,
    fs::{self, create_dir_all},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{bail, Result};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use shared::{
    db::{migrate, Database},
    encode::{FileInfo, FileInfoParts, FileType},
    read_pack::HEADER_SIZE,
};
use shared::{pack_config::Metadata, read_config::MediaCategory, read_pack::Header};
use tauri::async_runtime::spawn_blocking;
use tokio::{
    fs::{remove_file, File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};
use uuid::Uuid;

use crate::{media_protocol::Range, CreatePackDetails};

pub struct MediaPack {
    path: PathBuf,
    lock_file: fs::File,
    lock_path: PathBuf,
    header: Header,
    dir: PathBuf,
    metadata: Metadata,
    db_pool: Pool<SqliteConnectionManager>,
    tag_to_id: HashMap<String, u64>,
    id_to_tag: HashMap<u64, String>,
    db_path: PathBuf,
    saved: bool,
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
}

#[derive(Clone)]
pub struct EntryInfo {
    pub file_name: String,
    pub category: MediaCategory,
    pub file_info: FileInfo,
}

#[derive(Clone, Serialize, Deserialize)]
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
    pub async fn new(path: PathBuf, details: CreatePackDetails, data_dir: PathBuf) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .await?;

        let lock_path = path.with_added_extension("lock");
        let lock_file = fs::File::create(&lock_path)?;

        lock_file.try_lock()?;

        let header = Header::new();

        file.write_all(&header.to_buf()?).await?;

        let metadata = Metadata {
            name: details.name,
            ..Metadata::default()
        };

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
            lock_path,
            lock_file,
            header,
            dir,
            metadata,
            db_pool,
            tag_to_id,
            id_to_tag,
            saved: false,
            db_path,
        })
    }

    pub async fn open(path: PathBuf, data_dir: PathBuf) -> Result<Self> {
        println!("{}", path.display());
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .await?;

        let lock_path = path.with_added_extension("lock");
        let lock_file = fs::File::create(&lock_path)?;

        lock_file.try_lock()?;

        println!("Getting metadata");
        // let (header, metadata) = read_pack_metadata(&mut file)?;

        let mut buf = [0u8; HEADER_SIZE];
        file.read_exact(&mut buf).await?;

        let header = Header::from_buf(buf)?;

        println!("Read metadata");

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
            let buf = fs::read(metadata_path)?;

            Metadata::from_buf(&buf)?
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
            lock_path,
            lock_file,
            header,
            dir,
            metadata,
            db_pool,
            saved: !has_unsaved,
            tag_to_id,
            id_to_tag,
            db_path,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn id(&self) -> &Uuid {
        &self.header.id
    }

    async fn mark_unsaved(&mut self) -> Result<()> {
        if self.saved {
            File::create(self.dir.join("UNSAVED")).await?;
            self.saved = false;
        }

        Ok(())
    }

    async fn mark_saved(&mut self) -> Result<()> {
        if !self.saved {
            if let Err(err) = remove_file(self.dir.join("UNSAVED")).await {
                if err.kind() != io::ErrorKind::NotFound {
                    return Err(err.into());
                }
            }

            self.saved = true;
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

    async fn open_write(&mut self) -> io::Result<File> {
        OpenOptions::new().write(true).open(&self.path).await
    }

    async fn open_read_write(&mut self) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .await
    }

    pub async fn write_changes(&mut self) -> Result<()> {
        if self.saved {
            return Ok(());
        }

        let offset = self.write_files().await?;

        // Compress database
        self.db_execute(|conn| {
            conn.execute("VACUUM", []).map_err(|err| err.into())
        }).await?;

        let mut file = self.open_write().await?;

        file.seek(SeekFrom::Start(offset)).await?;

        let index_length = {
            let mut dbf = File::open(&self.db_path).await?;
            tokio::io::copy(&mut dbf, &mut file).await?
        };

        let buf = self.metadata.to_buf()?;
        let metadata_length = buf.len() as u64;

        file.write_all(&buf).await?;

        file.set_len(offset + metadata_length + index_length)
            .await?;

        self.header.index_offset = offset;
        self.header.index_length = index_length;
        self.header.metadata_offset = offset + index_length;
        self.header.metadata_length = metadata_length;

        println!("{:?}", self.header);

        file.seek(SeekFrom::Start(0)).await?;
        file.write_all(&self.header.to_buf()?).await?;
        file.sync_data().await?;

        self.mark_saved().await?;
        self.clean_media()?;

        Ok(())
    }

    async fn write_files(&mut self) -> Result<u64> {
        println!("Writing files");
        let dir = self.dir.clone();
        let path = self.path.clone();

        self.db_execute(move |conn| {
            let mut file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?;

            let mut offset = HEADER_SIZE as u64;

            file.seek(SeekFrom::Start(offset))?;

            let mut get_stmt = conn.prepare(
                "SELECT id, offset, length FROM media WHERE offset IS NOT NULL ORDER BY offset",
            )?;
            let mut edit_offset_stmt =
                conn.prepare_cached("UPDATE media SET offset = ? WHERE id = ?")?;

            let media = get_stmt.query_map(params![], |row| {
                Ok(MediaData {
                    id: row.get("id")?,
                    offset: row.get("offset")?,
                    length: row.get("length")?,
                })
            })?;

            for media_result in media {
                let media_data = media_result?;

                println!("{:?}", media_data);
                println!("{}", offset);

                if media_data.offset != offset {
                    println!("Moving file");
                    let mut buf = vec![0u8; media_data.length as usize];
                    file.seek(SeekFrom::Start(media_data.offset))?;
                    file.read_exact(&mut buf)?;

                    file.seek(SeekFrom::Start(offset))?;
                    file.write_all(&buf)?;

                    edit_offset_stmt.execute(params![offset, media_data.id])?;
                }

                offset += media_data.length;

                println!("File saved");
            }

            let mut get_stmt = conn.prepare("SELECT id, path FROM media WHERE path IS NOT NULL")?;
            let mut set_offset_len =
                conn.prepare("UPDATE media SET offset = ?, length = ?, path = NULL WHERE id = ?")?;

            let media = get_stmt.query_map::<(i64, String), _, _>(params![], |row| {
                Ok((row.get("id")?, row.get("path")?))
            })?;

            // We only need to do this here, since every file from here on will be written
            file.seek(SeekFrom::Start(offset))?;

            for media_result in media {
                println!("Saving media result");
                println!("{}", offset);
                let (id, path) = media_result?;
                println!("{:?}", path);

                let full_path = dir.join("media").join(path);

                let mut media_file = fs::File::open(&full_path)?;

                let size = io::copy(&mut media_file, &mut file)?;

                set_offset_len.execute(params![offset, size, id])?;

                offset += size as u64;

                if let Err(err) = fs::remove_file(&full_path) {
                    eprintln!("{err}");
                }

                println!("File saved");
            }

            Ok(offset)
        })
        .await
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

        let data = self.metadata.to_buf()?;

        tokio::fs::File::create(&temp_path)
            .await?
            .write_all(&data)
            .await?;

        fs::rename(temp_path, final_path)?;

        Ok(())
    }

    // async fn add(
    //     &mut self,
    //     table_name: &'static str,
    //     data: &[(String, Box<dyn ToSql + Send>)],
    // ) -> Result<u64> {
    //     let (names, values): (Vec<_>, Vec<_>) = data.into_iter().unzip();
    //
    //     self.db_execute(move |conn| {
    //         conn.query_row(
    //             &format!(
    //                 "INSERT INTO {table} ({names}) VALUES ({vars}) RETURNING ID",
    //                 table = table_name,
    //                 names = names.join(", "),
    //                 vars = repeat_vars(names.len())
    //             ),
    //             params_from_iter(values.iter()),
    //             |row| row.get("id"),
    //         )
    //         .map_err(|err| err.into())
    //     })
    //     .await
    // }
    //
    // async fn delete(&mut self, table_name: &'static str, id: u64) -> Result<()> {
    //     self.db_execute(move |conn| {
    //         conn.execute(
    //             &format!("DELETE FROM {} WHERE id = ?", table_name),
    //             params![id],
    //         )?;
    //
    //         Ok(())
    //     })
    //     .await
    // }
    //
    // async fn edit(
    //     &mut self,
    //     table_name: &'static str,
    //     id: u64,
    //     data: Vec<(String, Box<(dyn ToSql + Send)>)>,
    // ) -> Result<()> {
    //     let (names, mut values): (Vec<_>, Vec<_>) = data.into_iter().unzip();
    //
    //     values.push(Box::new(id));
    //
    //     self.db_execute(move |conn| {
    //         conn.execute(
    //             &format!(
    //                 "UPDATE {table} SET {set_query} WHERE id = ?",
    //                 table = table_name,
    //                 set_query = names
    //                     .iter()
    //                     .map(|x| format!("{} = ?", x))
    //                     .collect::<Vec<_>>()
    //                     .join(", "),
    //             ),
    //             params_from_iter(values.iter()),
    //         )?;
    //
    //         Ok(())
    //     })
    //     .await
    // }

    async fn edit_tags_of(&mut self, name: &str, id: u64, tags: &[String]) -> Result<()> {
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

        self.mark_unsaved().await
    }

    pub async fn add_file(&mut self, entry: PackedEntry) -> Result<u64> {
        let FileInfoParts {
            file_type,
            width,
            height,
            transparent,
            duration,
            audio,
        } = entry.info.file_info.to_parts();

        let info = entry.info;

        let path = entry.path.to_string_lossy().to_string();

        let hash = entry.hash;

        let id = self.db_execute(move |conn| {
            conn.query_row(
                "INSERT INTO media (file_name, file_type, category, path, width, height, transparent, duration, audio, hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
                params![info.file_name, file_type.as_str(), info.category.as_str(), path, width, height, transparent, duration, audio, hash.as_bytes()],
                |row| row.get("id")
            ).map_err(|err| err.into())
        }).await?;

        self.mark_unsaved().await?;

        Ok(id)
    }

    pub async fn delete_file(&mut self, id: u64) -> Result<()> {
        self.db_execute(move |conn| {
            conn.execute("DELETE FROM media WHERE id = ?", params![id])?;

            Ok(())
        })
        .await?;

        self.mark_unsaved().await
    }

    pub async fn edit_file(&mut self, id: u64, info: EntryInfo) -> Result<()> {
        let FileInfoParts {
            width,
            height,
            duration,
            ..
        } = info.file_info.to_parts();

        self.db_execute(move |conn| {
            conn.execute(
                "UPDATE media SET category = ?, width = ?, height = ?, duration = ? WHERE id = ?",
                params![info.category.as_str(), width, height, duration, id],
            )?;

            Ok(())
        })
        .await?;

        self.mark_unsaved().await
    }

    pub async fn get_file_info(&self, id: u64) -> Result<EntryInfo> {
        self.db_execute(move |conn| {
            conn.query_row_and_then(
                "SELECT file_name, category, file_type, width, height, transparent, duration, audio FROM media WHERE id = ?",
                params![id],
                |row| -> Result<_> {
                    Ok(EntryInfo {
                        file_name: row.get("file_name")?,
                        category: MediaCategory::from_str(&row.get::<_, String>("category")?)?,
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

    pub async fn get_file_range(&self, id: u64, range: Range) -> Result<(DataRange, FileType)> {
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

    pub async fn get_files(&self) -> Result<Vec<MediaInfo>> {
        self.db_execute(move |conn| {
            let mut stmt = conn
                .prepare("SELECT id, file_type, file_name, file_type, width, height, transparent, duration, audio FROM media")?;

            let result = stmt
                .query_and_then([], |row| -> Result<_> {
                    Ok(MediaInfo {
                        id: row.get("id")?,
                        file_name: row.get("file_name")?,
                        category: MediaCategory::Default,
                        file_info: FileInfo::try_from_parts(&FileInfoParts {
                            file_type: row.get::<_, String>("file_type")?.parse()?,
                            width: row.get("width")?,
                            height: row.get("height")?,
                            transparent: row.get("transparent")?,
                            duration: row.get("duration")?,
                            audio: row.get("audio")?,
                        })?
                    })
                })?
                .collect();

            result
        })
        .await
    }

    pub async fn get_tags(&self, id: u64) -> Result<Vec<String>> {
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

    async fn edit_file_tags(&mut self, id: u64, tags: &Vec<String>) -> Result<()> {
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
        if let Err(err) = self.lock_file.unlock() {
            eprintln!("{err}");
        }

        if let Err(err) = fs::remove_file(&self.lock_path) {
            eprintln!("{err}");
        }
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
