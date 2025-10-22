use std::path::{Path, PathBuf};

use anyhow::Result;
use ffmpeg_next as ffmpeg;
use image::ImageReader;
use shared::read_config::{Config, MediaCategory, find_config};
use rand::{rng, seq::IndexedRandom};
use walkdir::WalkDir;

use crate::media::{
    self, Audio, Image, Link, Notification, Prompt, Video,
    process::{Media, MediaType, Processed, process_path},
    types::{FileOrPath, Wallpaper},
};

pub struct DevPack {
    root: PathBuf,
    config: Config,
    media: Vec<Item<Media>>,
    audio: Vec<Item<PathBuf>>,
    wallpapers: Vec<Item<PathBuf>>,
    notifications: Vec<Item<Notification>>,
    links: Vec<Item<Link>>,
    prompts: Vec<Item<Prompt>>,
}

struct Item<T> {
    tags: Vec<String>,
    item: T,
}

impl DevPack {
    pub fn open(path: &Path) -> Result<Self> {
        ffmpeg::init()?;

        let config = find_config(path)?;

        let resolved = config.resolve();

        let mut media = Vec::new();
        let mut audio = Vec::new();
        let mut wallpapers = Vec::new();

        for entry in WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
        {
            let path = entry.path();

            match process_path(path) {
                Ok(Some(Processed::Media(item))) => {
                    let (tags, category) = config.get_tags_and_category(path, &resolved);

                    match category {
                        MediaCategory::Default => media.push(Item { tags, item }),
                        MediaCategory::Wallpaper => wallpapers.push(Item {
                            tags,
                            item: item.path,
                        }),
                    }
                }
                Ok(Some(Processed::Audio(path))) => {
                    let (tags, _) = config.get_tags_and_category(&path, &resolved);

                    audio.push(Item { tags, item: path });
                }
                Ok(None) => {}
                Err(err) => {
                    eprintln!("Error processing {}: {}", path.display(), err);
                }
            }
        }

        let mut notifications = Vec::new();

        for notification in &resolved.notifications {
            notifications.push(Item {
                tags: notification.tags.clone(),
                item: Notification {
                    summary: notification.opts.summary.clone(),
                    body: notification.primary.clone(),
                },
            });
        }

        let mut links = Vec::new();

        for link in &resolved.links {
            links.push(Item {
                tags: link.tags.clone(),
                item: Link {
                    link: link.primary.clone(),
                },
            });
        }

        let mut prompts = Vec::new();

        for prompt in &resolved.prompts {
            prompts.push(Item {
                tags: prompt.tags.clone(),
                item: Prompt {
                    prompt: prompt.primary.clone(),
                },
            });
        }

        Ok(Self {
            root: path.to_path_buf(),
            config,
            media,
            audio,
            wallpapers,
            notifications,
            links,
            prompts,
        })
    }

    pub fn get_random_idea(&self, tags: Option<Vec<String>>) -> Result<Option<Image>> {
        let items: Vec<_> = self
            .media
            .iter()
            .filter(|media| {
                media.item.media_type == MediaType::Image
                    && tags
                        .as_ref()
                        .is_none_or(|tags| media.tags.iter().any(|tag| tags.contains(tag)))
            })
            .collect();

        let item = match items.choose(&mut rng()) {
            Some(x) => &x.item,
            None => return Ok(None),
        };

        Ok(Some(self.read_image(&item.path)?))
    }

    pub fn get_random_media(&self, tags: Option<Vec<String>>) -> Result<Option<media::Media>> {
        let item = match choose_item(&self.media, tags) {
            Some(x) => &x.item,
            None => return Ok(None),
        };

        match item.media_type {
            MediaType::Image => Ok(Some(media::Media::Image(self.read_image(&item.path)?))),
            MediaType::Video { width, height } => Ok(Some(media::Media::Video(Video {
                width: width as i64,
                height: height as i64,
                file: FileOrPath::Path(item.path.clone()),
            }))),
        }
    }

    pub fn read_image(&self, path: &Path) -> Result<Image> {
        Ok(ImageReader::open(path)?.decode()?.into_rgba8())
    }

    pub fn get_random_audio(&self, tags: Option<Vec<String>>) -> Result<Option<Audio>> {
        let item = match choose_item(&self.audio, tags) {
            Some(x) => &x.item,
            None => return Ok(None),
        };

        Ok(Some(Audio {
            file: FileOrPath::Path(item.clone()),
        }))
    }

    pub fn get_random_wallpaper(&self, tags: Option<Vec<String>>) -> Result<Option<Wallpaper>> {
        let item = match choose_item(&self.wallpapers, tags) {
            Some(x) => &x.item,
            None => return Ok(None),
        };

        Ok(Some(Wallpaper {
            file: FileOrPath::Path(item.clone()),
        }))
    }

    pub fn get_random_notification(&self, tags: Option<Vec<String>>) -> Option<Notification> {
        choose_item(&self.notifications, tags).map(|x| x.item.clone())
    }

    pub fn get_random_link(&self, tags: Option<Vec<String>>) -> Option<Link> {
        choose_item(&self.links, tags).map(|x| x.item.clone())
    }

    pub fn get_random_prompt(&self, tags: Option<Vec<String>>) -> Option<Prompt> {
        choose_item(&self.prompts, tags).map(|x| x.item.clone())
    }
}

fn choose_item<T>(items: &[Item<T>], tags: Option<Vec<String>>) -> Option<&Item<T>> {
    if let Some(tags) = tags {
        let items: Vec<_> = items
            .iter()
            .filter(|media| media.tags.iter().any(|tag| tags.contains(tag)))
            .collect();

        items.choose(&mut rng()).cloned()
    } else {
        items.choose(&mut rng())
    }
}
