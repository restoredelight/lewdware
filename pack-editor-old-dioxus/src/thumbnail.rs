use std::{
    io::Write
};

use anyhow::{bail, Result};
use tempfile::NamedTempFile;
use tokio::process::Command;

use crate::pack::FileData;

pub async fn generate_preview(file_data: FileData, is_image: bool) -> Result<Vec<u8>> {
    let mut _temp_file = None;
    let path = match file_data {
        FileData::Path(path) => path,
        FileData::Data(data) => {
            println!("Data length: {}", data.len());
            let mut tempfile =
                NamedTempFile::with_suffix(if is_image { ".avif" } else { ".webm" })?;
            tempfile.write_all(&data)?;

            let path = tempfile.path().to_path_buf();
            _temp_file = Some(tempfile);

            path
        }
    };

    let mut command = Command::new("ffmpeg");

    command.args([
        // "-v",
        // "error",
        "-y",
    ]);

    if !is_image {
        command.args(["-ss", "0"]);
    }

    command.arg("-i").arg(path);

    if !is_image {
        command.args(["-frames:v", "1"]);
    } else {
        // command.args(["-c:v", "libdav1d"]);
    }

    command.args([
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

    let output = command.output().await?;

    if !output.status.success() {
        eprintln!("{:?}", String::from_utf8_lossy(&output.stderr));
        bail!("ffmpeg command failed");
    }

    println!("{}", output.stdout.len());

    Ok(output.stdout)
}
