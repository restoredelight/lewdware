use anyhow::{Result, bail};
use byteorder::{LittleEndian, WriteBytesExt};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{cmp, process};
use tempfile::NamedTempFile;
use walkdir::WalkDir;

const MAGIC: &[u8; 5] = b"MPACK";
const VERSION: u8 = 1;
const HEADER_SIZE: usize = 32;

#[derive(Debug, Clone)]
struct Header {
    index_offset: u64,
    total_files: u32,
}

impl Header {
    fn write_to<W: Write + Seek>(&self, mut w: W) -> Result<()> {
        w.seek(SeekFrom::Start(0))?;
        w.write_all(MAGIC)?;
        w.write_all(&[VERSION])?;
        w.write_u16::<LittleEndian>(0)?;
        w.write_u64::<LittleEndian>(self.index_offset)?;
        w.write_u32::<LittleEndian>(self.total_files)?;
        w.write_all(&[0u8; 12])?;
        Ok(())
    }
}

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Pack a media directory into a single .mp file with embedded SQLite index"
)]
struct Cli {
    input: PathBuf,
    output: PathBuf,
    #[arg(long, default_value = "500")]
    chunk_size: usize,
}

#[derive(Debug, Clone, Copy)]
enum MediaType {
    Image,
    Video,
    Audio,
    Other,
}

impl MediaType {
    fn as_str(&self) -> &'static str {
        match self {
            MediaType::Image => "image",
            MediaType::Video => "video",
            MediaType::Audio => "audio",
            MediaType::Other => "other",
        }
    }
}

#[derive(Debug, Clone)]
struct PackedEntry {
    rel_path: String,
    media_type: MediaType,
    offset: u64,
    length: u64,
    width: Option<i64>,
    height: Option<i64>,
    duration: Option<f64>,
    tags: Vec<String>,
}

#[derive(Debug)]
struct ProcessedFile {
    entry: PackedEntry,
    data: Vec<u8>,
}

fn main() -> Result<()> {
    let args = Cli::parse();
    anyhow::ensure!(args.input.is_dir(), "input must be a directory");
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)?;
    }

    let tags_path = args.input.join("tags.json");
    let tags_map: HashMap<String, Vec<String>> = match fs::read_to_string(tags_path) {
        Ok(content) => serde_json::from_str(&content)?,
        Err(err) => match err.kind() {
            ErrorKind::NotFound => HashMap::new(),
            _ => {
                anyhow::bail!(err);
            }
        },
    };

    let mut out = OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .truncate(true)
        .open(&args.output)?;

    Header {
        index_offset: 0,
        total_files: 0,
    }
    .write_to(&mut out)?;

    out.seek(SeekFrom::Start(HEADER_SIZE as u64))?;

    let files: Vec<_> = WalkDir::new(&args.input)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}",
        )
        .unwrap(),
    );

    let mut all_entries = Vec::new();
    let mut current_offset = HEADER_SIZE as u64;

    for chunk in files.chunks(args.chunk_size) {
        pb.set_message("Processing chunk...");

        let processed_files: Vec<ProcessedFile> = chunk
            .par_iter()
            .filter_map(|entry| {
                match process_single_file(entry, &args.input, &tags_map) {
                    Ok(Some(processed)) => {
                        pb.inc(1);
                        Some(processed)
                    }
                    Ok(None) => {
                        pb.inc(1);
                        None // Skipped file
                    }
                    Err(e) => {
                        eprintln!("Error processing {}: {}", entry.path().display(), e);
                        pb.inc(1);
                        None
                    }
                }
            })
            .collect();

        pb.set_message("Writing chunk to disk...");

        // Write chunk data sequentially
        for mut processed in processed_files {
            processed.entry.offset = current_offset;
            processed.entry.length = processed.data.len() as u64;

            out.write_all(&processed.data)?;
            current_offset += processed.data.len() as u64;

            all_entries.push(processed.entry);
        }
    }

    pb.finish_with_message("Encoding complete");

    let tmp_db = NamedTempFile::new()?;
    build_sqlite_index(tmp_db.path(), &all_entries)?;

    let index_offset = out.stream_position()?;
    {
        let mut dbf = File::open(tmp_db.path())?;
        std::io::copy(&mut dbf, &mut out)?;
    }

    let header = Header {
        index_offset,
        total_files: all_entries.len() as u32,
    };
    header.write_to(&mut out)?;

    println!(
        "✅ Packed {} files into '{}'. Index at offset {} bytes.",
        all_entries.len(),
        args.output.display(),
        index_offset
    );

    std::io::stdout().flush().ok();

    Ok(())
}

fn process_single_file(
    entry: &walkdir::DirEntry,
    input_dir: &Path,
    tags_map: &HashMap<String, Vec<String>>,
) -> Result<Option<ProcessedFile>> {
    let path = entry.path();
    let rel = path.strip_prefix(input_dir).unwrap().to_owned();
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_lowercase();
    let mut mtype = classify_ext(&ext);

    let mut width = None;
    let mut height = None;
    let mut duration = None;

    let encoded_bytes = match mtype {
        MediaType::Image => {
            if is_animated(path)? {
                mtype = MediaType::Video;
                let encoded;
                (
                    encoded,
                    Metadata {
                        width,
                        height,
                        duration,
                    },
                ) = encode_video(path, false)?;
                encoded
            } else {
                let encoded;
                (
                    encoded,
                    Metadata {
                        width,
                        height,
                        duration,
                    },
                ) = encode_image(path)?;
                encoded
            }
        }
        MediaType::Video => {
            let encoded;
            (
                encoded,
                Metadata {
                    width,
                    height,
                    duration,
                },
            ) = encode_video(path, true)?;
            encoded
        }
        MediaType::Audio => encode_audio(path)?,
        MediaType::Other => std::fs::read(path)?,
    };

    let tags = tags_map.get(&rel_str).cloned().unwrap_or_default();

    let entry = PackedEntry {
        rel_path: rel_str,
        media_type: mtype,
        offset: 0,
        length: 0,
        width,
        height,
        duration,
        tags,
    };

    Ok(Some(ProcessedFile {
        entry,
        data: encoded_bytes,
    }))
}

