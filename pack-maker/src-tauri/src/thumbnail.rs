use std::io::{Read, Write};

use anyhow::{bail, Result};
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tempfile::NamedTempFile;

pub async fn generate_thumbnail(
    app_handle: AppHandle,
    data: Vec<u8>,
    is_image: bool,
    large: bool,
) -> Result<Vec<u8>> {
    let (width, height) = if large { (300, 200) } else { (150, 100) };

    let mut output_file = NamedTempFile::with_suffix(".png")?;

    let mut input_file = NamedTempFile::with_suffix(if is_image { ".avif" } else { ".mp4" })?;

    input_file.write_all(&data)?;

    let scale_filter = format!(
        "scale='iw*min(1,if(gt(iw/{w},ih/{h}),{w}/iw,{h}/ih))':'ih*min(1,if(gt(iw/{w},ih/{h}),{w}/iw,{h}/ih))'",
        w = width,
        h = height
    );

    let mut command = app_handle.shell().sidecar("ffmpeg")?;

    command = command
        .arg("-i")
        .arg(input_file.path())
        .args(["-vf", &scale_filter, "-y"]);

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
