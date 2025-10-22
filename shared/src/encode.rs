use std::{cmp, fs::File, io::Read, path::Path, process::{self, Command}};

use anyhow::{bail, Result};

pub fn is_animated(path: &Path) -> Result<bool> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_frames",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()?;

    if !output.status.success() {
        return Ok(false);
    }

    let frames_str = String::from_utf8_lossy(&output.stdout);

    // If we can't get frame count or it's 1, treat as static
    // If it's > 1 or "N/A" (infinite like GIF), treat as animated
    match frames_str.trim().parse::<i32>() {
        Ok(frames) => Ok(frames > 1),
        Err(_) => Ok(frames_str == "N/A"),
    }
}

pub fn encode_image(input: &Path) -> anyhow::Result<(Vec<u8>, Metadata)> {
    let tmp = tempfile::NamedTempFile::with_suffix(".avif")?;
    let tmp_path = tmp.path();

    #[rustfmt::skip]
    let args = [
        // -i input
        "-y",
        "-vf", "scale='min(1280,iw)':'min(720,ih)':force_original_aspect_ratio=decrease",
        "-c:v", "libaom-av1",
        "-cpu-used", "6",
        "-crf", "32",
        "-b:v", "0",
        "-still-picture", "1",
        "-pix_fmt", "yuv420p10le",
        "-f", "avif",
        // tmp_path
    ];

    let status = Command::new("ffmpeg")
        .arg("-i")
        .arg(input)
        .args(args)
        .arg(tmp_path)
        .stderr(process::Stdio::null())
        .status()?;

    if !status.success() {
        bail!("ffmpeg failed for {}", input.display());
    }

    let metadata = get_metadata(tmp_path)?;

    let mut buf = Vec::new();
    File::open(tmp_path)?.read_to_end(&mut buf)?;
    Ok((buf, metadata))
}

pub fn encode_video(input: &Path, audio: bool) -> anyhow::Result<(Vec<u8>, Metadata)> {
    let tmp = tempfile::NamedTempFile::with_suffix(".mp4")?;
    let tmp_path = tmp.path();

    #[rustfmt::skip]
    let args = [
        "-y",
        "-vf", "scale=w='if(gt(a,1280/720),1280,-1)':h='if(gt(a,1280/720),-1,720)':force_original_aspect_ratio=decrease, scale='trunc(iw/2)*2':'trunc(ih/2)*2'",
        "-c:v", "libx264",
        "-f", "mp4",
    ];

    let mut command = Command::new("ffmpeg");
    command.arg("-i").arg(input).args(args);

    if audio {
        command.args(["-c:a", "libopus", "-b:a", "64k"]);
    } else {
        command.arg("-an");
    }

    let result = command
        .arg(tmp_path)
        .output()?;

    if !result.status.success() {
        eprintln!("{:?}", String::from_utf8_lossy(&result.stderr));
        bail!("ffmpeg failed for {}", input.display());
    }

    let metadata = get_metadata(tmp_path)?;

    let mut buf = Vec::new();
    File::open(tmp_path)?.read_to_end(&mut buf)?;
    Ok((buf, metadata))
}

pub fn encode_audio(input: &Path) -> anyhow::Result<Vec<u8>> {
    let tmp = tempfile::NamedTempFile::with_suffix(".opus")?;
    let tmp_path = tmp.path();

    #[rustfmt::skip]
    let args = [
        // -i input
        "-y",
        "-c:a", "libopus",
        "-b:a", "64k",
        // tmp_path
    ];

    let status = Command::new("ffmpeg")
        .arg("-i")
        .arg(input)
        .args(args)
        .arg(tmp_path)
        .stderr(process::Stdio::null())
        .status()?;

    if !status.success() {
        bail!("ffmpeg failed for {}", input.display());
    }

    let mut buf = Vec::new();
    File::open(tmp_path)?.read_to_end(&mut buf)?;
    Ok(buf)
}

#[derive(Default)]
pub struct Metadata {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<f64>,
}

pub fn get_metadata(path: &Path) -> anyhow::Result<Metadata> {
    #[rustfmt::skip]
    let args = [
        "-v", "quiet",
        "-print_format", "json",
        "-select_streams", "v:0",
        "-show_streams",
    ];

    let output = Command::new("ffprobe").args(args).arg(path).output()?;

    if !output.status.success() {
        bail!("ffprobe failed with status: {}", output.status);
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let streams = match json["streams"].as_array() {
        Some(x) => x,
        None => bail!("Invalid json response from ffprobe"),
    };

    if let Some(stream) = streams.first() {
        let width = stream["width"].as_i64().map(|x| cmp::min(x, 1280));
        let height = stream["height"].as_i64().map(|x| cmp::min(x, 720));
        let duration = stream["duration"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok());

        return Ok(Metadata {
            width,
            height,
            duration,
        });
    }

    bail!("Invalid json response from ffprobe")
}
