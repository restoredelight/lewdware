use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    rc::Rc,
};

use image::{ImageFormat, ImageReader};
use rusqlite::{Connection, MAIN_DB, Row, named_params, params, types::Value, vtab::array::Array};
use shared::{
    db::migrate,
    pack::{HEADER_SIZE, Header, Metadata},
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

/// A pack consisting of a header, raw media, and an SQLite database at the end. The database stores
/// the pack metadata plus the offset and length of each image/video/audio file, which can be used
/// to read it from the pack file.
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

struct MediaParams {
    name: Option<String>,
    id: Option<u64>,
    all_types: bool,
    images: bool,
    videos: bool,
    audio: bool,
    random_order: bool,
    single_row: bool,
    any_ids: Array,
    any_count: usize,
    any_enabled: bool,
    all_ids: Array,
    all_count: usize,
    none_ids: Array,
    none_enabled: bool,
}

fn array_param(ids: impl IntoIterator<Item = u64>) -> Array {
    Rc::new(
        ids.into_iter()
            .map(|id| Value::Integer(id as i64))
            .collect(),
    )
}

impl MediaPack {
    pub fn open(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        let mut file = fs::File::open(&path)?;

        let mut header_bytes = [0; HEADER_SIZE];
        file.read_exact(&mut header_bytes)?;
        let header = Header::from_buf(header_bytes)?;

        // Load the SQLite database straight into memory (no temp file: `deserialize_read_exact`
        // hands the bytes we just read directly to SQLite's own in-memory representation via
        // `sqlite3_deserialize`).
        file.seek(SeekFrom::Start(header.index_offset))?;
        let mut db_data = vec![0u8; header.index_length as usize];
        file.read_exact(&mut db_data)?;

        let mut connection = Connection::open_in_memory()?;
        connection.deserialize_read_exact(MAIN_DB, db_data.as_slice(), db_data.len(), false)?;

        migrate(&mut connection)?;
        let metadata = Metadata::read(&connection)?;

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

    fn media_params(&self, opts: MediaOpts) -> MediaParams {
        let mut any_ids = Vec::new();
        let mut any_count = 0;
        let mut all_ids = Vec::new();
        let mut all_count = 0;
        let mut none_ids = Vec::new();
        let mut any_enabled = false;
        let mut none_enabled = false;

        if let Some(filter) = &opts.tags {
            // Unknown tags are ignored for `any` and `none`, while an unknown `all` tag makes the
            // filter unsatisfiable. The requested count preserves that distinction below.
            any_ids = filter
                .any
                .iter()
                .filter_map(|tag| self.tag_map.get(tag).copied())
                .collect();
            any_count = filter.any.len();
            any_enabled = !any_ids.is_empty();

            all_ids = filter
                .all
                .iter()
                .filter_map(|tag| self.tag_map.get(tag).copied())
                .collect();
            all_count = filter.all.len();

            none_ids = filter
                .none
                .iter()
                .filter_map(|tag| self.tag_map.get(tag).copied())
                .collect();
            none_enabled = !none_ids.is_empty();
        }

        MediaParams {
            name: opts.name,
            id: opts.id,
            all_types: opts.types == MediaTypes::ALL,
            images: opts.types.image,
            videos: opts.types.video,
            audio: opts.types.audio,
            random_order: opts.random,
            single_row: opts.single,
            any_ids: array_param(any_ids),
            any_count,
            any_enabled,
            all_ids: array_param(all_ids),
            all_count,
            none_ids: array_param(none_ids),
            none_enabled,
        }
    }

    pub fn get_media(&self, name: String, types: MediaTypes) -> Result<Option<Media>> {
        Ok(self
            .query_media(MediaOpts {
                name: Some(name),
                id: None,
                types,
                tags: None,
                random: false,
                single: true,
            })?
            .into_iter()
            .next())
    }

    /// One file by its media id -- how the behaviour document's wallpaper/splash slots address
    /// media (see `shared::behaviour::Content`). `None` means the pack no longer has it, which
    /// for an id means it was deleted rather than merely renamed.
    pub fn get_media_by_id(&self, id: u64) -> Result<Option<Media>> {
        Ok(self
            .query_media(MediaOpts {
                name: None,
                id: Some(id),
                types: MediaTypes::ALL,
                tags: None,
                random: false,
                single: true,
            })?
            .into_iter()
            .next())
    }

    pub fn random_media(
        &self,
        types: MediaTypes,
        tags: Option<TagFilter>,
    ) -> Result<Option<Media>> {
        Ok(self
            .query_media(MediaOpts {
                name: None,
                id: None,
                types,
                tags,
                random: true,
                single: true,
            })?
            .into_iter()
            .next())
    }

    pub fn list_media(&self, types: MediaTypes, tags: Option<TagFilter>) -> Result<Vec<Media>> {
        self.query_media(MediaOpts {
            name: None,
            id: None,
            types,
            tags,
            random: false,
            single: false,
        })
    }

    fn query_media(&self, opts: MediaOpts) -> Result<Vec<Media>> {
        let params = self.media_params(opts);
        let MediaParams {
            name,
            id,
            all_types,
            images,
            videos,
            audio,
            random_order,
            single_row,
            any_ids,
            any_count,
            any_enabled,
            all_ids,
            all_count,
            none_ids,
            none_enabled,
        } = params;
        let mut statement = self.db.prepare(
            "SELECT id, file_name, file_type, offset, length, width, height, duration, audio, transparent,
                    (SELECT group_concat(tags.name, char(31))
                     FROM media_tags
                     JOIN tags ON tags.id = media_tags.tag_id
                     WHERE media_tags.media_id = media.id) AS tag_names
             FROM media
             WHERE (:name IS NULL OR file_name = :name)
               AND (:id IS NULL OR id = :id)
               AND (:all_types OR
                    (:images AND file_type = 'image') OR
                    (:videos AND file_type = 'video') OR
                    (:audio AND file_type = 'audio'))
               AND (:any_count = 0 OR (:any_enabled = TRUE AND EXISTS (SELECT 1 FROM media_tags
                    WHERE media_id = media.id
                      AND tag_id IN (SELECT value FROM rarray(:any_ids)))))
               AND (:all_count = 0 OR
                    (SELECT COUNT(*) FROM rarray(:all_ids)
                     WHERE value IN (SELECT tag_id FROM media_tags WHERE media_id = media.id))
                    = :all_count)
               AND (:none_enabled = FALSE OR NOT EXISTS (SELECT 1 FROM media_tags
                    WHERE media_id = media.id
                      AND tag_id IN (SELECT value FROM rarray(:none_ids))))
             ORDER BY CASE WHEN :random_order THEN RANDOM() ELSE 0 END
             LIMIT CASE WHEN :single_row THEN 1 ELSE -1 END",
        )?;
        statement
            .query_and_then(
                named_params![
                    ":name": name,
                    ":id": id,
                    ":all_types": all_types,
                    ":images": images,
                    ":videos": videos,
                    ":audio": audio,
                    ":random_order": random_order,
                    ":single_row": single_row,
                    ":any_ids": any_ids,
                    ":any_count": any_count,
                    ":any_enabled": any_enabled,
                    ":all_ids": all_ids,
                    ":all_count": all_count,
                    ":none_ids": none_ids,
                    ":none_enabled": none_enabled,
                ],
                parse_media,
            )?
            .collect()
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
    /// the `pack_data` blob -- see `shared::behaviour::Behaviour`. The engine sees one `Behaviour`
    /// either way; where each part is stored is that module's business.
    pub fn get_behaviour(&self) -> anyhow::Result<shared::behaviour::Behaviour> {
        shared::behaviour::Behaviour::read(&self.db)
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
