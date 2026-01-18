use std::{io::{Read, Write}};

use anyhow::{bail, Result};
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tempfile::NamedTempFile;

use crate::pack::FileData;

pub async fn generate_thumbnail(
    app_handle: AppHandle,
    file_data: FileData,
    is_image: bool,
    large: bool,
) -> Result<Vec<u8>> {
    let (width, height) = if large { (300, 200) } else { (150, 100) };

    let mut output_file = NamedTempFile::with_suffix(".png")?;

    let (_tempfile, input_path) = match file_data {
        FileData::Path(path_buf) => {
            (None, path_buf)
        },
        FileData::Data(data) => {
            let mut file = NamedTempFile::with_suffix(if is_image { ".avif" } else { ".webm" })?;

            let path = file.path().to_path_buf();

            file.write_all(&data)?;

            (Some(file), path)
        },
    };

    let scale_filter = format!(
        "scale='min({w},iw)':'min({h},ih)':force_original_aspect_ratio=decrease",
        w = width,
        h = height
    );

    let mut command = app_handle.shell().sidecar("ffmpeg")?;

    command = command
        .arg("-i")
        .arg(input_path)
        .args(["-vf", &scale_filter, "-v", "error", "-y"]);

    if !is_image {
        command = command.args(["-ss", "1", "-frames:v", "1", "-vf", &scale_filter]);
    }

    command = command.arg(output_file.path());

    let output = command.output().await?;

    if !output.status.success() {
        eprintln!("{:?}", String::from_utf8_lossy(&output.stderr));
        bail!("ffmpeg command failed");
    }

    let mut buf = Vec::new();
    output_file.read_to_end(&mut buf)?;

    Ok(buf)
}
