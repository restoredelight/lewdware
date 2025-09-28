use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

pub enum Media {
    Image(Image),
    Video(Video),
}

pub type Image = image::ImageBuffer<image::Rgba<u8>, Vec<u8>>;

pub struct Video {
    pub width: i64,
    pub height: i64,
    pub file: FileOrPath,
}

pub enum FileOrPath {
    File(NamedTempFile),
    Path(PathBuf),
}

impl FileOrPath {
    pub fn path(&self) -> &Path {
        match self {
            FileOrPath::File(file) => file.path(),
            FileOrPath::Path(path_buf) => &path_buf,
        }
    }
}

pub struct Audio {
    pub file: FileOrPath,
}

#[derive(Clone)]
pub struct Notification {
    pub summary: Option<String>,
    pub body: String,
}

#[derive(Clone)]
pub struct Link {
    pub link: String,
}

#[derive(Clone)]
pub struct Prompt {
    pub prompt: String,
}

pub struct Wallpaper {
    pub file: FileOrPath,
}
