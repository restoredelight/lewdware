//! A read-only handle onto a saved pack, used by the media server to stream byte ranges
//! straight out of the archive while the editor keeps writing to its working copy.

use super::*;

use std::{
    fs::{self},
    io::{self, Read, Seek, SeekFrom, Write},
};

use anyhow::{bail, Result};
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use shared::encode::FileType;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt},
    task::spawn_blocking,
};

impl MediaPackView {
    pub(super) async fn db_execute<T, F>(&self, mut f: F) -> Result<T>
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

    pub(super) async fn open_read(&self) -> io::Result<File> {
        let path = self.path.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "pack has not been saved yet")
        })?;
        OpenOptions::new().read(true).open(path).await
    }

    pub(super) async fn get_raw_file(&self, id: u64) -> Result<(FileData, FileType, bool)> {
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

    pub async fn get_file_data(&self, id: u64) -> Result<(Vec<u8>, FileType)> {
        let _handle = self.archive_io.read().await;
        let (file_data, file_type, _) = self.get_raw_file(id).await?;
        let data = match file_data {
            FileData::Path(path) => tokio::fs::read(path).await?,
            FileData::Data(data) => data,
        };
        Ok((data, file_type))
    }

    /// Opens the requested byte range of a media file, ready to be streamed out.
    ///
    /// The archive lock is held only long enough to open and position the handle, not for the
    /// life of the response: a video preview holds its response open for as long as it is being
    /// watched, and making a save (which takes the same lock for writing, behind the editor-wide
    /// mutation lock) wait on that would freeze the editor for the length of a video. See
    /// [`FileRangeStream::into_stream`] for why an open handle is a safe thing to keep.
    pub async fn open_file_range(
        &self,
        id: u64,
        range: Range,
    ) -> Result<(FileRangeStream, FileType)> {
        let _handle = self.archive_io.read().await;

        let (offset, length, path, file_type, hash) = self
            .db_execute(move |conn| {
                conn.query_row_and_then(
                    "SELECT p.\"offset\", COALESCE(p.length, l.length) AS length,
                            l.path, m.file_type, m.hash
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
                            row.get::<_, Vec<u8>>("hash")?,
                        ))
                    },
                )
            })
            .await?;
        // The content hash doubles as the entity tag: a media file's bytes never change under a
        // given id (an edit imports a new row), so it is as strong a validator as they come.
        let entity_tag = hash.iter().map(|byte| format!("{byte:02x}")).collect();

        let data_range = match (offset, length, path) {
            (Some(offset), Some(length), _) => {
                let mut file = self.open_read().await?;
                let (start, end) = resolve_range(range, length)?;
                file.seek(SeekFrom::Start(offset + start)).await?;
                FileRangeStream {
                    file,
                    start,
                    end,
                    total_size: length,
                    entity_tag,
                }
            }
            (_, _, Some(path)) => {
                let path = self.dir.join("media").join(path);
                let mut file = tokio::fs::File::open(&path).await?;
                let size = file.metadata().await?.len();
                let (start, end) = resolve_range(range, size)?;
                file.seek(SeekFrom::Start(start)).await?;
                FileRangeStream {
                    file,
                    start,
                    end,
                    total_size: size,
                    entity_tag,
                }
            }
            _ => bail!("No offset, length or path"),
        };

        Ok((data_range, file_type))
    }
}

pub(super) fn copy_range_forward(
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

/// The half-open byte range a request resolves to, which is exactly what it asked for -- clamped
/// to the file, never to a chunk size. A response carrying less than the range it was given is
/// indistinguishable, to a media client, from a file that simply ends there (see
/// [`FileRangeStream`]).
pub(super) fn resolve_range(range: Range, size: u64) -> Result<(u64, u64)> {
    if size == 0 {
        return Err(InvalidRange.into());
    }

    let (start, end) = match (range.start, range.end) {
        (Some(start), Some(end)) if start <= end && start < size => {
            (start, end.saturating_add(1).min(size))
        }
        (Some(start), None) if start < size => (start, size),
        // RFC 9110 suffix-byte-range-spec: `bytes=-N` requests the final N bytes.
        (None, Some(suffix)) if suffix > 0 => (size.saturating_sub(suffix), size),
        _ => return Err(InvalidRange.into()),
    };
    if start >= end {
        return Err(InvalidRange.into());
    }
    Ok((start, end))
}
