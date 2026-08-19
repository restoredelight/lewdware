use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

use crate::media::bounded_input::{BoundedInput, open_bounded};

pub type ImageData = image::ImageBuffer<image::Rgba<u8>, Vec<u8>>;

#[derive(Debug)]
pub struct VideoData {
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

/// A media file extracted out of a pack into a temp file. The file is deleted when this is
/// dropped, so anything handed the path has to finish reading before then.
#[derive(Debug)]
pub struct ExtractedFile(pub NamedTempFile);

impl ExtractedFile {
    pub fn path(&self) -> &Path {
        self.0.path()
    }
}