fn classify_ext(ext: &str) -> MediaType {
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "bmp" | "tiff" => MediaType::Image,
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "m4v" => MediaType::Video,
        "mp3" | "wav" | "flac" | "ogg" | "opus" | "m4a" => MediaType::Audio,
        _ => MediaType::Other,
    }
}

fn is_animated(path: &Path) -> Result<bool> {
    // Use ffprobe to check if the file has multiple frames/is animated
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
        Err(_) => Ok(frames_str == "N/A"), // N/A usually means infinite frames (animated)
    }
}

fn build_sqlite_index(db_path: &Path, entries: &[PackedEntry]) -> Result<()> {
    let mut conn = Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = OFF;
        PRAGMA synchronous = OFF;
        PRAGMA temp_store = MEMORY;
        PRAGMA page_size = 4096;
        CREATE TABLE media (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL,
            media_type TEXT CHECK(media_type IN ('image','video','audio','other')) NOT NULL,
            offset INTEGER NOT NULL,
            length INTEGER NOT NULL,
            width INTEGER,
            height INTEGER,
            duration REAL
        );
        CREATE TABLE tags (
            id INTEGER PRIMARY KEY,
            name TEXT UNIQUE NOT NULL
        );
        CREATE TABLE media_tags (
            media_id INTEGER NOT NULL,
            tag_id INTEGER NOT NULL,
            PRIMARY KEY(media_id, tag_id),
            FOREIGN KEY(media_id) REFERENCES media(id) ON DELETE CASCADE,
            FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
        );
        CREATE INDEX idx_tag_name ON tags(name);
        CREATE INDEX idx_media_tags ON media_tags(tag_id, media_id);
        "#,
    )?;

    let tx = conn.transaction()?;
    let mut tag_cache: HashMap<String, i64> = HashMap::new();
    {
        let mut media_stmt = tx.prepare("INSERT INTO media (path, media_type, offset, length, width, height, duration) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) RETURNING id")?;
        let mut tag_stmt = tx.prepare("INSERT INTO tags (name) VALUES (?1) RETURNING id")?;
        let mut media_tag_stmt =
            tx.prepare("INSERT INTO media_tags (media_id, tag_id) VALUES (?1, ?2)")?;

        for e in entries {
            let media_id: i64 = media_stmt.query_row(
                params![
                    e.rel_path,
                    e.media_type.as_str(),
                    e.offset as i64,
                    e.length as i64,
                    e.width,
                    e.height,
                    e.duration
                ],
                |row| row.get("id"),
            )?;

            for tag in &e.tags {
                let tag_id = if let Some(&id) = tag_cache.get(tag) {
                    id
                } else {
                    let id = tag_stmt.query_row(params![tag], |row| row.get("id"))?;
                    tag_cache.insert(tag.clone(), id);
                    id
                };

                media_tag_stmt.execute(params![media_id, tag_id])?;
            }
        }
    }
    tx.commit()?;

    Ok(())
}

fn encode_image(input: &Path) -> anyhow::Result<(Vec<u8>, Metadata)> {
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

fn encode_video(input: &Path, audio: bool) -> anyhow::Result<(Vec<u8>, Metadata)> {
    let tmp = tempfile::NamedTempFile::with_suffix(".webm")?;
    let tmp_path = tmp.path();

    #[rustfmt::skip]
    let args = [
        // -i input
        "-y",
        "-vf", "scale='min(1280,iw)':'min(720,ih)':force_original_aspect_ratio=decrease",
        "-c:v", "libvpx-vp9",
        "-cpu-used", "2",
        "-crf", "32",
        "-b:v", "0",
        "-row-mt", "1",
        "-tile-columns", "2",
        "-tile-rows", "1",
        "-c:a", if audio {"libopus"} else { "" },
        "-b:a", "64k",
        "-application", "voip",
        "-f", "webm",
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

fn encode_audio(input: &Path) -> anyhow::Result<Vec<u8>> {
    let tmp = tempfile::NamedTempFile::with_suffix("opus")?;
    let tmp_path = tmp.path();

    #[rustfmt::skip]
    let args = [
        // -i input
        "-y",
        "-c:a", "libopus",
        "-b:a", "64k",
        "-compression-level", "10",
        // tmp_path
    ];

    let status = Command::new("ffmpeg")
        .arg("-i")
        .arg(input)
        .args(args)
        .arg(tmp_path)
        .status()?;

    if !status.success() {
        bail!("ffmpeg failed for {}", input.display());
    }

    let mut buf = Vec::new();
    File::open(tmp_path)?.read_to_end(&mut buf)?;
    Ok(buf)
}

struct Metadata {
    width: Option<i64>,
    height: Option<i64>,
    duration: Option<f64>,
}

fn get_metadata(path: &Path) -> anyhow::Result<Metadata> {
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
