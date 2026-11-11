use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::pack::FileData;

pub async fn generate_preview(
    file_data: FileData,
    is_image: bool,
    transparent: bool,
) -> Result<Vec<u8>> {
    let mut _temp_file = None;

    let path = match file_data {
        FileData::Path(path) => path,
        FileData::Data(data) => {
            let mut tempfile = NamedTempFile::with_suffix(if is_image { ".avif" } else { ".mp4" })?;
            tempfile.write_all(&data)?;
            let path = tempfile.path().to_path_buf();
            _temp_file = Some(tempfile);
            path
        }
    };

    // Transparent videos are packed as color-on-top, alpha-as-luma-on-bottom, so only the top
    // half is the picture. Only videos: a transparent image keeps a real alpha plane, so
    // cropping one would throw away the bottom half of the picture.
    let packed_alpha = transparent && !is_image;
    tokio::task::spawn_blocking(move || {
        let _temp_file = _temp_file;
        shared::encode::preview_jpeg(&path, packed_alpha)
    })
    .await?
}
