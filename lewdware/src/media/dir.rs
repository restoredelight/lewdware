use std::path::{Path};

use anyhow::Result;
use image::ImageReader;
use rand::{random_range, rng, seq::IndexedRandom};
use walkdir::WalkDir;

use ffmpeg_next as ffmpeg;

use crate::media::process::{process_path, Audio, Media, MediaType, Processed};
use crate::media::types::ImageData;
use crate::media::{self, Image, Video, types::FileOrPath};

pub struct MediaDir {
    media: Vec<Media>,
    audio: Vec<Audio>,
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
                Ok(Processed::Media(item)) => {
                    if matches!(item.media_type, MediaType::Image { .. }) {
                        image_count += 1;
                    } else {
                        video_count += 1;
                    }

                    media.push(item);
                }
                Ok(Processed::Audio(path)) => {
                    audio.push(path);
                }
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

        let (path, width, height, transparent) = match self
            .media
            .iter()
            .filter_map(|x| match x.media_type {
                MediaType::Image { width, height, transparent } => Some((&x.path, width, height, transparent)),
                _ => None
            })
            .nth(index)
        {
            Some(x) => x,
            None => return Ok(None),
        };

        Ok(Some(Image {
            width,
            height,
            data: self.read_image_data(path)?,
            transparent,
        }))
    }

    pub fn read_image_data(&self, path: &Path) -> Result<ImageData> {
        Ok(ImageReader::open(path)?.decode()?.into_rgba8())
    }

    // pub fn get_random_item(&self) -> Result<Option<media::Media>> {
    //     let item = match self.media.choose(&mut rng()) {
    //         Some(x) => x,
    //         None => return Ok(None),
    //     };
    //
    //     match item.media_type {
    //         MediaType::Image { width, height, transparent, } => Ok(Some(media::Media::Image(Image { width, height, transparent, data: self.read_image_data(&item.path)? }))),
    //         MediaType::Video { width, height, audio, duration } => Ok(Some(media::Media::Video(Video {
    //             width,
    //             height,
    //             audio,
    //             duration,
    //             file: FileOrPath::Path(item.path.clone()),
    //         }))),
    //     }
    // }

    pub fn get_random_audio(&self) -> Result<Option<media::Audio>> {
        let item = match self.audio.choose(&mut rng()) {
            Some(x) => x,
            None => return Ok(None),
        };

        Ok(Some(media::Audio {
            duration: item.duration,
            file: FileOrPath::Path(item.path.clone()),
        }))
    }
}
