mod db;
mod encode;

use anyhow::Result;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use shared::pack_config::OneOrMore;
use shared::read_config::{find_config, glob_matches, Config, MediaCategory, Resolved};
use shared::read_pack::{Header, HEADER_SIZE};
use shared::utils::{classify_ext, FileType};
use rayon::prelude::*;
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use walkdir::WalkDir;

use crate::db::{build_sqlite_index};
use crate::encode::{Metadata, encode_audio, encode_image, encode_video, is_animated};

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
    #[arg(long, help = "Prefer file size over encoding speed (uses VP9)")]
    prefer_compression: bool,
    #[arg(
        long,
        help = "Force software encoding even if hardware acceleration is available"
    )]
    no_hw_accel: bool,
}

#[derive(Debug, Clone)]
struct PackedEntry {
    rel_path: String,
    media_type: FileType,
    category: MediaCategory,
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

    let config = find_config(&args.input)?;

    let resolved = config.resolve();

    let mut out = OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .truncate(true)
        .open(&args.output)?;

    Header {
        index_length: 0,
        total_files: 0,
        metadata_length: 0,
    }
    .write_to(&mut out)?;

    out.seek(SeekFrom::Start(HEADER_SIZE as u64))?;

    let files: Vec<_> = WalkDir::new(&args.input)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            config.root_config.ignore.as_ref().is_none_or(|ignore| {
                let entry_path = e.path().strip_prefix(&args.input);

                entry_path.is_ok_and(|entry_path| match ignore {
                    OneOrMore::One(path) => !glob_matches(path, entry_path),
                    OneOrMore::More(items) => !items
                        .iter()
                        .any(|path| glob_matches(path, entry_path)),
                })
            })
        })
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

    out.seek(SeekFrom::Start(current_offset))?;

    for chunk in files.chunks(args.chunk_size) {
        pb.set_message("Processing chunk...");

        let processed_files: Vec<ProcessedFile> = chunk
            .par_iter()
            .filter_map(|entry| {
                match process_single_file(entry, &args.input, &config, &resolved) {
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
    build_sqlite_index(tmp_db.path(), &all_entries, &config, resolved)?;

    let index_length = {
        let mut dbf = File::open(tmp_db.path())?;
        std::io::copy(&mut dbf, &mut out)?
    };

    let buf = config.root_config.metadata.to_buf()?;
    let metadata_length = buf.len() as u64;
    println!("{}", metadata_length);

    out.write_all(&buf)?;

    let header = Header {
        index_length,
        metadata_length,
        total_files: all_entries.len() as u32,
    };
    header.write_to(&mut out)?;

    println!(
        "✅ Packed {} files into '{}'.",
        all_entries.len(),
        args.output.display(),
    );

    std::io::stdout().flush().ok();

    Ok(())
}

fn process_single_file(
    entry: &walkdir::DirEntry,
    input_dir: &Path,
    config: &Config,
    resolved: &Resolved,
) -> Result<Option<ProcessedFile>> {
    let path = entry.path();
    let rel = path.strip_prefix(input_dir).unwrap().to_owned();
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let mut mtype = classify_ext(path);

    let mut width = None;
    let mut height = None;
    let mut duration = None;

    let encoded_bytes = match mtype {
        FileType::Image => {
            if is_animated(path)? {
                mtype = FileType::Video;
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
        FileType::Video => {
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
        FileType::Audio => encode_audio(path)?,
        FileType::Other => return Ok(None),
    };

    let (tags, category) = config.get_tags_and_category(&rel, resolved);

    let entry = PackedEntry {
        rel_path: rel_str,
        media_type: mtype,
        offset: 0,
        length: 0,
        width,
        height,
        duration,
        tags,
        category,
    };

    Ok(Some(ProcessedFile {
        entry,
        data: encoded_bytes,
    }))
}
