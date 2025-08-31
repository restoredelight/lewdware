use anyhow::{Result, anyhow};
use byteorder::{LittleEndian, ReadBytesExt};
use image::{DynamicImage, ImageFormat, ImageReader};
use rand::prelude::IndexedRandom;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use tempfile::NamedTempFile;

const MAGIC: &[u8; 5] = b"MPACK";
const VERSION: u8 = 1;
const _HEADER_SIZE: usize = 32;

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
            MediaType::Other => "other"
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaEntry {
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
    pub fn into_image(self, media_manager: &mut MediaManager) -> Result<DynamicImage> {
        assert_eq!(self.media_type, MediaType::Image);

        media_manager.read_image_data(&self)
    }

    pub fn into_video(self) -> Result<Video> {
        assert_eq!(self.media_type, MediaType::Video);

        Ok(Video {
            id: self.id,
            path: self.path,
            width: self.width.unwrap(),
            height: self.height.unwrap(),
            offset: self.offset,
            length: self.length,
        })
    }
}

pub enum Media {
    Image(DynamicImage),
    Video(Video)
}

pub struct Image {
    pub id: i64,
    pub path: String,
    pub width: i64,
    pub height: i64,
    pub image: DynamicImage,
}

pub struct Video {
    pub id: i64,
    pub path: String,
    pub width: i64,
    pub height: i64,
    offset: u64,
    length: u64,
}

#[derive(Debug)]
struct Header {
    index_offset: u64,
    total_files: u32,
}

pub struct MediaManager {
    file: File,
    db: Connection,
    header: Header,
    temp_file: NamedTempFile,
    tag_map: HashMap<String, u64>,
}

impl MediaManager {
    /// Open a media pack file for reading
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(path)?;

