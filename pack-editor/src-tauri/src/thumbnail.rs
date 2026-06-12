use std::io::Write;

use anyhow::{bail, Result};
use tempfile::NamedTempFile;
use tokio::process::Command;

use crate::pack::FileData;

pub async fn generate_display_image(file_data: FileData) -> Result<Vec<u8>> {
    let mut _temp_file = None;

    let path = match file_data {
        FileData::Path(path) => path,
        FileData::Data(data) => {
            let mut tempfile = NamedTempFile::with_suffix(".avif")?;
            tempfile.write_all(&data)?;
            let path = tempfile.path().to_path_buf();
            _temp_file = Some(tempfile);
            path
        }
    };

    #[allow(unused_mut)]
    let mut std_cmd = std::process::Command::new(crate::encode::get_ffmpeg_path());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        std_cmd.creation_flags(0x08000000);
    }
    shared::utils::sanitize_child_env(&mut std_cmd);
    let mut cmd = Command::from(std_cmd);

    cmd.args(["-y", "-i"]).arg(&path).args([
        "-vf",
        "scale='min(iw,2560)':'min(ih,1440)':force_original_aspect_ratio=decrease",
        "-pix_fmt",
        "yuv420p",
        "-f",
        "mjpeg",
        "-q:v",
        "2",
        "pipe:1",
    ]);

    let output = cmd.output().await?;

    if !output.status.success() {
        bail!("ffmpeg display image generation failed");
    }

    Ok(output.stdout)
}

pub async fn generate_preview(file_data: FileData, is_image: bool) -> Result<Vec<u8>> {
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
    let mut std_cmd = std::process::Command::new(crate::encode::get_ffmpeg_path());
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

    cmd.args([
        "-vf",
        "scale='min(iw,300)':'min(ih,200)':force_original_aspect_ratio=decrease",
        "-pix_fmt",
        "yuv420p",
        "-f",
        "mjpeg",
        "-q:v",
        "4",
        "pipe:1",
    ]);

    let output = cmd.output().await?;

    if !output.status.success() {
        bail!("ffmpeg preview generation failed");
    }

    Ok(output.stdout)
}
