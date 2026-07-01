use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

use crate::media::bounded_input::{BoundedInput, open_bounded};

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
    pub source: MediaSource,
}

/// Points at a video/audio clip stored inside a pack file, as a byte range rather than a
/// standalone file. `open()` hands this straight to ffmpeg via a custom `AVIOContext` bound to
/// that range (see [`crate::media::bounded_input`]), so playing a clip never requires copying it
/// out to a temp file first.
#[derive(Debug, Clone)]
pub struct MediaSource {
    pub path: PathBuf,
    pub offset: u64,
    pub length: u64,
}

impl MediaSource {
    pub fn open(&self) -> anyhow::Result<BoundedInput> {
        open_bounded(&self.path, self.offset, self.length)
    }
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
