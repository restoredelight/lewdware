use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

use image::{ImageFormat, ImageReader};
use rusqlite::{Connection, MAIN_DB, Row, params, params_from_iter};
use shared::{
    db::migrate,
    read_pack::{Header, Metadata, read_pack_metadata},
};
use tempfile::NamedTempFile;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
    sync::oneshot,
};

use crate::{
    lua::{Media, MediaData},
    media::{
        VideoData,
        manager::{MediaError, MediaTypes, Result},
        types::{FileOrPath, ImageData, MediaSource},
    },
};

/// A simple utility to repeat variables n times in a SQLite query (i.e. returns "?,?,?,?..." n
/// times).
fn repeat_vars(count: usize) -> String {
    assert_ne!(count, 0);
    let mut s = "?,".repeat(count);
    // Remove trailing comma
    s.pop();
    s
}

/// A media pack, consisting of a header, some metadata and an SQLite database at the end, which
/// contains information about all the media in the file. The database stores the offset and length
/// of each image/video/audio file, which can be used to read it from the pack file.
pub struct MediaPack {
    path: PathBuf,
    db: Connection,
    #[allow(unused)]
    header: Header,
    metadata: Metadata,
    tag_map: HashMap<String, u64>,
}

struct MediaOpts {
    name: Option<String>,
    types: MediaTypes,
    tags: Option<Vec<String>>,
    random: bool,
    single: bool,
}

