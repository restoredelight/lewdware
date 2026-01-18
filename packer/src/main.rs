mod db;

use anyhow::Result;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use shared::encode::{FileInfo, encode_file};
use shared::pack_config::OneOrMore;
use shared::read_config::{Config, MediaCategory, Resolved, find_config, glob_matches};
use shared::read_pack::{HEADER_SIZE, Header};
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{NamedTempFile};
use walkdir::WalkDir;

use crate::db::build_sqlite_index;

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
    file_name: String,
    file_info: FileInfo,
    category: MediaCategory,
    offset: u64,
    length: u64,
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

    let mut header = Header::new();

    out.write_all(&header.to_buf()?)?;

    out.seek(SeekFrom::Start(HEADER_SIZE as u64))?;

    let files: Vec<_> = WalkDir::new(&args.input)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            config.root_config.ignore.as_ref().is_none_or(|ignore| {
                let entry_path = e.path().strip_prefix(&args.input);

                entry_path.is_ok_and(|entry_path| match ignore {
                    OneOrMore::One(path) => !glob_matches(path, entry_path),
                    OneOrMore::More(items) => {
                        !items.iter().any(|path| glob_matches(path, entry_path))
                    }
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

    let index_offset = out.stream_position()?;

    let index_length = {
        let mut dbf = File::open(tmp_db.path())?;
        std::io::copy(&mut dbf, &mut out)?
    };

    let buf = config.root_config.metadata.to_buf()?;
    let metadata_length = buf.len() as u64;
    println!("{}", metadata_length);

    out.write_all(&buf)?;

    header.index_offset = index_offset;
    header.index_length = index_length;
    header.metadata_offset = index_offset + index_length;
    header.metadata_length = metadata_length;

    out.seek(SeekFrom::Start(0))?;
    out.write_all(&header.to_buf()?)?;

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
    let file_name = path.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "".to_string());
    let rel = path.strip_prefix(input_dir)?.to_owned();

    let output_file = NamedTempFile::new()?;

    let (file_info, output_path) = match encode_file(
        || Command::new("ffmpeg"),
        || Command::new("ffprobe"),
        path,
        output_file.path(),
    )? {
        Some(x) => x,
        None => return Ok(None),
    };

    let data = fs::read(output_path)?;

    let (tags, category) = config.get_tags_and_category(&rel, resolved);

    let entry = PackedEntry {
        file_name,
        file_info,
        offset: 0,
        length: 0,
        tags,
        category,
    };

    Ok(Some(ProcessedFile {
        entry,
        data,
    }))
}
