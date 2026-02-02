use std::{
    collections::HashMap,
    fmt::Display,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use image::{ImageFormat, ImageReader};
use rusqlite::{Connection, Row, params, params_from_iter};
use shared::{
    encode::FileInfo,
    pack_config::Metadata,
    read_pack::{Header, read_pack_metadata},
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

#[derive(Debug, Clone, PartialEq)]
pub enum MediaType {
    Image,
    Video,
    Audio,
    Other,
}

impl MediaType {
    fn from_str(s: &str) -> Self {
        match s {
            "image" => MediaType::Image,
            "video" => MediaType::Video,
            "audio" => MediaType::Audio,
            _ => MediaType::Other,
        }
    }

    fn to_str(&self) -> &'static str {
        match self {
            MediaType::Image => "image",
            MediaType::Video => "video",
            MediaType::Audio => "audio",
            MediaType::Other => "other",
        }
    }
}

#[derive(Debug, Clone)]
struct MediaEntry {
    pub id: i64,
    pub file_name: String,
    pub file_info: FileInfo,
    pub offset: u64,
    pub length: u64,
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

/// A media pack, consisting of a header, some metadata and an SQLite database at the end, which
/// contains information about all the media in the file. The database stores the offset and length
/// of each image/video/audio file, which can be used to read it from the pack file.
pub struct MediaPack {
    path: PathBuf,
    db: Connection,
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
            SELECT id, file_name, file_type, offset, length, width, height, duration, audio
            FROM media
        "
        .to_string();

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if opts.tags.is_some() {
            sql.push_str(" LEFT JOIN media_tags ON media.id = media_tags.media_id ");
        }

        let mut where_queries = Vec::new();

        if let Some(name) = &opts.name {
            where_queries.push("WHERE name = ?".to_string());
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
        let (offset, length, width, height) = self.db.query_row(
            "SELECT offset, length, width, height FROM media WHERE id = ?",
            params![id],
            |row| {
                Ok((
                    row.get("offset")?,
                    row.get("Length")?,
                    row.get("width")?,
                    row.get("height")?,
                ))
            },
        )?;

        Ok(VideoData {
            file: FileOrPath::File(self.write_to_temp_file(offset, length, ".webm").await?),
            width,
            height,
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

    // pub async fn get_image(&self, name: String) -> Result<Option<Image>> {
    //     let mut stmt = self.db.prepare(r#"
    //         SELECT id, file_name, file_type, offset, length, width, height, transparent, duration, audio
    //         FROM media
    //         WHERE file_type = "image" AND file_name = ?
    //         ORDER BY RANDOM()
    //         LIMIT 1
    //     "#)?;
    //
    //     let media = stmt
    //         .query_and_then(params![name], parse_media_entry)?
    //         .next()
    //         .transpose()?;
    //
    //     Ok(match media {
    //         Some(media) => match media.file_info {
    //             FileInfo::Image {
    //                 width,
    //                 height,
    //                 transparent,
    //             } => Some(Image {
    //                 width,
    //                 height,
    //                 transparent,
    //                 data: self.read_image_data(&media).await?,
    //             }),
    //             _ => bail!("Not an image"),
    //         },
    //         None => None,
    //     })
    // }

    // pub async fn get_video(&self, name: String) -> Result<Option<Video>> {
    //     let connection = self.db_pool.get()?;
    //
    //     let mut stmt = connection.prepare(r#"
    //         SELECT id, file_name, file_type, offset, length, width, height, transparent, duration, audio
    //         FROM media
    //         WHERE file_type = "video" AND file_name = ?
    //         ORDER BY RANDOM()
    //         LIMIT 1
    //     "#)?;
    //
    //     let media = stmt
    //         .query_and_then(params![name], parse_media_entry)?
    //         .next()
    //         .transpose()?;
    //
    //     Ok(match media {
    //         Some(media) => match media.file_info {
    //             FileInfo::Video {
    //                 width,
    //                 height,
    //                 duration,
    //                 audio,
    //             } => Some(Video {
    //                 width,
    //                 height,
    //                 duration,
    //                 audio,
    //                 file: FileOrPath::File(
    //                     self.write_to_temp_file(media.offset, media.length, ".webm")
    //                         .await?,
    //                 ),
    //             }),
    //             _ => bail!("Not an image"),
    //         },
    //         None => None,
    //     })
    // }

    // fn get_random_media_type(&self, media_type: MediaType) -> Result<Option<MediaEntry>> {
    //     let type_str = media_type.to_str();
    //
    //     let connection = self.db_pool.get()?;
    //
    //     let mut stmt = connection.prepare_cached(
    //         "SELECT id, file_name, file_type, offset, length, width, height, transparent, duration, audio
    //          FROM media
    //          WHERE file_type = ?1
    //          ORDER BY random()
    //          LIMIT 1",
    //     )?;
    //
    //     stmt.query_and_then(params![type_str], parse_media_entry)?
    //         .next()
    //         .transpose()
    // }

    // fn get_random_media_type_with_tags(
    //     &self,
    //     media_type: MediaType,
    //     tags: Vec<String>,
    // ) -> Result<Option<MediaEntry>> {
    //     let type_str = media_type.to_str();
    //
    //     let tag_ids: Vec<_> = tags
    //         .iter()
    //         .filter_map(|tag| self.tag_map.get(tag))
    //         .collect();
    //
    //     let sql = format!(
    //         r#"
    //         SELECT id, file_name, file_type, offset, length, width, height, transparent, duration, audio
    //         FROM media
    //         LEFT JOIN media_tags ON media.id = media_tags.media_id
    //         WHERE file_type = ?1
    //         AND media_tags.tag_id IN ({})
    //         ORDER BY random()
    //         LIMIT 1
    //     "#,
    //         repeat_vars(tag_ids.len())
    //     );
    //
    //     let connection = self.db_pool.get()?;
    //
    //     let mut stmt = self.db.prepare_cached(&sql)?;
    //
    //     let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
    //
    //     params_vec.push(&type_str);
    //
    //     for id in tag_ids {
    //         params_vec.push(id);
    //     }
    //
    //     stmt.query_and_then(&*params_vec, parse_media_entry)?
    //         .next()
    //         .transpose()
    // }

    // fn get_random_media(&self, popup_type: PopupType) -> Result<Option<MediaEntry>> {
    //     let sql = format!(
    //         r#"
    //         SELECT id, file_name, file_type, offset, length, width, height, transparent, duration, audio
    //         FROM media
    //         WHERE {type_query}
    //         ORDER BY random()
    //         LIMIT 1
    //     "#,
    //         type_query = popup_type_query(popup_type),
    //     );
    //
    //     let mut stmt = self.db.prepare_cached(&sql)?;
    //
    //     stmt.query_and_then([], parse_media_entry)?
    //         .next()
    //         .transpose()
    // }

    // fn get_random_media_with_tags(
    //     &self,
    //     tags: Vec<String>,
    //     popup_type: PopupType,
    // ) -> Result<Option<MediaEntry>> {
    //     let tag_ids: Vec<_> = tags
    //         .iter()
    //         .filter_map(|tag| self.tag_map.get(tag))
    //         .collect();
    //
    //     let sql = format!(
    //         r#"
    //         SELECT id, file_name, file_type, offset, length, width, height, transparent, duration, audio
    //         FROM media
    //         LEFT JOIN media_tags ON media.id = media_tags.media_id
    //         WHERE {type_query}
    //         AND media_tags.tag_id IN ({vars})
    //         GROUP BY id
    //         ORDER BY random()
    //         LIMIT 1
    //     "#,
    //         type_query = popup_type_query(popup_type),
    //         vars = repeat_vars(tag_ids.len())
    //     );
    //
    //     let mut stmt = self.db.prepare_cached(&sql)?;
    //
    //     stmt.query_and_then(params_from_iter(tag_ids), parse_media_entry)?
    //         .next()
    //         .transpose()
    // }

    /// Get a random image entry
    // pub async fn get_random_image(&self, tags: Option<Vec<String>>) -> Result<Option<Image>> {
    //     let media = match tags {
    //         Some(tags) => self.get_random_media_type_with_tags(MediaType::Image, tags)?,
    //         None => self.get_random_media_type(MediaType::Image)?,
    //     };
    //
    //     Ok(match media {
    //         Some(media) => match media.file_info {
    //             FileInfo::Image {
    //                 width,
    //                 height,
    //                 transparent,
    //             } => Some(Image {
    //                 width,
    //                 height,
    //                 transparent,
    //                 data: self.read_image_data(&media).await?,
    //             }),
    //             _ => bail!("Not an image"),
    //         },
    //         None => None,
    //     })
    // }

    /// Get a random video entry
    // pub async fn get_random_video(&self, tags: Option<Vec<String>>) -> Result<Option<Video>> {
    //     let media = match tags {
    //         Some(tags) => self.get_random_media_type_with_tags(MediaType::Video, tags)?,
    //         None => self.get_random_media_type(MediaType::Video)?,
    //     };
    //
    //     Ok(match media {
    //         Some(media) => match media.file_info {
    //             FileInfo::Video {
    //                 width,
    //                 height,
    //                 duration,
    //                 audio,
    //             } => Some(Video {
    //                 width,
    //                 height,
    //                 duration,
    //                 audio,
    //                 file: FileOrPath::File(
    //                     self.write_to_temp_file(media.offset, media.length, ".webm")
    //                         .await?,
    //                 ),
    //             }),
    //             _ => bail!("Not a video"),
    //         },
    //         None => None,
    //     })
    // }

    /// Get a random popup (either an image or a video).
    // pub async fn get_random_popup(
    //     &self,
    //     tags: Option<Vec<String>>,
    //     popup_type: PopupType,
    // ) -> Result<Option<Media>> {
    //     let media = match tags {
    //         Some(tags) => self.get_random_media_with_tags(tags, popup_type)?,
    //         None => self.get_random_media(popup_type)?,
    //     };
    //
    //     Ok(match media {
    //         Some(media) => match media.file_info {
    //             FileInfo::Image {
    //                 width,
    //                 height,
    //                 transparent,
    //             } => Some(Media::Image(Image {
    //                 width,
    //                 height,
    //                 transparent,
    //                 data: self.read_image_data(&media).await?,
    //             })),
    //             FileInfo::Video {
    //                 width,
    //                 height,
    //                 duration,
    //                 audio,
    //             } => Some(Media::Video(Video {
    //                 width,
    //                 height,
    //                 duration,
    //                 audio,
    //                 file: FileOrPath::File(
    //                     self.write_to_temp_file(media.offset, media.length, ".webm")
    //                         .await?,
    //                 ),
    //             })),
    //             _ => bail!("Not an image or video"),
    //         },
    //         None => None,
    //     })
    // }

    // pub async fn get_random_audio(&self, tags: Option<Vec<String>>) -> Result<Option<Audio>> {
    //     let media = match tags {
    //         Some(tags) => self.get_random_media_type_with_tags(MediaType::Audio, tags)?,
    //         None => self.get_random_media_type(MediaType::Audio)?,
    //     };
    //
    //     Ok(match media {
    //         Some(media) => match media.file_info {
    //             FileInfo::Audio { duration } => Some(Audio {
    //                 duration,
    //                 file: FileOrPath::File(
    //                     self.write_to_temp_file(media.offset, media.length, ".opus")
    //                         .await?,
    //                 ),
    //             }),
    //             _ => bail!("Not audio"),
    //         },
    //         None => None,
    //     })
    // }

    // pub fn get_random_notification(
    //     &self,
    //     tags: Option<Vec<String>>,
    // ) -> Result<Option<Notification>> {
    //     let sql = match &tags {
    //         Some(tags) => format!(
    //             r#"
    //             SELECT body, summary
    //             FROM notifications
    //             LEFT JOIN notification_tags ON notifications.id = notification_tags.notification_id
    //             WHERE notification_tags.tag_id IN ({})
    //             ORDER BY random()
    //             LIMIT 1
    //         "#,
    //             repeat_vars(tags.len())
    //         ),
    //         None => r#"
    //             SELECT body, summary
    //             FROM notifications
    //             ORDER BY random()
    //             LIMIT 1
    //         "#
    //         .to_string(),
    //     };
    //
    //     let mut stmt = self.db.prepare_cached(&sql)?;
    //
    //     let params = params_from_iter(
    //         tags.into_iter()
    //             .flatten()
    //             .map(|x| self.tag_map.get(&x).unwrap()),
    //     );
    //
    //     stmt.query_row(params, |row| {
    //         Ok(Notification {
    //             summary: row.get("summary")?,
    //             body: row.get("body")?,
    //         })
    //     })
    //     .optional()
    //     .map_err(|err| anyhow!(err))
    // }

    // pub fn get_random_link(&self, tags: Option<Vec<String>>) -> Result<Option<Link>> {
    //     let sql = match &tags {
    //         Some(tag_ids) => format!(
    //             r#"
    //             SELECT link
    //             FROM links
    //             LEFT JOIN link_tags ON links.id = link_tags.link_id
    //             WHERE link_tags.tag_id IN ({})
    //             ORDER BY random()
    //             LIMIT 1
    //         "#,
    //             repeat_vars(tag_ids.len())
    //         ),
    //         None => r#"
    //             SELECT link
    //             FROM links
    //             ORDER BY random()
    //             LIMIT 1
    //         "#
    //         .to_string(),
    //     };
    //
    //     let mut stmt = self.db.prepare_cached(&sql)?;
    //
    //     let params = params_from_iter(
    //         tags.into_iter()
    //             .flatten()
    //             .map(|x| self.tag_map.get(&x).unwrap()),
    //     );
    //
    //     stmt.query_row(params, |row| {
    //         Ok(Link {
    //             link: row.get("link")?,
    //         })
    //     })
    //     .optional()
    //     .map_err(|err| anyhow!(err))
    // }

    // pub fn get_random_prompt(&self, tags: Option<Vec<String>>) -> Result<Option<Prompt>> {
    //     let sql = match &tags {
    //         Some(tag_ids) => format!(
    //             r#"
    //             SELECT prompt
    //             FROM prompts
    //             LEFT JOIN prompt_tags ON prompts.id = prompt_tags.prompt_id
    //             WHERE prompt_tags.tag_id IN ({})
    //             ORDER BY random()
    //             LIMIT 1
    //         "#,
    //             repeat_vars(tag_ids.len())
    //         ),
    //         None => r#"
    //             SELECT prompt
    //             FROM prompts
    //             ORDER BY random()
    //             LIMIT 1
    //         "#
    //         .to_string(),
    //     };
    //
    //     let mut stmt = self.db.prepare_cached(&sql)?;
    //
    //     let params = params_from_iter(
    //         tags.into_iter()
    //             .flatten()
    //             .map(|x| self.tag_map.get(&x).unwrap()),
    //     );
    //
    //     stmt.query_row(params, |row| {
    //         Ok(Prompt {
    //             prompt: row.get("prompt")?,
    //         })
    //     })
    //     .optional()
    //     .map_err(|err| anyhow!(err))
    // }

    // pub async fn get_random_wallpaper(
    //     &self,
    //     tags: Option<Vec<String>>,
    // ) -> Result<Option<Wallpaper>> {
    //     // TODO: Fix this
    //     let sql = match &tags {
    //         Some(tag_ids) => format!(
    //             r#"
    //             SELECT id, file_name, offset, length
    //             FROM wallpapers
    //             LEFT JOIN wallpaper_tags ON wallpapers.id = wallpaper_tags.wallpaper_id
    //             WHERE wallpaper_tags.tag_id IN ({})
    //             ORDER BY random()
    //             LIMIT 1
    //         "#,
    //             repeat_vars(tag_ids.len())
    //         ),
    //         None => r#"
    //             SELECT id, file_name, offset, length
    //             FROM wallpapers
    //             ORDER BY random()
    //             LIMIT 1
    //         "#
    //         .to_string(),
    //     };
    //
    //     let mut stmt = self.db.prepare_cached(&sql)?;
    //
    //     let params = params_from_iter(
    //         tags.into_iter()
    //             .flatten()
    //             .map(|x| self.tag_map.get(&x).unwrap()),
    //     );
    //
    //     let media = stmt
    //         .query_and_then(params, parse_media_entry)?
    //         .next()
    //         .transpose()?;
    //
    //     Ok(match media {
    //         Some(media) => match media.file_info {
    //             FileInfo::Image {
    //                 width,
    //                 height,
    //                 transparent,
    //             } => Some(Wallpaper {
    //                 width,
    //                 height,
    //                 transparent,
    //                 file: FileOrPath::File(
    //                     self.write_to_temp_file(media.offset, media.length, ".avif")
    //                         .await?,
    //                 ),
    //             }),
    //             _ => bail!("Not an image"),
    //         },
    //         None => None,
    //     })
    // }

    /// Extract file data for a given entry
    async fn extract_file_data(&self, entry: &MediaEntry) -> Result<Vec<u8>> {
        let mut file = File::open(&self.path).await?;
        file.seek(SeekFrom::Start(entry.offset)).await?;
        let mut buffer = vec![0u8; entry.length as usize];
        file.read_exact(&mut buffer).await?;
        Ok(buffer)
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
                        .resize_exact(width, height, image::imageops::FilterType::Lanczos3)
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

    /// Extract file data and write to a path
    async fn extract_file_to_path(&self, entry: &MediaEntry, output_path: &Path) -> Result<()> {
        let data = self.extract_file_data(entry).await?;
        std::fs::write(output_path, data)?;
        Ok(())
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
        },
        "video" => MediaData::Video {
            width: row.get("width")?,
            height: row.get("height")?,
            duration: row.get("duration")?,
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
