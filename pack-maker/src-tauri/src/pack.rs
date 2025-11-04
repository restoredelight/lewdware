use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    str::FromStr,
};

use anyhow::Result;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use shared::read_pack::HEADER_SIZE;
use shared::{
    pack_config::Metadata,
    read_config::MediaCategory,
    read_pack::{read_pack_metadata_async, Header},
    utils::FileType,
};
use tauri::async_runtime::spawn_blocking;
use tempfile::NamedTempFile;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};

use crate::CreatePackDetails;

pub struct MediaPack {
    path: PathBuf,
    metadata: Metadata,
    db_pool: Pool<SqliteConnectionManager>,
    current_offset: u64,
    tag_to_id: HashMap<String, u64>,
    id_to_tag: HashMap<u64, String>,
    db_file: NamedTempFile,
}

pub struct MediaData {
    pub id: i64,
    pub offset: u64,
    pub length: u64,
}

pub struct PackedEntry {
    pub data: Vec<u8>,
    pub media_type: FileType,
    pub info: EntryInfo,
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub id: u64,
    pub info: EntryInfo,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct EntryInfo {
    pub path: String,
    pub category: MediaCategory,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<i64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub id: u64,
    pub file_type: String,
    pub file_name: String,
    pub category: MediaCategory,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<i64>,
}

impl MediaPack {
    pub async fn new(path: PathBuf, details: CreatePackDetails) -> Result<Self> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .await?;

        Header::default().write_to_async(&mut file).await?;

        let metadata = Metadata {
            name: details.name,
            ..Metadata::default()
        };

        let temp_file = NamedTempFile::new()?;

        let manager = SqliteConnectionManager::file(temp_file.path());
        let db_pool = Pool::builder().build(manager)?;

        // TODO: Create tables

        let offset = HEADER_SIZE as u64;

        let tag_to_id = HashMap::new();
        let id_to_tag = HashMap::new();

        Ok(Self {
            path,
            metadata,
            db_pool,
            current_offset: offset,
            tag_to_id,
            id_to_tag,
            db_file: temp_file,
        })
    }

    pub async fn open(path: PathBuf) -> Result<Self> {
        println!("{}", path.display());
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .await?;

        println!("Getting metadata");
        let (header, metadata) = read_pack_metadata_async(&mut file).await?;

        println!("{}", metadata.name);
        println!("Files: {}", header.total_files);

        // Extract the SQLite database to a temporary location
        file.seek(SeekFrom::End(-(header.index_offset() as i64)))
            .await?;
        let mut db_data = Vec::new();
        file.read_to_end(&mut db_data).await?;

        println!("Read db data");
        println!("{}", db_data.len());

        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(&db_data)?;

        println!("Wrote db data");

        let manager = SqliteConnectionManager::file(temp_file.path());
        let db_pool = Pool::builder().build(manager)?;

        println!("Built connection pool");

        let offset = header.index_length;

        let pool = db_pool.clone();

        let (tag_to_id, id_to_tag) = spawn_blocking(move || -> Result<_> {
            println!("Thread spawned");
            let mut tag_to_id = HashMap::new();
            let mut id_to_tag = HashMap::new();

            let conn = pool.get()?;

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
            metadata,
            db_pool,
            current_offset: offset,
            tag_to_id,
            id_to_tag,
            db_file: temp_file,
        })
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
        OpenOptions::new()
            .read(true)
            .append(true)
            .open(&self.path)
            .await
    }

