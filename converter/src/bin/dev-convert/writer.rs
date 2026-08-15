//! One-shot `.lwpack` assembly for the dev batch driver. Unlike `pack-editor`'s `MediaPack`
//! (built for repeated incremental/interactive saves -- staged files, offset compaction, a
//! dedicated DB-writer thread), this always builds a pack from scratch in a single pass, so none
//! of that machinery is needed.

use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Seek, SeekFrom, Write},
    path::Path,
};

use anyhow::{Context, Result};
use converter::{ConversionOutput, PackSource};
use rusqlite::{Connection, params};
use shared::{
    db::migrate,
    encode::{HardwareEncoder, encode_file, hash_file},
    read_pack::Header,
};
use uuid::Uuid;

/// Writes `conversion` out as a `.lwpack` at `output_path`, pulling media bytes from `source` and
/// re-encoding each one via `encoder`. Returns the number of media files actually written (may be
/// less than `conversion.media.len()` if a file couldn't be read or ffprobe couldn't identify
/// it -- these degrade to a printed warning, matching `convert()`'s own "content problems don't
/// abort the run" stance).
pub fn write_pack(
    output_path: &Path,
    conversion: &ConversionOutput,
    source: &dyn PackSource,
    encoder: &HardwareEncoder,
) -> Result<usize> {
    let mut out =
        File::create(output_path).with_context(|| format!("creating {}", output_path.display()))?;
    out.write_all(&Header::new().to_buf()?)?;

    let work_dir = tempfile::tempdir()?;
    let db_path = work_dir.path().join("index.db");
    let db = Connection::open(&db_path)?;
    migrate(&db)?;

    let mut used_names: HashSet<String> = HashSet::new();
    let mut used_hashes: HashSet<[u8; 32]> = HashSet::new();
    let mut written = 0usize;

    for (i, media) in conversion.media.iter().enumerate() {
        let input_path = work_dir.path().join(format!("in-{i}"));
        if let Err(err) = source.extract_file(&media.source_path, &input_path) {
            eprintln!(
                "warning: couldn't read \"{}\" from the source pack ({err}); skipping",
                media.source_path
            );
            continue;
        }

        let output_stem = work_dir.path().join(format!("out-{i}"));
        let encoded = match encode_file(&input_path, &output_stem, encoder.clone()) {
            Ok(Some(encoded)) => encoded,
            Ok(None) => {
                eprintln!(
                    "warning: \"{}\" isn't a recognizable image/video/audio file; skipping",
                    media.source_path
                );
                let _ = fs::remove_file(&input_path);
                continue;
            }
            Err(err) => {
                eprintln!(
                    "warning: failed to encode \"{}\" ({err}); skipping",
                    media.source_path
                );
                let _ = fs::remove_file(&input_path);
                continue;
            }
        };
        let _ = fs::remove_file(&input_path);

        let hash = hash_file(&encoded.path)?;
        if !used_hashes.insert(*hash.as_bytes()) {
            eprintln!(
                "warning: \"{}\" is identical to already-converted media; skipping duplicate",
                media.source_path
            );
            let _ = fs::remove_file(&encoded.path);
            continue;
        }

        let file_name = unique_file_name(&media.suggested_name, &mut used_names);

        let offset = out.stream_position()?;
        let mut encoded_file = File::open(&encoded.path)
            .with_context(|| format!("opening encoded output {}", encoded.path.display()))?;
        let length = io::copy(&mut encoded_file, &mut out)?;
        drop(encoded_file);
        let _ = fs::remove_file(&encoded.path);

        let parts = encoded.info.to_parts();
        db.execute(
            "INSERT INTO media (file_name, file_type, offset, length, width, height, transparent, duration, audio, hash, thumbnail)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                file_name,
                parts.file_type.as_str(),
                offset,
                length,
                parts.width,
                parts.height,
                parts.transparent,
                parts.duration,
                parts.audio,
                hash.as_bytes().as_slice(),
                encoded.thumbnail,
            ],
        )?;
        let media_id = db.last_insert_rowid();

        for tag in &media.tags {
            db.execute("INSERT OR IGNORE INTO tags (name) VALUES (?)", params![tag])?;
            let tag_id: i64 =
                db.query_row("SELECT id FROM tags WHERE name = ?", params![tag], |row| {
                    row.get(0)
                })?;
            db.execute(
                "INSERT OR IGNORE INTO media_tags (media_id, tag_id) VALUES (?, ?)",
                params![media_id, tag_id],
            )?;
        }

        written += 1;
    }

    // Through the storage module: the document is rows now, so writing it is its business.
    let mut db = db;
    let tx = db.transaction()?;
    shared::behaviour::storage::write(&tx, &conversion.behaviour)?;
    tx.commit()?;

    // Forces a full checkpoint into the main db file -- without this, data written under a WAL
    // journal could still be sitting in a separate `-wal` file when we read `db_path`'s raw bytes
    // below (mirrors pack-editor's `MediaPack::save`, which VACUUMs before copying the db file).
    db.execute("VACUUM", [])?;
    drop(db);

    let index_offset = out.stream_position()?;
    let db_bytes = fs::read(&db_path).with_context(|| format!("reading {}", db_path.display()))?;
    out.write_all(&db_bytes)?;
    let index_length = db_bytes.len() as u64;

    let metadata_offset = index_offset + index_length;
    let metadata_bytes = conversion.metadata.to_buf()?;
    out.write_all(&metadata_bytes)?;
    let metadata_length = metadata_bytes.len() as u64;

    let header = Header {
        id: Uuid::new_v4(),
        index_offset,
        index_length,
        metadata_offset,
        metadata_length,
    };
    out.seek(SeekFrom::Start(0))?;
    out.write_all(&header.to_buf()?)?;

    Ok(written)
}

/// Resolves a name collision the same way pack-editor's `MediaPack::add_file` does: bump a
/// trailing `" (n)"` before the extension, counting up on repeated collisions.
fn unique_file_name(suggested: &str, used: &mut HashSet<String>) -> String {
    let mut name = suggested.to_string();
    while !used.insert(name.clone()) {
        name = next_candidate_name(&name);
    }
    name
}

fn next_candidate_name(name: &str) -> String {
    let (base, ext) = match name.rsplit_once('.') {
        Some((base, ext)) if !base.is_empty() => (base, Some(ext)),
        _ => (name, None),
    };

    let (stem, next_n) = match base.rfind(" (") {
        Some(open)
            if base.ends_with(')')
                && let Ok(n) = base[open + 2..base.len() - 1].parse::<u32>() =>
        {
            (&base[..open], n + 1)
        }
        _ => (base, 1),
    };

    match ext {
        Some(ext) => format!("{stem} ({next_n}).{ext}"),
        None => format!("{stem} ({next_n})"),
    }
}
