use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

// #[derive(Debug)]
// pub enum Media {
//     Image(Image),
//     Video(Video),
//     Audio(Audio),
// }

#[derive(Debug)]
#[allow(unused)]
pub struct Image {
    pub width: u64,
    pub height: u64,
    pub transparent: bool,
    pub data: ImageData,
}

pub type ImageData = image::ImageBuffer<image::Rgba<u8>, Vec<u8>>;

#[derive(Debug)]
#[allow(unused)]
pub struct VideoData {
    pub width: u32,
    pub height: u32,
    pub transparent: bool,
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

#[allow(unused)]
#[derive(Debug)]
pub struct Audio {
    pub duration: f64,
    pub file: FileOrPath,
}
