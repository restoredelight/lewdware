use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use ffmpeg_next::{self as ffmpeg};
use shared::encode::FileInfo;

pub struct Media {
    pub media_type: MediaType,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum MediaType {
    Image { width: u64, height: u64, transparent: bool, },
    Video { width: u64, height: u64, duration: f64, audio: bool },
}

pub struct Audio {
    pub path: PathBuf,
    pub duration: f64,
}

pub fn classify_file(path: &Path) -> Result<FileInfo> {
    let ictx = ffmpeg::format::input(&path)?;

    let video_stream = ictx.streams().best(ffmpeg::media::Type::Video);
    let audio_stream = ictx.streams().best(ffmpeg::media::Type::Audio);

    if let Some(video_stream) = video_stream {
        // This seems to be the only way to get the width and height without initializing a
        // decoder. Should be safe because this is essentially what ffmpeg does under the
        // hood anyway.
        let (width, height) = unsafe {
            let parameters_ptr = video_stream.parameters().as_ptr();

            if parameters_ptr.is_null() {
                bail!("Parameters are NULL");
            }

            let parameters = *parameters_ptr;
            (parameters.width, parameters.height)
        };

        if audio_stream.is_some() || video_stream.frames() > 1 {
            Ok(FileInfo::Video {
                width: width as u64,
                height: height as u64,
                duration: video_stream.duration() as f64 * f64::from(video_stream.time_base()),
                audio: audio_stream.is_some(),
            })
        } else {
            Ok(FileInfo::Image {
                width: width as u64,
                height: height as u64,
                // TODO: Look more into this
                transparent: true,
            })
        }
    } else if let Some(audio_stream) = audio_stream {
        Ok(FileInfo::Audio {
            duration: audio_stream.duration() as f64 * f64::from(audio_stream.time_base()),
        })
    } else {
        bail!("No audio or video stream");
    }
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
    Audio(Audio),
}

pub fn process_path(path: &Path) -> Result<Processed> {
    match classify_file(path)? {
        FileInfo::Image { width, height, transparent } => {
            Ok(Processed::Media(Media {
                media_type: MediaType::Image { width, height, transparent },
                path: path.to_path_buf(),
            }))
        }
        FileInfo::Video { width, height, duration, audio } => {
            Ok(Processed::Media(Media {
                media_type: MediaType::Video { width, height, duration, audio },
                path: path.to_path_buf(),
            }))
        }
        FileInfo::Audio { duration } => Ok(Processed::Audio(Audio { duration, path: path.to_path_buf() })),
    }
}