    async fn open_write(&self) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .await
    }

    async fn open_append(&self) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .append(true)
            .open(&self.path)
            .await
    }

    pub async fn write_changes(&mut self, pack_files: bool) -> Result<()> {
        let offset = if pack_files {
            self.write_files().await?
        } else {
            self.db_execute(|conn| {
                let offset = conn.query_row(
                    "SELECT MAX(offset) as offset FROM media",
                    params![],
                    |row| row.get("offset"),
                )?;

                Ok(offset)
            })
            .await?
        };

        let total_files = self
            .db_execute(move |conn| {
                conn.query_row("SELECT COUNT(*) as files FROM media", params![], |row| {
                    row.get("files")
                })
                .map_err(|err| err.into())
            })
            .await?;

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .await?;

        file.seek(SeekFrom::Start(offset)).await?;

        let buf = self.metadata.to_buf()?;
        let metadata_length = buf.len() as u64;

        file.write_all(&buf).await?;

        let index_length = {
            let mut dbf = File::open(self.db_file.path()).await?;
            tokio::io::copy(&mut dbf, &mut file).await?
        };

        file.set_len(offset + metadata_length + index_length).await?;

        let header = Header {
            index_length,
            metadata_length,
            total_files,
        };

        header.write_to_async(&mut file).await?;

        Ok(())
    }

    async fn write_files(&mut self) -> Result<u64> {
        let path = self.path.clone();

        self.db_execute(move |mut conn| {
            let mut file = fs::OpenOptions::new().read(true).write(true).open(&path)?;

            let tx = conn.transaction()?;

            let mut offset = HEADER_SIZE as u64;

            {
                let mut get_stmt =
                    tx.prepare_cached("SELECT id, offset, length FROM media ORDER BY offset")?;
                let mut edit_offset_stmt =
                    tx.prepare_cached("UPDATE media SET offset = ? WHERE id = ?")?;

                let media = get_stmt.query_map(params![], |row| {
                    Ok(MediaData {
                        id: row.get("id")?,
                        offset: row.get("offset")?,
                        length: row.get("length")?,
                    })
                })?;

                for media_result in media {
                    let media_data = media_result?;

                    if media_data.offset != offset {
                        let mut buf = vec![0u8; media_data.length as usize];
                        file.seek(SeekFrom::Start(media_data.offset))?;
                        file.read_exact(&mut buf)?;

                        file.seek(SeekFrom::Start(offset))?;
                        file.write_all(&buf)?;

                        edit_offset_stmt.execute(params![offset, media_data.id])?;
                    }

                    offset += media_data.length;
                }
            }

            tx.commit()?;

            Ok(offset)
        })
        .await
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
        .await
    }

    pub async fn add_file(&mut self, entry: PackedEntry) -> Result<u64> {
        self.file.seek(SeekFrom::Start(self.current_offset)).await?;
        self.file.write_all(&entry.data).await?;

        let current_offset = self.current_offset;
        let media_type = entry.media_type.clone();
        let info = entry.info.clone();
        let len = entry.data.len();

        let id = self.db_execute(move |conn| {
            conn.query_row(
                "INSERT INTO media (path, media_type, category, offset, length, width, height, duration) VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
                params![info.path, media_type.as_str(), info.category.as_str(), current_offset, len, info.width, info.height, info.duration],
                |row| row.get("id")
            ).map_err(|err| err.into())
        }).await?;

        self.current_offset += entry.data.len() as u64;

        Ok(id)
    }

    pub async fn delete_file(&mut self, id: u64) -> Result<()> {
        self.db_execute(move |conn| {
            conn.execute("DELETE FROM media WHERE id = ?", params![id])?;

            Ok(())
        })
        .await
    }

    pub async fn edit_file(&mut self, id: u64, info: EntryInfo) -> Result<()> {
        self.db_execute(move |conn| {
            conn.execute(
                "UPDATE media SET category = ?, width = ?, height = ?, duration = ? WHERE id = ?",
                params![
                    info.category.as_str(),
                    info.width,
                    info.height,
                    info.duration,
                    id
                ],
            )?;

            Ok(())
        })
        .await
    }

    pub async fn get_file_info(&self, id: u64) -> Result<EntryInfo> {
        self.db_execute(move |conn| {
            conn.query_row_and_then(
                "SELECT path, category, width, height, duration FROM media WHERE id = ?",
                params![id],
                |row| -> Result<_> {
                    Ok(EntryInfo {
                        path: row.get("path")?,
                        category: MediaCategory::from_str(&row.get::<_, String>("category")?)?,
                        width: row.get("width")?,
                        height: row.get("height")?,
                        duration: row.get("duration")?,
                    })
                },
            )
        })
        .await
    }

    pub async fn get_file(&mut self, id: u64) -> Result<(Vec<u8>, FileType)> {
        let (offset, length, file_type): (u64, usize, FileType) = self
            .db_execute(move |conn| {
                conn.query_row(
                    "SELECT offset, length, media_type FROM media WHERE id = ?",
                    params![id],
                    |row| {
                        Ok((
                            row.get("offset")?,
                            row.get("length")?,
                            row.get::<_, String>("media_type")?.parse().unwrap(),
                        ))
                    },
                )
                .map_err(|err| err.into())
            })
            .await?;

        self.file.seek(SeekFrom::Start(offset)).await?;

        let mut buf = vec![0u8; length];

        self.file.read_exact(&mut buf).await?;

        Ok((buf, file_type))
    }

    pub async fn get_files(&self) -> Result<Vec<MediaInfo>> {
        self.db_execute(move |conn| {
            let mut stmt =
                conn.prepare("SELECT id, media_type, path, width, height, duration FROM media")?;

            let result = stmt
                .query_and_then([], |row| -> Result<_> {
                    let path: String = row.get("path")?;
                    let file_name = PathBuf::from(path)
                        .file_name()
                        .map_or("".to_string(), |x| x.to_string_lossy().to_string());

                    Ok(MediaInfo {
                        id: row.get("id")?,
                        file_type: row.get("media_type")?,
                        file_name,
                        category: MediaCategory::Default,
                        width: row.get("width")?,
                        height: row.get("height")?,
                        duration: row.get("duration")?,
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
