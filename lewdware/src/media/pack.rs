use std::{collections::HashMap, fs, io::{Cursor, Read, Seek, SeekFrom, Write}, path::{Path, PathBuf}};

use anyhow::{anyhow, Result};
use async_fs::File;
use futures_lite::{AsyncReadExt, AsyncSeekExt};
use image::{ImageFormat, ImageReader};
use pack_format::{config::Metadata, Header};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use tempfile::NamedTempFile;

use crate::utils::read_pack_metadata;


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
    pub path: String,
    pub media_type: MediaType,
    pub offset: u64,
    pub length: u64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<f64>,
}

impl MediaEntry {
    pub async fn into_image(self, media_manager: &MediaPack) -> Result<Image> {
        assert_eq!(self.media_type, MediaType::Image);

        media_manager.read_image_data(&self).await
    }

    pub async fn into_video(self, media_manager: &MediaPack) -> Result<Video> {
        assert_eq!(self.media_type, MediaType::Video);

        let tempfile = media_manager
            .write_to_temp_file(self.offset, self.length, ".webm")
            .await?;

        Ok(Video {
            width: self.width.unwrap(),
            height: self.height.unwrap(),
            tempfile,
        })
    }

    pub async fn into_audio(self, media_manager: &MediaPack) -> Result<Audio> {
        assert_eq!(self.media_type, MediaType::Audio);

        let tempfile = media_manager
            .write_to_temp_file(self.offset, self.length, ".opus")
            .await?;

        Ok(Audio { tempfile })
    }

    pub async fn into_wallpaper(self, media_manager: &MediaPack) -> Result<NamedTempFile> {
        assert_eq!(self.media_type, MediaType::Image);

        let tempfile = media_manager
            .write_to_temp_file(self.offset, self.length, ".avif")
            .await?;

        Ok(tempfile)
    }
}

pub enum Media {
    Image(Image),
    Video(Video),
}

pub type Image = image::ImageBuffer<image::Rgba<u8>, Vec<u8>>;

pub struct Video {
    pub width: i64,
    pub height: i64,
    pub tempfile: NamedTempFile,
}

pub struct Audio {
    pub tempfile: NamedTempFile,
}

pub struct Notification {
    pub summary: Option<String>,
    pub body: String,
}

pub struct Link {
    pub link: String,
}

pub struct Prompt {
    pub prompt: String,
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
    metadata: Metadata,
    temp_file: NamedTempFile,
    tag_map: HashMap<String, u64>,
}

impl MediaPack {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut file = fs::File::open(&path)?;

        let (header, metadata) = read_pack_metadata(&mut file)?;

        println!("{}", metadata.name);

        // Extract the SQLite database to a temporary location
        file.seek(SeekFrom::Start(header.index_offset))?;
        let mut db_data = Vec::new();
        file.read_to_end(&mut db_data)?;

        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(&db_data).unwrap();

        let db = Connection::open(temp_file.path())?;

        let mut tag_map: HashMap<String, u64> = HashMap::new();

