use std::path::{Path, PathBuf};

use anyhow::Result;
use image::ImageReader;
use shared::utils::{FileType, classify_ext};
use rand::{random_range, rng, seq::IndexedRandom};
use walkdir::WalkDir;

use ffmpeg_next as ffmpeg;

use crate::media::process::{process_path, Media, MediaType, Processed};
use crate::media::{self, Audio, Image, Video, types::FileOrPath};

pub struct MediaDir {
    media: Vec<Media>,
    audio: Vec<PathBuf>,
    image_count: usize,
    video_count: usize,
}

impl MediaDir {
    pub fn open(dir: &Path) -> Result<Self> {
        ffmpeg::init()?;

        let mut media = Vec::new();
        let mut audio = Vec::new();

        let mut image_count = 0;
        let mut video_count = 0;

        for entry in WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
        {
            let path = entry.path();

            match process_path(path) {
                Ok(Some(Processed::Media(item))) => {
                    if item.media_type == MediaType::Image {
                        image_count += 1;
                    } else {
                        video_count += 1;
                    }

                    media.push(item);
                }
                Ok(Some(Processed::Audio(path))) => {
                    audio.push(path);
                }
                Ok(None) => {}
                Err(err) => {
                    eprintln!("Error processing {}: {}", path.display(), err);
                }
            }
        }

        Ok(Self {
            media,
            audio,
            image_count,
            video_count,
        })
    }

    pub fn get_random_image(&self) -> Result<Option<Image>> {
        let index = random_range(0..self.image_count);

        let item = match self
            .media
            .iter()
            .filter(|x| x.media_type == MediaType::Image)
            .nth(index)
        {
            Some(x) => x,
            None => return Ok(None),
        };

        Ok(Some(self.read_image(&item.path)?))
    }

    pub fn read_image(&self, path: &Path) -> Result<Image> {
        Ok(ImageReader::open(path)?.decode()?.into_rgba8())
    }

    pub fn get_random_item(&self) -> Result<Option<media::Media>> {
        let item = match self.media.choose(&mut rng()) {
            Some(x) => x,
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

    pub fn get_random_audio(&self) -> Result<Option<Audio>> {
        let item = match self.audio.choose(&mut rng()) {
            Some(x) => x,
            None => return Ok(None),
        };

        Ok(Some(Audio {
            file: FileOrPath::Path(item.clone()),
        }))
    }
}
