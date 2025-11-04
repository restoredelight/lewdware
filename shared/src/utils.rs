use std::{error::Error, ffi::OsStr, fmt::Display, path::Path, str::FromStr};

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FileType {
    #[serde(rename = "image")]
    Image,
    #[serde(rename = "video")]
    Video,
    #[serde(rename = "audio")]
    Audio,
    Other,
}

#[derive(Debug)]
pub struct FromStrError();

impl Display for FromStrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Invalid file type value")
    }
}

impl Error for FromStrError {}

impl FromStr for FileType {
    type Err = FromStrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "image" => Ok(FileType::Image),
            "video" => Ok(FileType::Video),
            "audio" => Ok(FileType::Audio),
            "other" => Ok(FileType::Other),
            _ => Err(FromStrError()),
        }
    }
}

impl FileType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileType::Image => "image",
            FileType::Video => "video",
            FileType::Audio => "audio",
            FileType::Other => "other",
        }
    }
}

pub fn classify_ext(path: &Path) -> FileType {
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_lowercase();

    match &*ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "bmp" | "tiff" => FileType::Image,
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "m4v" => FileType::Video,
        "mp3" | "wav" | "flac" | "ogg" | "opus" | "m4a" => FileType::Audio,
        _ => FileType::Other,
    }
}
