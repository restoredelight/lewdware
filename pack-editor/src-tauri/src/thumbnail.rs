use std::io::Write;

use anyhow::{bail, Result};
use tempfile::NamedTempFile;
use tokio::process::Command;

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

    #[allow(unused_mut)]
    let mut std_cmd = std::process::Command::new(shared::encode::get_ffmpeg_path());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        std_cmd.creation_flags(0x08000000);
    }
    shared::utils::sanitize_child_env(&mut std_cmd);
    let mut cmd = Command::from(std_cmd);
    cmd.args(["-y"]);

    if !is_image {
        cmd.args(["-ss", "0"]);
    }

    cmd.arg("-i").arg(&path);

    if !is_image {
        cmd.args(["-frames:v", "1"]);
    }

    // Transparent videos are packed as color-on-top, alpha-as-luma-on-bottom (see
    // `encode_video_with_transparency`); crop to the top half before scaling so the preview
    // shows the actual color frame instead of the raw double-height packed frame. Only videos:
    // a transparent image keeps a real alpha plane (see `encode_image`), so cropping one would
    // throw away the bottom half of the picture.
    // Sized for the inspector preview, the largest consumer: it grows with the inspector, up to
    // 420x262 CSS px, which is twice that on a HiDPI display. `min()` keeps this a cap rather
    // than a target -- a small source is served at its own size instead of being upscaled here.
    let scale = "scale='min(iw,800)':'min(ih,600)':force_original_aspect_ratio=decrease";
    let filter = if transparent && !is_image {
        format!("crop=iw:ih/2:0:0,{scale}")
    } else {
        scale.to_string()
    };

    cmd.args([
        "-vf", &filter, "-pix_fmt", "yuv420p", "-f", "mjpeg", "-q:v", "4", "pipe:1",
    ]);

    let output = cmd.output().await?;

    if !output.status.success() {
        bail!("ffmpeg preview generation failed");
    }

    Ok(output.stdout)
}
