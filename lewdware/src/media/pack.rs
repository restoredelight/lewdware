use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

use image::{ImageFormat, ImageReader};
use rusqlite::{Connection, Row, params, params_from_iter};
use shared::{
    db::migrate, read_pack::{Header, Metadata, read_pack_metadata}
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
        types::{FileOrPath, ImageData},
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
    _db_file: NamedTempFile,
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

        // Extract the SQLite database to a temporary location
        file.seek(SeekFrom::Start(header.index_offset))?;
        let mut db_data = vec![0u8; header.index_length as usize];
        file.read_exact(&mut db_data)?;

        let mut db_file = NamedTempFile::new()?;
        db_file.write_all(&db_data)?;

        let connection = Connection::open(db_file.path())?;

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
            _db_file: db_file,
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

    pub async fn get_video_data(&self, id: u64) -> Result<VideoData> {
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
            file: FileOrPath::File(self.write_to_temp_file(offset, length, ".mp4").await?),
            width,
            height,
            transparent,
        })
    }

    pub async fn get_audio_data(&self, id: u64) -> Result<FileOrPath> {
        let (offset, length) = self.get_offset_length(id)?;

        Ok(FileOrPath::File(
            self.write_to_temp_file(offset, length, ".opus").await?,
        ))
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

        let result = if image.width() != width || image.height() != height {
            let (tx, rx) = oneshot::channel();

            rayon::spawn(move || {
                let _ = tx.send(
                    image
                        .resize_exact(width, height, image::imageops::FilterType::Triangle)
                        .into_rgba8(),
                );
            });

            rx.await
                .map_err(|_| MediaError::Internal("Image resizing sender dropped"))
        } else {
            Ok(image.into_rgba8())
        };

        result
    }

    async fn write_to_temp_file(
        &self,
        offset: u64,
        length: u64,
        suffix: &str,
    ) -> Result<NamedTempFile> {
        let mut tempfile = NamedTempFile::with_suffix(suffix)?;
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