        {
            let mut stmt = db.prepare("SELECT id, name FROM tags")?;

            stmt.query_map(params![], |row| {
                tag_map.insert(row.get("name")?, row.get("id")?);
                Ok(())
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        }

        Ok(MediaPack {
            path,
            db,
            header,
            metadata,
            temp_file,
            tag_map,
        })
    }

    fn get_random_media_type(&self, media_type: MediaType) -> Result<Option<MediaEntry>> {
        let type_str = media_type.to_str();

        let mut stmt = self.db.prepare_cached(
            "SELECT id, path, media_type, offset, length, width, height, duration
             FROM media
             WHERE media_type = ?1
             ORDER BY random()
             LIMIT 1",
        )?;

        stmt.query_row(params![type_str], |row| {
            Ok(MediaEntry {
                id: row.get("id")?,
                path: row.get("path")?,
                media_type: MediaType::from_str(&row.get::<_, String>("media_type")?),
                offset: row.get::<_, i64>("offset")? as u64,
                length: row.get::<_, i64>("length")? as u64,
                width: row.get("width")?,
                height: row.get("height")?,
                duration: row.get("duration")?,
            })
        })
        .optional()
        .map_err(|err| anyhow!(err))
    }

    fn get_random_media_type_with_tags(
        &self,
        media_type: MediaType,
        tags: Vec<String>,
    ) -> Result<Option<MediaEntry>> {
        let type_str = media_type.to_str();

        let tag_ids: Vec<_> = tags
            .iter()
            .filter_map(|tag| self.tag_map.get(tag))
            .collect();

        let sql = format!(
            r#"
            SELECT id, path, media_type, offset, length, width, height, duration
            FROM media
            LEFT JOIN media_tags ON media.id = media_tags.media_id
            WHERE media_type = ?1
            AND media_tags.tag_id IN ({})
            ORDER BY random()
            LIMIT 1
        "#,
            repeat_vars(tag_ids.len())
        );

        let mut stmt = self.db.prepare_cached(&sql)?;

        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();

        params_vec.push(&type_str);

        for id in tag_ids {
            params_vec.push(id);
        }

        stmt.query_row(&*params_vec, |row| {
            Ok(MediaEntry {
                id: row.get("id")?,
                path: row.get("path")?,
                media_type: MediaType::from_str(&row.get::<_, String>("media_type")?),
                offset: row.get::<_, i64>("offset")? as u64,
                length: row.get::<_, i64>("length")? as u64,
                width: row.get("width")?,
                height: row.get("height")?,
                duration: row.get("duration")?,
            })
        })
        .optional()
        .map_err(|err| anyhow!(err))
    }

    fn get_random_media(&self) -> Result<Option<MediaEntry>> {
        let mut stmt = self.db.prepare_cached(
            "SELECT id, path, media_type, offset, length, width, height, duration
             FROM media
             WHERE media_type = 'image' OR media_type = 'video'
             ORDER BY random()
             LIMIT 1",
        )?;

        stmt.query_row([], |row| {
            Ok(MediaEntry {
                id: row.get("id")?,
                path: row.get("path")?,
                media_type: MediaType::from_str(&row.get::<_, String>("media_type")?),
                offset: row.get::<_, i64>("offset")? as u64,
                length: row.get::<_, i64>("length")? as u64,
                width: row.get("width")?,
                height: row.get("height")?,
                duration: row.get("duration")?,
            })
        })
        .optional()
        .map_err(|err| anyhow!(err))
    }

    fn get_random_media_with_tags(&self, tags: Vec<String>) -> Result<Option<MediaEntry>> {
        let tag_ids: Vec<_> = tags
            .iter()
            .filter_map(|tag| self.tag_map.get(tag))
            .collect();

        let sql = format!(
            r#"
            SELECT id, path, media_type, offset, length, width, height, duration
            FROM media
            LEFT JOIN media_tags ON media.id = media_tags.media_id
            WHERE (media_type = "image" OR media_type = "video")
            AND media_tags.tag_id IN ({})
            GROUP BY id
            ORDER BY random()
            LIMIT 1
        "#,
            repeat_vars(tag_ids.len())
        );

        let mut stmt = self.db.prepare_cached(&sql)?;

        stmt.query_row(params_from_iter(tag_ids), |row| {
            Ok(MediaEntry {
                id: row.get("id")?,
                path: row.get("path")?,
                media_type: MediaType::from_str(&row.get::<_, String>("media_type")?),
                offset: row.get::<_, i64>("offset")? as u64,
                length: row.get::<_, i64>("length")? as u64,
                width: row.get("width")?,
                height: row.get("height")?,
                duration: row.get("duration")?,
            })
        })
        .optional()
        .map_err(|err| anyhow!(err))
    }

    /// Get a random image entry
    pub async fn get_random_image(&self, tags: Option<Vec<String>>) -> Result<Option<Image>> {
        let media = match tags {
            Some(tags) => self.get_random_media_type_with_tags(MediaType::Image, tags)?,
            None => self.get_random_media_type(MediaType::Image)?,
        };

        Ok(match media {
            Some(x) => Some(x.into_image(self).await?),
            None => None,
        })
    }

    /// Get a random video entry
    pub async fn get_random_video(&self, tags: Option<Vec<String>>) -> Result<Option<Video>> {
        let media = match tags {
            Some(tags) => self.get_random_media_type_with_tags(MediaType::Video, tags)?,
            None => self.get_random_media_type(MediaType::Video)?,
        };

        Ok(match media {
            Some(x) => Some(x.into_video(self).await?),
            None => None,
        })
    }

    /// Get a random popup (either an image or a video).
    pub async fn get_random_popup(&self, tags: Option<Vec<String>>) -> Result<Option<Media>> {
        let media = match tags {
            Some(tags) => self.get_random_media_with_tags(tags)?,
            None => self.get_random_media()?,
        };

        Ok(match media {
            Some(media) => {
                if media.media_type == MediaType::Image {
                    Some(Media::Image(media.into_image(self).await?))
                } else {
                    Some(Media::Video(media.into_video(self).await?))
                }
            }
            None => None,
        })
    }

    pub async fn get_random_audio(&self, tags: Option<Vec<String>>) -> Result<Option<Audio>> {
        let media = match tags {
            Some(tags) => self.get_random_media_type_with_tags(MediaType::Audio, tags)?,
            None => self.get_random_media_type(MediaType::Audio)?,
        };

        Ok(match media {
            Some(x) => Some(x.into_audio(self).await?),
            None => None,
        })
    }

    pub fn get_random_notification(
        &self,
        tags: Option<Vec<String>>,
    ) -> Result<Option<Notification>> {
        let sql = match &tags {
            Some(tags) => format!(
                r#"
                SELECT body, summary
                FROM notifications
                LEFT JOIN notification_tags ON notifications.id = notification_tags.notification_id
                WHERE notification_tags.tag_id IN ({})
                ORDER BY random()
                LIMIT 1
            "#,
                repeat_vars(tags.len())
            ),
            None => r#"
                SELECT body, summary
                FROM notifications
                ORDER BY random()
                LIMIT 1
            "#
            .to_string(),
        };

        let mut stmt = self.db.prepare_cached(&sql)?;

        let params = params_from_iter(
            tags.into_iter()
                .flatten()
                .map(|x| self.tag_map.get(&x).unwrap()),
        );

        stmt.query_row(params, |row| {
            Ok(Notification {
                summary: row.get("summary")?,
                body: row.get("body")?,
            })
        })
        .optional()
        .map_err(|err| anyhow!(err))
    }

    pub fn get_random_link(&self, tags: Option<Vec<String>>) -> Result<Option<Link>> {
        let sql = match &tags {
            Some(tag_ids) => format!(
                r#"
                SELECT link
                FROM links
                LEFT JOIN link_tags ON links.id = link_tags.link_id
                WHERE link_tags.tag_id IN ({})
                ORDER BY random()
                LIMIT 1
            "#,
                repeat_vars(tag_ids.len())
            ),
            None => r#"
                SELECT link
                FROM links
                ORDER BY random()
                LIMIT 1
            "#
            .to_string(),
        };

        let mut stmt = self.db.prepare_cached(&sql)?;

        let params = params_from_iter(
            tags.into_iter()
                .flatten()
                .map(|x| self.tag_map.get(&x).unwrap()),
        );

        stmt.query_row(params, |row| {
            Ok(Link {
                link: row.get("link")?,
            })
        })
        .optional()
        .map_err(|err| anyhow!(err))
    }

    pub fn get_random_prompt(&self, tags: Option<Vec<String>>) -> Result<Option<Prompt>> {
        let sql = match &tags {
            Some(tag_ids) => format!(
                r#"
                SELECT prompt
                FROM prompts
                LEFT JOIN prompt_tags ON prompts.id = prompt_tags.prompt_id
                WHERE prompt_tags.tag_id IN ({})
                ORDER BY random()
                LIMIT 1
            "#,
                repeat_vars(tag_ids.len())
            ),
            None => r#"
                SELECT prompt
                FROM prompts
                ORDER BY random()
                LIMIT 1
            "#
            .to_string(),
        };

        let mut stmt = self.db.prepare_cached(&sql)?;

        let params = params_from_iter(
            tags.into_iter()
                .flatten()
                .map(|x| self.tag_map.get(&x).unwrap()),
        );

        stmt.query_row(params, |row| {
            Ok(Prompt {
                prompt: row.get("prompt")?,
            })
        })
        .optional()
        .map_err(|err| anyhow!(err))
    }

    pub async fn get_random_wallpaper(
        &self,
        tags: Option<Vec<String>>,
    ) -> Result<Option<NamedTempFile>> {
        let sql = match &tags {
            Some(tag_ids) => format!(
                r#"
                SELECT id, path, offset, length
                FROM wallpapers
                LEFT JOIN wallpaper_tags ON wallpapers.id = wallpaper_tags.wallpaper_id
                WHERE wallpaper_tags.tag_id IN ({})
                ORDER BY random()
                LIMIT 1
            "#,
                repeat_vars(tag_ids.len())
            ),
            None => r#"
                SELECT id, path, offset, length
                FROM wallpapers
                ORDER BY random()
                LIMIT 1
            "#
            .to_string(),
        };

        let mut stmt = self.db.prepare_cached(&sql)?;

        let params = params_from_iter(
            tags.into_iter()
                .flatten()
                .map(|x| self.tag_map.get(&x).unwrap()),
        );

        let media = stmt
            .query_row(params, |row| {
                Ok(MediaEntry {
                    id: row.get("id")?,
                    path: row.get("path")?,
                    media_type: MediaType::Image,
                    offset: row.get::<_, i64>("offset")? as u64,
                    length: row.get::<_, i64>("length")? as u64,
                    width: None,
                    height: None,
                    duration: None,
                })
            })
            .optional()?;

        Ok(match media {
            Some(media) => Some(media.into_wallpaper(self).await?),
            None => None,
        })
    }

    /// Extract file data for a given entry
    async fn extract_file_data(&self, entry: &MediaEntry) -> Result<Vec<u8>> {
        let mut file = File::open(&self.path).await?;
        file.seek(SeekFrom::Start(entry.offset)).await?;
        let mut buffer = vec![0u8; entry.length as usize];
        file.read_exact(&mut buffer).await?;
        Ok(buffer)
    }

    async fn read_image_data(&self, entry: &MediaEntry) -> Result<Image> {
        let mut buffer = vec![0u8; entry.length as usize];

        let mut file = File::open(&self.path).await?;
        file.seek(SeekFrom::Start(entry.offset)).await?;

        file.read_exact(&mut buffer).await?;

        let mut reader = ImageReader::new(Cursor::new(buffer));

        reader.set_format(ImageFormat::Avif);

        let image = reader.decode()?;

        Ok(image.into_rgba8())
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
