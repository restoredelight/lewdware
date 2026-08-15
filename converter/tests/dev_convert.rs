//! Exercises the `dev-convert` binary (`src/bin/dev-convert/`) end to end: build a tiny synthetic
//! Edgeware pack with *real* (ffmpeg-decodable) media -- unlike `tests/fixtures/{modern,legacy}`,
//! which hold placeholder text and are only ever read structurally by `convert()` itself, never
//! actually encoded -- and confirm the produced `.lwpack` is a well-formed pack: header/metadata
//! parse, the embedded SQLite index has the expected media/tag rows, and the `pack_data` row named
//! `"behaviour"` round-trips to the expected content.
//!
//! Soft-skipped (like `tests/real_pack.rs`) when `ffmpeg`/`ffprobe` aren't on `PATH` -- this is
//! the dev-only driver's whole premise, so CI environments without them just don't get this
//! coverage rather than failing.

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::Command,
};

use rusqlite::Connection;
use shared::read_pack::read_pack_metadata;

fn tool_available(name: &str) -> bool {
    Command::new(name)
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn ffmpeg_available() -> bool {
    tool_available("ffmpeg") && tool_available("ffprobe")
}

/// Builds a directory-layout Edgeware pack with one real, tiny, ffmpeg-generated image and audio
/// file (video is skipped -- an image and an audio file already exercise both the image/audio
/// encode paths and the media/tags DB rows; adding video wouldn't test anything new here).
fn build_real_media_pack(root: &Path) {
    fs::create_dir_all(root.join("img")).unwrap();
    fs::create_dir_all(root.join("aud")).unwrap();

    fs::write(
        root.join("info.json"),
        r#"{"name": "Dev Convert Test Pack", "creator": "Test"}"#,
    )
    .unwrap();
    fs::write(
        root.join("index.json"),
        r#"{"moods": [{"mood": "test", "media": ["a.png"], "captions": ["Hi"]}]}"#,
    )
    .unwrap();

    let status = Command::new("ffmpeg")
        .args(["-y", "-f", "lavfi", "-i", "color=c=blue:s=32x32"])
        .args(["-frames:v", "1", "-update", "1"])
        .arg(root.join("img/a.png"))
        .status()
        .unwrap();
    assert!(status.success(), "failed to generate test image fixture");

    let status = Command::new("ffmpeg")
        .args(["-y", "-f", "lavfi", "-i", "anullsrc=r=8000"])
        .args(["-t", "1"])
        .arg(root.join("aud/b.mp3"))
        .status()
        .unwrap();
    assert!(status.success(), "failed to generate test audio fixture");
}

#[test]
fn dev_convert_produces_a_loadable_pack() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg/ffprobe not on PATH");
        return;
    }

    let src_dir = tempfile::tempdir().unwrap();
    build_real_media_pack(src_dir.path());

    let out_dir = tempfile::tempdir().unwrap();
    let output_path = out_dir.path().join("out.lwpack");

    let status = Command::new(env!("CARGO_BIN_EXE_dev-convert"))
        .arg(src_dir.path())
        .arg(&output_path)
        .status()
        .unwrap();
    assert!(status.success());

    let mut file = fs::File::open(&output_path).unwrap();
    let (header, metadata) = read_pack_metadata(&mut file).unwrap();
    assert_eq!(metadata.name, "Dev Convert Test Pack");
    assert_eq!(metadata.creator.as_deref(), Some("Test"));

    file.seek(SeekFrom::Start(header.index_offset)).unwrap();
    let mut db_bytes = vec![0u8; header.index_length as usize];
    file.read_exact(&mut db_bytes).unwrap();
    let db_path = out_dir.path().join("index.db");
    fs::write(&db_path, &db_bytes).unwrap();
    let db = Connection::open(&db_path).unwrap();

    let media_count: i64 = db
        .query_row("SELECT COUNT(*) FROM media", [], |r| r.get(0))
        .unwrap();
    assert_eq!(media_count, 2, "expected the image and the audio file");

    let tagged_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM media_tags mt JOIN tags t ON mt.tag_id = t.id WHERE t.name = 'test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tagged_count, 1, "only the image was tagged \"test\"");

    let behaviour = shared::behaviour::storage::read(&db).unwrap();
    assert_eq!(behaviour.content.content_groups.len(), 1);
    assert_eq!(behaviour.content.content_groups[0].id, "test");
    assert_eq!(behaviour.content.captions.len(), 1);
    assert_eq!(behaviour.content.captions[0].text, "Hi");
}