impl MediaPack {
    pub fn open(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        let mut file = fs::File::open(&path)?;

        let (header, metadata) = read_pack_metadata(&mut file)?;

        // Load the SQLite database straight into memory (no temp file: `deserialize_read_exact`
        // hands the bytes we just read directly to SQLite's own in-memory representation via
        // `sqlite3_deserialize`).
        file.seek(SeekFrom::Start(header.index_offset))?;
        let mut db_data = vec![0u8; header.index_length as usize];
        file.read_exact(&mut db_data)?;

        let mut connection = Connection::open_in_memory()?;
        connection.deserialize_read_exact(MAIN_DB, db_data.as_slice(), db_data.len(), false)?;

        migrate(&connection)?;

        let mut tag_map: HashMap<String, u64> = HashMap::new();

        {
            let mut stmt = connection.prepare("SELECT id, name FROM tags")?;

            stmt.query_map(params![], |row| {
                tag_map.insert(row.get("name")?, row.get::<_, u64>("id")?);
                Ok(())
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        }

        Ok(MediaPack {
            path,
            db: connection,
            header,
            metadata,
            tag_map,
        })
    }

    fn build_sql(&self, opts: MediaOpts) -> Result<(String, Vec<Box<dyn rusqlite::ToSql + '_>>)> {
        let mut sql = "
            SELECT id, file_name, file_type, offset, length, width, height, duration, audio, transparent
            FROM media
        "
        .to_string();

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if opts.tags.is_some() {
            sql.push_str(" LEFT JOIN media_tags ON media.id = media_tags.media_id ");
        }

        let mut where_queries = Vec::new();

        if let Some(name) = &opts.name {
            where_queries.push("file_name = ?".to_string());
            params.push(Box::new(name.clone()));
        }

        if let Some(query) = self.build_media_types_query(&opts.types) {
            where_queries.push(query);
        }

        if let Some(tags) = &opts.tags {
            let tag_ids = tags
                .iter()
                .map(|tag| {
                    self.tag_map
                        .get(tag)
                        .ok_or(MediaError::InvalidTag(tag.clone()))
                })
                .collect::<Result<Vec<_>>>()?;

            where_queries.push(format!(
                "media_tags.tag_id IN ({})",
                repeat_vars(tag_ids.len())
            ));

            for id in tag_ids {
                params.push(Box::new(id));
            }
        }

        if !where_queries.is_empty() {
            sql.push_str(&format!("WHERE {} ", where_queries.join(" AND ")));
        }

        if opts.random {
            sql.push_str(" ORDER BY RANDOM() ");
        }

        if opts.single {
            sql.push_str(" LIMIT 1 ");
        }

        Ok((sql, params))
    }

    pub fn get_media(&self, name: String, types: MediaTypes) -> Result<Option<Media>> {
        let (sql, params) = self.build_sql(MediaOpts {
            name: Some(name),
            types,
            tags: None,
            random: false,
            single: true,
        })?;

        let mut stmt = self.db.prepare(&sql)?;

        stmt.query_and_then(params_from_iter(params), parse_media)?
            .next()
            .transpose()
    }

    pub fn random_media(
        &self,
        types: MediaTypes,
        tags: Option<Vec<String>>,
    ) -> Result<Option<Media>> {
        let (sql, params) = self.build_sql(MediaOpts {
            name: None,
            types,
            tags,
            random: true,
            single: true,
        })?;

        let mut stmt = self.db.prepare(&sql)?;

        stmt.query_and_then(params_from_iter(params), parse_media)?
            .next()
            .transpose()
    }

    pub fn list_media(&self, types: MediaTypes, tags: Option<Vec<String>>) -> Result<Vec<Media>> {
        let (sql, params) = self.build_sql(MediaOpts {
            name: None,
            types,
            tags,
            random: false,
            single: false,
        })?;

        let mut stmt = self.db.prepare(&sql)?;

        stmt.query_and_then(params_from_iter(params), parse_media)?
            .collect()
    }

    fn build_media_types_query(&self, types: &MediaTypes) -> Option<String> {
        match *types {
            MediaTypes::ALL => None,
            MediaTypes::NONE => Some("FALSE".to_string()),
            _ => {
                let mut queries = Vec::new();

                if types.image {
                    queries.push("file_type = 'image'".to_string());
                }

                if types.video {
                    queries.push("file_type = 'video'".to_string());
                }

                if types.audio {
                    queries.push("file_type = 'audio'".to_string());
                }

                Some(format!("({})", queries.join(" OR ")))
            }
        }
    }

    pub async fn get_image_data(&self, id: u64, width: u32, height: u32) -> Result<ImageData> {
        let (offset, length) = self.get_offset_length(id)?;

        self.read_image_data(offset, length, width, height).await
    }

    pub async fn get_image_file(&self, id: u64) -> Result<FileOrPath> {
        let (offset, length) = self.get_offset_length(id)?;

        Ok(FileOrPath::File(
            self.write_to_temp_file(offset, length, ".avif").await?,
        ))
    }

    pub fn get_video_data(&self, id: u64) -> Result<VideoData> {
        let (offset, length, width, height, transparent) = self.db.query_row(
            "SELECT offset, length, width, height, transparent FROM media WHERE id = ?",
            params![id],
            |row| {
                Ok((
                    row.get("offset")?,
                    row.get("Length")?,
                    row.get("width")?,
                    row.get("height")?,
                    row.get("transparent")?,
                ))
            },
        )?;

        Ok(VideoData {
            source: self.media_source(offset, length),
            width,
            height,
            transparent,
        })
    }

    pub fn get_audio_data(&self, id: u64) -> Result<MediaSource> {
        let (offset, length) = self.get_offset_length(id)?;

        Ok(self.media_source(offset, length))
    }

    fn media_source(&self, offset: u64, length: u64) -> MediaSource {
        MediaSource {
            path: self.path.clone(),
            offset,
            length,
        }
    }

    fn get_offset_length(&self, id: u64) -> Result<(u64, u64)> {
        let mut stmt = self
            .db
            .prepare("SELECT offset, length FROM media WHERE id = ?")?;

        stmt.query_row(params![id], |row| {
            Ok((row.get("offset")?, row.get("length")?))
        })
        .map_err(|err| err.into())
    }

    pub fn get_mode(&self, id: u64) -> anyhow::Result<Vec<u8>> {
        let mut stmt = self.db.prepare("SELECT file FROM modes WHERE id = ?")?;

        stmt.query_row(params![id], |row| row.get("file"))
            .map_err(|err| err.into())
    }

    async fn read_image_data(
        &self,
        offset: u64,
        length: u64,
        width: u32,
        height: u32,
    ) -> Result<ImageData> {
        let mut file = std::fs::File::open(&self.path)?;
        file.seek(SeekFrom::Start(offset))?;
        let file = file.take(length);

        let mut reader = ImageReader::new(std::io::BufReader::new(file));

        reader.set_format(ImageFormat::Avif);

        let image = reader.decode()?;

        if image.width() != width || image.height() != height {
            let (tx, rx) = oneshot::channel();

            rayon::spawn(move || {
                use fast_image_resize::{
                    FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images::Image,
                };

                let src: image::DynamicImage = image.into_rgba8().into();
                let mut dst = Image::new(width, height, PixelType::U8x4);
                let opts =
                    ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));
                let result = Resizer::new()
                    .resize(&src, &mut dst, &opts)
                    .map(|_| {
                        image::ImageBuffer::from_raw(width, height, dst.into_vec())
                            .expect("buffer size is always width * height * 4")
                    })
                    .map_err(|_| MediaError::Internal("Image resizing failed"));
                let _ = tx.send(result);
            });

            rx.await
                .map_err(|_| MediaError::Internal("Image resizing sender dropped"))?
        } else {
            Ok(image.into_rgba8())
        }
    }

