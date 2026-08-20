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
    pack::{Header, Metadata, read_pack_metadata},
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
        manager::{MediaError, MediaTypes, Result, TagFilter},
        types::{ExtractedFile, ImageData, MediaSource},
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

/// A pack, consisting of a header, some metadata and an SQLite database at the end, which
/// contains information about all the media in the file. The database stores the offset and length
/// of each image/video/audio file, which can be used to read it from the pack file.
pub struct MediaPack {
    path: PathBuf,
    db: Connection,
    header: Header,
    metadata: Metadata,
    tag_map: HashMap<String, u64>,
}

struct MediaOpts {
    name: Option<String>,
    id: Option<u64>,
    types: MediaTypes,
    tags: Option<TagFilter>,
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

        migrate(&mut connection)?;

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

    fn build_sql(&self, opts: MediaOpts) -> (String, Vec<Box<dyn rusqlite::ToSql + '_>>) {
        let mut sql = "
            SELECT id, file_name, file_type, offset, length, width, height, duration, audio, transparent,
                (SELECT group_concat(tags.name, char(31))
                 FROM media_tags
                 JOIN tags ON tags.id = media_tags.tag_id
                 WHERE media_tags.media_id = media.id) AS tag_names
            FROM media
        "
        .to_string();

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        let mut where_queries = Vec::new();

        if let Some(name) = &opts.name {
            where_queries.push("file_name = ?".to_string());
            params.push(Box::new(name.clone()));
        }

        if let Some(id) = opts.id {
            where_queries.push("id = ?".to_string());
            params.push(Box::new(id));
        }

        if let Some(query) = self.build_media_types_query(&opts.types) {
            where_queries.push(query);
        }

        if let Some(filter) = &opts.tags {
            // Tag conditions are expressed as EXISTS subqueries rather than a join, so a file
            // matching several requested tags still yields a single row (no DISTINCT needed,
            // and `ORDER BY RANDOM() LIMIT 1` stays uniform across files).
            //
            // Tags the pack doesn't define never match anything; they're dropped rather than
            // treated as errors, since callers (e.g. the default mode) routinely probe for
            // optional tags like `hypno` or `wallpaper` that a given pack may simply not have.

            if !filter.any.is_empty() {
                let tag_ids: Vec<u64> = filter
                    .any
                    .iter()
                    .filter_map(|tag| self.tag_map.get(tag).copied())
                    .collect();

                if tag_ids.is_empty() {
                    // None of the requested tags exist in this pack, so nothing can match.
                    where_queries.push("FALSE".to_string());
                } else {
                    where_queries.push(format!(
                        "EXISTS (SELECT 1 FROM media_tags
                         WHERE media_id = media.id AND tag_id IN ({}))",
                        repeat_vars(tag_ids.len())
                    ));

                    for id in tag_ids {
                        params.push(Box::new(id));
                    }
                }
            }

            for tag in &filter.all {
                match self.tag_map.get(tag) {
                    Some(id) => {
                        where_queries.push(
                            "EXISTS (SELECT 1 FROM media_tags
                             WHERE media_id = media.id AND tag_id = ?)"
                                .to_string(),
                        );
                        params.push(Box::new(*id));
                    }
                    // A required tag the pack doesn't define can never be satisfied.
                    None => where_queries.push("FALSE".to_string()),
                }
            }

            let none_ids: Vec<u64> = filter
                .none
                .iter()
                .filter_map(|tag| self.tag_map.get(tag).copied())
                .collect();

            if !none_ids.is_empty() {
                where_queries.push(format!(
                    "NOT EXISTS (SELECT 1 FROM media_tags
                     WHERE media_id = media.id AND tag_id IN ({}))",
                    repeat_vars(none_ids.len())
                ));

                for id in none_ids {
                    params.push(Box::new(id));
                }
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

