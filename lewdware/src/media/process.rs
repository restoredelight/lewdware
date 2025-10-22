use std::path::{Path, PathBuf};

use anyhow::Result;
use ffmpeg_next as ffmpeg;

use shared::utils::{classify_ext, FileType};

pub struct Media {
    pub media_type: MediaType,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video { width: u32, height: u32 },
}

pub fn is_animated(path: &Path) -> Result<bool> {
    let ictx = ffmpeg::format::input(&path)?;

    let video_stream = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or(ffmpeg::Error::StreamNotFound)?;

    Ok(video_stream.frames() > 1)
}

pub fn video_size(path: &Path) -> Result<(u32, u32)> {
    let ictx = ffmpeg::format::input(&path)?;

    let video_stream = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or(ffmpeg::Error::StreamNotFound)?;

    let parameters = video_stream.parameters();

    let context = ffmpeg::codec::context::Context::from_parameters(parameters)?;
    let decoder = context.decoder().video()?;

    Ok((decoder.width(), decoder.height()))
}


pub enum Processed {
    Media(Media),
    Audio(PathBuf),
}

pub fn process_path(path: &Path) -> Result<Option<Processed>> {
    match classify_ext(path) {
        FileType::Image => {
            if is_animated(path)? {
                let (width, height) = video_size(path)?;

                Ok(Some(Processed::Media(Media {
                    media_type: MediaType::Video { width, height },
                    path: path.to_path_buf(),
                })))
            } else {
                Ok(Some(Processed::Media(Media {
                    media_type: MediaType::Image,
                    path: path.to_path_buf(),
                })))
            }
        }
        FileType::Video => {
            let (width, height) = video_size(path)?;

            Ok(Some(Processed::Media(Media {
                media_type: MediaType::Video { width, height },
                path: path.to_path_buf(),
            })))
        }
        FileType::Audio => Ok(Some(Processed::Audio(path.to_path_buf()))),
        FileType::Other => Ok(None),
    }
}
