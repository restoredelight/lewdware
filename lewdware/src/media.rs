use anyhow::{Result, anyhow};
use async_channel::{Receiver, Sender, bounded};
use async_executor::LocalExecutor;
use async_fs::File;
use futures_lite::future::block_on;
use futures_lite::{AsyncReadExt, AsyncSeekExt};
use image::{ImageFormat, ImageReader};
use pack_format::config::Metadata;
use pack_format::{HEADER_SIZE, Header};
use rand::prelude::IndexedRandom;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::{fs, thread};
use tempfile::NamedTempFile;
use winit::event_loop::EventLoop;

use crate::app::UserEvent;

pub fn spawn_media_manager_thread(
    event_loop: &EventLoop<UserEvent>,
) -> Result<(Sender<MediaRequest>, Receiver<MediaResponse>, Metadata)> {
    let (req_tx, req_rx) = bounded(20);
    let (resp_tx, resp_rx) = bounded(20);

    let event_loop_proxy = event_loop.create_proxy();

    let manager = MediaManager::open("pack.md")?;
    let metadata = manager.metadata().clone();

    thread::spawn(move || {
        let executor = Rc::new(LocalExecutor::new());
        let ex = executor.clone();

        block_on(executor.run(async move {
            let manager = Rc::new(manager);

            while let Ok(request) = req_rx.recv().await {
                let resp_tx = resp_tx.clone();
                let manager = manager.clone();

                let event_loop_proxy = event_loop_proxy.clone();

                ex.spawn(async move {
                    let response = manager.handle_request(request).await.unwrap();

                    if let Some(response) = response {
                        match resp_tx.send(response).await {
                            Ok(_) => {
                                if let Err(err) =
                                    event_loop_proxy.send_event(UserEvent::MediaResponse)
                                {
                                    eprintln!("{}", err);
                                };
                            }
                            Err(err) => {
                                eprintln!("{}", err);
                            }
                        }
                    }
                })
                .detach();
            }
        }));
    });

    Ok((req_tx, resp_rx, metadata))
}

#[derive(Debug)]
pub enum MediaRequest {
    RandomMedia {
        only_images: bool,
        tags: Option<Vec<String>>,
    },
    RandomAudio {
        tags: Option<Vec<String>>,
    },
    RandomNotification {
        tags: Option<Vec<String>>,
    },
    RandomPrompt {
        tags: Option<Vec<String>>,
    },
    RandomLink {
        tags: Option<Vec<String>>,
    },
}

pub enum MediaResponse {
    Media(Media),
    Audio(Audio),
    Notification(Notification),
    Prompt(Prompt),
    Link(Link),
}

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
    pub async fn into_image(self, media_manager: &MediaManager) -> Result<Image> {
        assert_eq!(self.media_type, MediaType::Image);

        media_manager.read_image_data(&self).await
    }

    pub async fn into_video(self, media_manager: &MediaManager) -> Result<Video> {
        assert_eq!(self.media_type, MediaType::Video);

        let tempfile = media_manager
            .write_to_temp_file(self.offset, self.length)
            .await?;

        Ok(Video {
            width: self.width.unwrap(),
            height: self.height.unwrap(),
            tempfile,
        })
    }

    pub async fn into_audio(self, media_manager: &MediaManager) -> Result<Audio> {
        assert_eq!(self.media_type, MediaType::Audio);

        let tempfile = media_manager
            .write_to_temp_file(self.offset, self.length)
            .await?;

        Ok(Audio { tempfile })
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

pub struct MediaManager {
    path: PathBuf,
    db: Connection,
    header: Header,
    metadata: Metadata,
    temp_file: NamedTempFile,
    tag_map: HashMap<String, u64>,
}

impl MediaManager {
    /// Open a media pack file for reading
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut file = fs::File::open(&path)?;

        // Read and validate header
        let header = Header::read_from(&file)?;

        file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;

        println!("{}", header.metadata_length);
        let mut buf = vec![0u8; header.metadata_length as usize];
        file.read_exact(&mut buf)?;
        println!("{}", String::from_utf8_lossy(&buf));
        let metadata = Metadata::from_buf(&buf)?;
        // let metadata = Metadata::default();

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

        Ok(MediaManager {
            path,
            db,
            header,
            metadata,
            temp_file,
            tag_map,
        })
    }

    pub async fn handle_request(&self, request: MediaRequest) -> Result<Option<MediaResponse>> {
        match request {
            MediaRequest::RandomMedia { only_images, tags } => {
                if only_images {
                    self.get_random_image(tags)
                        .await
                        .map(|x| x.map(|image| MediaResponse::Media(Media::Image(image))))
                } else {
                    self.get_random_item(tags)
                        .await
                        .map(|x| x.map(MediaResponse::Media))
                }
            }
            MediaRequest::RandomAudio { tags } => self
                .get_random_audio(tags)
                .await
                .map(|x| x.map(MediaResponse::Audio)),
            MediaRequest::RandomNotification { tags } => self
                .get_random_notification(tags)
                .map(|x| x.map(MediaResponse::Notification)),
            MediaRequest::RandomPrompt { tags } => self
                .get_random_prompt(tags)
                .map(|x| x.map(MediaResponse::Prompt)),
            MediaRequest::RandomLink { tags } => self
                .get_random_link(tags)
                .map(|x| x.map(MediaResponse::Link)),
        }
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

    pub async fn get_random_item(&self, tags: Option<Vec<String>>) -> Result<Option<Media>> {
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

    /// Get a random entry of any media type
    pub fn get_random_entry(&self) -> Result<Option<MediaEntry>> {
        let all = self.get_all_entries()?;
        Ok(all.choose(&mut rand::rng()).cloned())
    }

    /// Extract file data for a given entry
    pub async fn extract_file_data(&self, entry: &MediaEntry) -> Result<Vec<u8>> {
        let mut file = File::open(&self.path).await?;
        file.seek(SeekFrom::Start(entry.offset)).await?;
        let mut buffer = vec![0u8; entry.length as usize];
        file.read_exact(&mut buffer).await?;
        Ok(buffer)
    }

    pub async fn read_image_data(&self, entry: &MediaEntry) -> Result<Image> {
        let mut buffer = vec![0u8; entry.length as usize];

        let mut file = File::open(&self.path).await?;
        file.seek(SeekFrom::Start(entry.offset)).await?;

        file.read_exact(&mut buffer).await?;

        let mut reader = ImageReader::new(Cursor::new(buffer));

        reader.set_format(ImageFormat::Avif);

        let image = reader.decode()?;

        Ok(image.into_rgba8())

        // libavif_image::read(&buffer).map_err(|e| anyhow!(e))
    }

    /// Extract file data and write to a path
    pub async fn extract_file_to_path(&self, entry: &MediaEntry, output_path: &Path) -> Result<()> {
        let data = self.extract_file_data(entry).await?;
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

    async fn write_to_temp_file(&self, offset: u64, length: u64) -> Result<NamedTempFile> {
        println!("Writing to tempfile");
        let mut tempfile = NamedTempFile::with_suffix(".webm")?;
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

fn repeat_vars(count: usize) -> String {
    assert_ne!(count, 0);
    let mut s = "?,".repeat(count);
    // Remove trailing comma
    s.pop();
    s
}