        // Read and validate header
        let header = Self::read_header(&mut file)?;

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
            })?.collect::<rusqlite::Result<Vec<_>>>()?;
        }

        Ok(MediaManager {
            file,
            db,
            header,
            temp_file,
            tag_map,
        })
    }

    fn read_header(file: &mut File) -> Result<Header> {
        file.seek(SeekFrom::Start(0))?;

        // Read and validate magic
        let mut magic = [0u8; 5];
        file.read_exact(&mut magic)?;
        anyhow::ensure!(magic == *MAGIC, "Invalid magic bytes");

        // Read version
        let version = file.read_u8()?;
        anyhow::ensure!(version == VERSION, "Unsupported version: {}", version);

        // Skip reserved bytes
        file.read_u16::<LittleEndian>()?;

        // Read header data
        let index_offset = file.read_u64::<LittleEndian>()?;
        let total_files = file.read_u32::<LittleEndian>()?;

        Ok(Header {
            index_offset,
            total_files,
        })
    }

    /// Get total number of files in the pack
    pub fn total_files(&self) -> u32 {
        self.header.total_files
    }

    /// Get all media entries
    pub fn get_all_entries(&self) -> Result<Vec<MediaEntry>> {
        let mut stmt = self.db.prepare(
            "SELECT m.id, m.path, m.media_type, m.offset, m.length, m.width, m.height, m.duration,
                    GROUP_CONCAT(t.name, '|') as tags
             FROM media m
             LEFT JOIN media_tags mt ON m.id = mt.media_id
             LEFT JOIN tags t ON mt.tag_id = t.id
             GROUP BY m.id
             ORDER BY m.path",
        )?;

        let entries: rusqlite::Result<Vec<_>> = stmt
            .query_map([], |row| {
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
            })?
            .collect();

        entries.map_err(|err| anyhow::anyhow!(err))
    }

    /// Get entries by media type
    pub fn get_entries_by_type(&self, media_type: MediaType) -> Result<Vec<MediaEntry>> {
        let type_str = media_type.to_str();

        let mut stmt = self.db.prepare(
            "SELECT m.id, m.path, m.media_type, m.offset, m.length, m.width, m.height, m.duration,
                    GROUP_CONCAT(t.name, '|') as tags
             FROM media m
             LEFT JOIN media_tags mt ON m.id = mt.media_id
             LEFT JOIN tags t ON mt.tag_id = t.id
             WHERE m.media_type = ?1
             GROUP BY m.id
             ORDER BY m.path",
        )?;

        let entries: rusqlite::Result<Vec<MediaEntry>> = stmt
            .query_map(params![type_str], |row| {
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
            })?
            .collect();

        entries.map_err(|err| anyhow::anyhow!(err))
    }

    /// Get all image entries
    pub fn get_images(&self) -> Result<Vec<MediaEntry>> {
        self.get_entries_by_type(MediaType::Image)
    }

    /// Get all video entries
    pub fn get_videos(&self) -> Result<Vec<MediaEntry>> {
        self.get_entries_by_type(MediaType::Video)
    }

    /// Get all audio entries
    pub fn get_audio(&self) -> Result<Vec<MediaEntry>> {
        self.get_entries_by_type(MediaType::Audio)
    }

    fn get_random_media_type(&self, media_type: MediaType) -> Result<Option<MediaEntry>> {
        let type_str = media_type.to_str();

        let mut stmt = self.db.prepare_cached(
            "SELECT id, path, media_type, offset, length, width, height, duration
             FROM media
             WHERE media_type = ?1
             GROUP BY id
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
        tags: &[&str],
    ) -> Result<Option<MediaEntry>> {
        let type_str = media_type.to_str();

        let tag_ids: Vec<_> = tags
            .iter()
            .filter_map(|tag| self.tag_map.get(*tag))
            .collect();

        let sql = format!(
            r#"
            SELECT id, path, media_type, offset, length, width, height, duration
            FROM media
            LEFT JOIN media_tags ON media.id = media_tags.id
            WHERE media_type = ?1
            AND media_tags.tag_id IN ({})
            GROUP BY id
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
             GROUP BY id
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

    fn get_random_media_with_tags(
        &self,
        tags: &[&str],
    ) -> Result<Option<MediaEntry>> {
        let tag_ids: Vec<_> = tags
            .iter()
            .filter_map(|tag| self.tag_map.get(*tag))
            .collect();

        let sql = format!(
            r#"
            SELECT id, path, media_type, offset, length, width, height, duration
            FROM media
            LEFT JOIN media_tags ON media.id = media_tags.id
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
    pub fn get_random_image(&mut self, tags: Option<&[&str]>) -> Result<Option<DynamicImage>> {
        let media = match tags {
            Some(tags) => self.get_random_media_type_with_tags(MediaType::Image, tags)?,
            None => self.get_random_media_type(MediaType::Image)?,
        };

        Ok(match media {
            Some(x) => Some(x.into_image(self)?),
            None => None,
        })
    }

    /// Get a random video entry
    pub fn get_random_video(&mut self, tags: Option<&[&str]>) -> Result<Option<Video>> {
        let media = match tags {
            Some(tags) => self.get_random_media_type_with_tags(MediaType::Video, tags)?,
            None => self.get_random_media_type(MediaType::Video)?,
        };

        Ok(match media {
            Some(x) => Some(x.into_video()?),
            None => None,
        })
    }

    pub fn get_random_item(&mut self, tags: Option<&[&str]>) -> Result<Option<Media>> {
        let media = match tags {
            Some(tags) => self.get_random_media_with_tags(tags)?,
            None => self.get_random_media()?,
        };

        Ok(match media {
            Some(media) => {
                if media.media_type == MediaType::Image {
                    Some(Media::Image(media.into_image(self)?))
                } else {
                    Some(Media::Video(media.into_video()?))
                }
            },
            None => None,
        })
    }

    /// Get a random entry of any media type
    pub fn get_random_entry(&self) -> Result<Option<MediaEntry>> {
        let all = self.get_all_entries()?;
        Ok(all.choose(&mut rand::rng()).cloned())
    }

    /// Extract file data for a given entry
    pub fn extract_file_data(&mut self, entry: &MediaEntry) -> Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(entry.offset))?;
        let mut buffer = vec![0u8; entry.length as usize];
        self.file.read_exact(&mut buffer)?;
        Ok(buffer)
    }

    pub fn read_image_data(&mut self, entry: &MediaEntry) -> Result<DynamicImage> {
        self.file.seek(SeekFrom::Start(entry.offset))?;

        let mut limited_reader = self.file.try_clone()?.take(entry.length);
        let img = ImageReader::with_format(BufReader::new(&mut limited_reader), ImageFormat::Avif)
            .decode()?;

        Ok(img)
    }

    /// Extract file data and write to a path
    pub fn extract_file_to_path(&mut self, entry: &MediaEntry, output_path: &Path) -> Result<()> {
        let data = self.extract_file_data(entry)?;
        std::fs::write(output_path, data)?;
        Ok(())
    }

    /// Get entry by path
    pub fn get_entry_by_path(&self, path: &str) -> Result<Option<MediaEntry>> {
        let mut stmt = self.db.prepare(
            "SELECT id, path, media_type, offset, length, width, height, duration
             FROM media m
             WHERE path = ?1
             GROUP BY id",
        )?;

        let mut rows = stmt.query_map(params![path], |row| {
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
        })?;

        match rows.next() {
            Some(entry) => Ok(Some(entry?)),
            None => Ok(None),
        }
    }

    pub fn write_to_temp_file(&mut self, video: &Video) -> Result<NamedTempFile> {
        let mut tempfile = NamedTempFile::with_suffix(".webm")?;

        self.file.seek(SeekFrom::Start(video.offset))?;

        let mut buffer = vec![0u8; video.length as usize];
        self.file.read_exact(&mut buffer)?;

        tempfile.write_all(&mut buffer)?;

        Ok(tempfile)
    }
}

fn repeat_vars(count: usize) -> String {
    assert_ne!(count, 0);
    let mut s = "?,".repeat(count);
    // Remove trailing comma
    s.pop();
    s
}