    async fn write_to_temp_file(
        &self,
        offset: u64,
        length: u64,
        suffix: &str,
    ) -> Result<NamedTempFile> {
        let mut tempfile = NamedTempFile::with_suffix_in(suffix, crate::utils::temp_dir())?;
        let mut buffer = vec![0u8; length as usize];

        let mut file = File::open(&self.path).await?;

        file.seek(SeekFrom::Start(offset)).await?;

        file.read_exact(&mut buffer).await?;

        tempfile.write_all(&buffer)?;

        Ok(tempfile)
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

fn parse_media(row: &Row<'_>) -> Result<Media> {
    let media_data = match row.get::<_, String>("file_type")?.as_str() {
        "image" => MediaData::Image {
            width: row.get("width")?,
            height: row.get("height")?,
            transparent: row.get::<_, Option<bool>>("transparent")?.unwrap_or(false),
        },
        "video" => MediaData::Video {
            width: row.get("width")?,
            height: row.get("height")?,
            duration: row.get("duration")?,
            transparent: row.get::<_, Option<bool>>("transparent")?.unwrap_or(false),
        },
        "audio" => MediaData::Audio {
            duration: row.get("duration")?,
        },
        _ => return Err(MediaError::Internal("Invalid file type")),
    };

    Ok(Media {
        id: row.get("id")?,
        name: row.get("file_name")?,
        media_data,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use ffmpeg_next as ffmpeg;
    use shared::read_pack::HEADER_SIZE;

    use super::*;

    /// The pack-editor (`pack-editor/src-tauri/src/pack.rs`, `Pack::save`) doesn't build the
    /// index via `rusqlite`'s `serialize()` -- it runs `VACUUM` on a plain on-disk connection
    /// opened through `SqliteConnectionManager::file(...)` (default journal mode, no WAL), then
    /// copies the file's raw bytes into the pack. Confirms that byte stream -- not just the
    /// output of `Connection::serialize()` -- is exactly what `deserialize_read_exact` expects.
    #[test]
    fn deserializes_a_vacuumed_on_disk_file_copy_like_pack_editor_produces() {
        let db_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let db_path = db_path.to_path_buf();

        {
            let conn = Connection::open(&db_path).unwrap();
            migrate(&conn).unwrap();
            conn.execute("INSERT INTO tags (name) VALUES ('from-disk')", [])
                .unwrap();
            // Mirrors `Pack::save`: VACUUM then treat the file as done, no explicit close/flush
            // beyond what VACUUM itself guarantees.
            conn.execute("VACUUM", []).unwrap();
        }

        // Raw file copy, exactly like `tokio::io::copy(&mut dbf, &mut file)` in pack-editor --
        // deliberately not using `Connection::serialize()`.
        let db_bytes = std::fs::read(&db_path).unwrap();

        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .deserialize_read_exact(MAIN_DB, db_bytes.as_slice(), db_bytes.len(), false)
            .unwrap();

        let name: String = connection
            .query_row("SELECT name FROM tags", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "from-disk");
    }

    /// Builds a pack file on disk with a real (migrated) SQLite index, to check that
    /// `MediaPack::open`'s `deserialize_read_exact` round-trip actually produces a working,
    /// queryable database rather than just satisfying the type checker.
    #[test]
    fn open_reads_deserialized_index() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();

        db.execute("INSERT INTO tags (name) VALUES ('test-tag')", [])
            .unwrap();
        db.execute(
            "INSERT INTO media (file_name, file_type, width, height, transparent, hash)
             VALUES ('pic.avif', 'image', 64, 32, 1, x'00')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO media_tags (media_id, tag_id) VALUES (1, 1)",
            [],
        )
        .unwrap();

        let db_bytes = db.serialize(MAIN_DB).unwrap();

        let metadata = Metadata {
            name: "test-pack".to_string(),
            ..Default::default()
        };
        let metadata_bytes = metadata.to_buf().unwrap();

        let mut header = Header::new();
        header.metadata_offset = HEADER_SIZE as u64;
        header.metadata_length = metadata_bytes.len() as u64;
        header.index_offset = header.metadata_offset + header.metadata_length;
        header.index_length = db_bytes.len() as u64;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&header.to_buf().unwrap()).unwrap();
        file.write_all(&metadata_bytes).unwrap();
        file.write_all(&db_bytes).unwrap();
        file.flush().unwrap();

        let pack = MediaPack::open(file.path()).unwrap();

        assert_eq!(pack.metadata().name, "test-pack");

        let results = pack
            .list_media(MediaTypes::ALL, Some(vec!["test-tag".to_string()]))
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "pic.avif");
        assert!(matches!(
            results[0].media_data,
            MediaData::Image {
                width: 64,
                height: 32,
                transparent: true,
            }
        ));

        // Also confirm a tag that doesn't exist is rejected rather than silently ignored.
        assert!(matches!(
            pack.list_media(MediaTypes::ALL, Some(vec!["nonexistent".to_string()])),
            Err(MediaError::InvalidTag(_))
        ));
    }

    /// End-to-end check of the zero-copy video path: builds a pack file with a real embedded
    /// video (offset/length recorded in the index, exactly like a real pack), then confirms
    /// `get_video_data` produces a `MediaSource` that ffmpeg can actually open and decode --
    /// exercising the same offset/length plumbing `MediaManager`/`VideoDecoder` rely on, not
    /// just the isolated `open_bounded` helper.
    #[test]
    fn get_video_data_opens_embedded_clip() {
        const TEST_CLIP: &[u8] = include_bytes!("test_fixtures/test_clip.mp4");

        ffmpeg::init().unwrap();

        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        db.execute(
            "INSERT INTO media (file_name, file_type, width, height, transparent, duration, hash)
             VALUES ('clip.mp4', 'video', 64, 48, 0, 1.0, x'00')",
            [],
        )
        .unwrap();

        let db_bytes = db.serialize(MAIN_DB).unwrap();

        let metadata = Metadata {
            name: "test-pack".to_string(),
            ..Default::default()
        };
        let metadata_bytes = metadata.to_buf().unwrap();

        let mut header = Header::new();
        header.metadata_offset = HEADER_SIZE as u64;
        header.metadata_length = metadata_bytes.len() as u64;
        header.index_offset = header.metadata_offset + header.metadata_length;
        header.index_length = db_bytes.len() as u64;
        let video_offset = header.index_offset + header.index_length;

        // The row's `offset`/`length` columns are set after building the header above, so we
        // need a second pass over the DB to update them before serializing -- keep it simple and
        // just build the DB again, now that `video_offset` is known.
        db.execute(
            "UPDATE media SET offset = ?, length = ? WHERE file_name = 'clip.mp4'",
            params![video_offset, TEST_CLIP.len() as u64],
        )
        .unwrap();
        let db_bytes = db.serialize(MAIN_DB).unwrap();

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&header.to_buf().unwrap()).unwrap();
        file.write_all(&metadata_bytes).unwrap();
        file.write_all(&db_bytes).unwrap();
        file.write_all(TEST_CLIP).unwrap();
        file.flush().unwrap();

        let pack = MediaPack::open(file.path()).unwrap();
        let media = pack
            .list_media(MediaTypes::VIDEO, None)
            .unwrap()
            .pop()
            .unwrap();

        let data = pack.get_video_data(media.id).unwrap();
        assert_eq!(data.source.offset, video_offset);
        assert_eq!(data.source.length, TEST_CLIP.len() as u64);

        let ictx = data.source.open().unwrap();
        assert!(ictx.streams().best(ffmpeg::media::Type::Video).is_some());
        assert!(ictx.streams().best(ffmpeg::media::Type::Audio).is_some());
    }
}
