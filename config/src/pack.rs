use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::PathBuf, sync::Arc,
};

use anyhow::{Context, Result};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use shared::{
    db::migrate,
    mode::read_mode_metadata_async,
    pack_config::Metadata,
    read_pack::{Header, read_pack_metadata},
};
use tempfile::NamedTempFile;
use tokio::{
    task::spawn_blocking,
};
use tokio_stream::StreamExt;

use crate::modes::PackMode;

#[derive(Clone)]
pub struct MediaPack {
    path: PathBuf,
    header: Header,
    metadata: Metadata,
    db_pool: Pool<SqliteConnectionManager>,
    db_file: Arc<NamedTempFile>,
}

impl MediaPack {
    pub fn open(path: PathBuf) -> Result<Self> {
        let mut file = std::fs::File::open(&path)?;

        let (header, metadata) = read_pack_metadata(&mut file)?;

        let mut db_file = NamedTempFile::new()?;

        file.seek(SeekFrom::Start(header.index_offset))?;
        let mut db_data = file.take(header.index_length);

        std::io::copy(&mut db_data, db_file.as_file_mut())?;

        let manager = SqliteConnectionManager::file(&db_file.path());
        let db_pool = Pool::builder().build(manager)?;

        let conn = db_pool.get()?;

        migrate(&conn)?;

        Ok(Self {
            path,
            header,
            metadata,
            db_pool,
            db_file: Arc::new(db_file),
        })
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
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

    pub async fn get_modes(&self) -> Result<Vec<PackMode>> {
        let files: Vec<(u64, Vec<u8>)> = self
            .db_execute(move |conn| {
                let mut stmt = conn.prepare("SELECT id, file FROM modes")?;

                let result = stmt
                    .query_map([], |row| Ok((row.get("id")?, row.get("file")?)))?
                    .collect::<rusqlite::Result<_>>()?;

                Ok(result)
            })
            .await?;

        tokio_stream::iter(files)
            .then(async |(id, data)| -> Result<PackMode> {
                let mut cursor = Cursor::new(data);

                let (_, metadata) = read_mode_metadata_async(&mut cursor).await?;

                Ok(PackMode { id, metadata })
            })
            .collect()
            .await
    }

    pub async fn get_mode(&self, id: u64, mode: &str) -> Result<shared::mode::Mode> {
        let data: Vec<u8> = self
            .db_execute(move |conn| {
                let mut stmt = conn.prepare("SELECT file FROM modes WHERE id = ?")?;

                let result = stmt
                    .query_row(params![id], |row| row.get("file"))?;

                Ok(result)
            })
            .await?;

        let mut cursor = Cursor::new(data);

        let (_, mut metadata) = read_mode_metadata_async(&mut cursor).await?;

        metadata.modes.swap_remove(mode).context("Invalid mode")
    }
}