        (sql, params)
    }

    pub fn get_media(&self, name: String, types: MediaTypes) -> Result<Option<Media>> {
        let (sql, params) = self.build_sql(MediaOpts {
            name: Some(name),
            id: None,
            types,
            tags: None,
            random: false,
            single: true,
        });

        let mut stmt = self.db.prepare(&sql)?;

        stmt.query_and_then(params_from_iter(params), parse_media)?
            .next()
            .transpose()
    }

    /// One file by its media id -- how the behaviour document's wallpaper/splash slots address
    /// media (see `shared::behaviour::Content`). `None` means the pack no longer has it, which
    /// for an id means it was deleted rather than merely renamed.
    pub fn get_media_by_id(&self, id: u64) -> Result<Option<Media>> {
        let (sql, params) = self.build_sql(MediaOpts {
            name: None,
            id: Some(id),
            types: MediaTypes::ALL,
            tags: None,
            random: false,
            single: true,
        });

        let mut stmt = self.db.prepare(&sql)?;

        stmt.query_and_then(params_from_iter(params), parse_media)?
            .next()
            .transpose()
    }

    pub fn random_media(
        &self,
        types: MediaTypes,
        tags: Option<TagFilter>,
    ) -> Result<Option<Media>> {
        let (sql, params) = self.build_sql(MediaOpts {
            name: None,
            id: None,
            types,
            tags,
            random: true,
            single: true,
        });

        let mut stmt = self.db.prepare(&sql)?;

        stmt.query_and_then(params_from_iter(params), parse_media)?
            .next()
            .transpose()
    }

    pub fn list_media(&self, types: MediaTypes, tags: Option<TagFilter>) -> Result<Vec<Media>> {
        let (sql, params) = self.build_sql(MediaOpts {
            name: None,
            id: None,
            types,
            tags,
            random: false,
            single: false,
        });

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

    pub async fn get_image_file(&self, id: u64) -> Result<ExtractedFile> {
        let (offset, length) = self.get_offset_length(id)?;

        Ok(ExtractedFile(
            self.write_to_temp_file(offset, length, ".avif").await?,
        ))
    }

    pub fn get_video_data(&self, id: u64) -> Result<VideoData> {
        let (offset, length, transparent) = self.db.query_row(
            "SELECT offset, length, transparent FROM media WHERE id = ?",
            params![id],
            |row| {
                Ok((
                    row.get("offset")?,
                    row.get("length")?,
                    row.get("transparent")?,
                ))
            },
        )?;

        Ok(VideoData {
            source: self.media_source(offset, length),
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

    /// Reads a named blob from the pack's `pack_data` table (e.g. behaviour.json). `None` if the
    /// pack doesn't carry an entry with this name -- packs predating a given key, or not written
    /// by a tool that populates it, are expected, not an error.
    /// The pack's behaviour document, assembled from the `behaviour_*` tables and what is left of
    /// the `pack_data` blob -- see `shared::behaviour::storage`. The engine sees one `Behaviour`
    /// either way; where each part is stored is that module's business.
    pub fn get_behaviour(&self) -> anyhow::Result<shared::behaviour::Behaviour> {
        shared::behaviour::storage::read(&self.db)
    }

    /// The pack's full tag vocabulary, sorted for a stable order -- not a query, so (unlike
    /// `list_media`'s tag filtering) this is never affected by the user's tag exclusion list: a
    /// tag whose every file happens to be excluded is still listed.
    pub fn list_tags(&self) -> Result<Vec<String>> {
        let mut tags: Vec<String> = self.tag_map.keys().cloned().collect();
        tags.sort();
        Ok(tags)
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

    pub fn header(&self) -> &Header {
        &self.header
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

    let tags = row
        .get::<_, Option<String>>("tag_names")?
        .map(|names| names.split('\u{1f}').map(str::to_string).collect())
        .unwrap_or_default();

    Ok(Media {
        id: row.get("id")?,
        name: row.get("file_name")?,
        tags,
        media_data,
    })
}

#[cfg(test)]
mod tests;

