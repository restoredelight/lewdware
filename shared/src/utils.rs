use std::{ffi::OsStr, path::Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileType {
    Image,
    Video,
    Audio,
    Other
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

