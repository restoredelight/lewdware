use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

// #[derive(Debug)]
// pub enum Media {
//     Image(Image),
//     Video(Video),
//     Audio(Audio),
// }

#[derive(Debug)]
pub struct Image {
    pub width: u64,
    pub height: u64,
    pub transparent: bool,
    pub data: ImageData,
}

pub type ImageData = image::ImageBuffer<image::Rgba<u8>, Vec<u8>>;

#[derive(Debug)]
pub struct Video {
    pub width: u64,
    pub height: u64,
    pub duration: f64,
    pub audio: bool,
    pub file: FileOrPath,
}

#[derive(Debug)]
pub struct VideoData {
    pub width: u32,
    pub height: u32,
    pub file: FileOrPath,
}

#[derive(Debug)]
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

#[derive(Debug)]
pub struct Audio {
    pub duration: f64,
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
    pub width: u64,
    pub height: u64,
    pub transparent: bool,
    pub file: FileOrPath,
}
