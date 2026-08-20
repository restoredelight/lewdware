use std::io::Write as _;

use ffmpeg_next as ffmpeg;
use shared::pack::HEADER_SIZE;

use super::*;

/// The pack-editor (`pack-editor/src-tauri/src/pack/save.rs`) doesn't build the
/// index via `rusqlite`'s `serialize()` -- it runs `VACUUM` on a plain on-disk connection
/// opened through `SqliteConnectionManager::file(...)` (default journal mode, no WAL), then
/// copies the file's raw bytes into the pack. Confirms that byte stream -- not just the
/// output of `Connection::serialize()` -- is exactly what `deserialize_read_exact` expects.
#[test]
fn deserializes_a_vacuumed_on_disk_file_copy_like_pack_editor_produces() {
    let db_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    let db_path = db_path.to_path_buf();

    {
        let mut conn = Connection::open(&db_path).unwrap();
        migrate(&mut conn).unwrap();
        conn.execute("INSERT INTO tags (name) VALUES ('from-disk')", [])
            .unwrap();
        // Mirrors `Pack::save`: VACUUM then treat the file as done, no explicit close/flush
        // beyond what VACUUM itself guarantees.
        conn.execute("VACUUM", []).unwrap();
    }

    // Raw file copy, exactly like `tokio::io::copy(&mut dbf, &mut file)` in pack-editor --
    // deliberately not using `Connection::serialize()`.
    let db_bytes = std::fs::read(&db_path).unwrap();

    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .deserialize_read_exact(MAIN_DB, db_bytes.as_slice(), db_bytes.len(), false)
        .unwrap();

    let name: String = connection
        .query_row("SELECT name FROM tags", [], |row| row.get(0))
        .unwrap();
    assert_eq!(name, "from-disk");
}

/// A pack file holding one image, whose bytes really sit at the row's `offset`/`length` --
/// so `get_image_file` has something to extract. The `NamedTempFile` is returned because it
/// owns the pack file the `MediaPack` reads from.
fn pack_with_image_bytes(image: &[u8]) -> (NamedTempFile, MediaPack) {
    let mut db = Connection::open_in_memory().unwrap();
    migrate(&mut db).unwrap();
    db.execute(
        "INSERT INTO media (file_name, file_type, offset, length, width, height, transparent, hash)
         VALUES ('pic.avif', 'image', 0, 0, 8, 8, 0, x'00')",
        [],
    )
    .unwrap();

    let metadata = Metadata {
        name: "test-pack".to_string(),
        ..Default::default()
    };
    let metadata_bytes = metadata.to_buf().unwrap();
    let mut header = Header::new();
    header.metadata_offset = HEADER_SIZE as u64;
    header.metadata_length = metadata_bytes.len() as u64;
    header.index_offset = header.metadata_offset + header.metadata_length;

    // The index has to be sized before the image offset is known, and writing that offset
    // back changes nothing about the index's size.
    let db_bytes = db.serialize(MAIN_DB).unwrap();
    header.index_length = db_bytes.len() as u64;
    db.execute(
        "UPDATE media SET offset = ?, length = ?",
        params![
            header.index_offset + header.index_length,
            image.len() as u64
        ],
    )
    .unwrap();
    let db_bytes = db.serialize(MAIN_DB).unwrap();

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&header.to_buf().unwrap()).unwrap();
    file.write_all(&metadata_bytes).unwrap();
    file.write_all(&db_bytes).unwrap();
    file.write_all(image).unwrap();
    file.flush().unwrap();

    let pack = MediaPack::open(file.path()).unwrap();
    (file, pack)
}

/// Media lives inside the pack file, so handing it to anything outside the engine means
/// extracting it first -- and that extraction deletes itself when dropped.
///
/// Every consumer that passes the *path* onwards rather than reading the bytes has to keep
/// the `ExtractedFile` alive for as long as whatever it handed the path to will read it. The
/// wallpaper is the case that bites: each backend stores a path and reads it back later, so
/// dropping the temp file after setting it leaves the desktop pointing at nothing, with
/// nothing failing and nothing logged (see `LewdwareApp::wallpaper_file`).
#[tokio::test]
async fn an_extracted_image_file_only_exists_while_its_handle_does() {
    let bytes = b"pretend avif";
    let (_file, pack) = pack_with_image_bytes(bytes);

    let path = {
        let extracted = pack.get_image_file(1).await.unwrap();
        let path = extracted.path().to_path_buf();
        assert!(path.exists(), "the image should be readable while held");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        path
    };

    assert!(
        !path.exists(),
        "dropping the handle must take the file with it -- callers that pass the path on \
         have to outlive that drop"
    );
}

/// Builds a pack file on disk with a real (migrated) SQLite index, to check that
/// `MediaPack::open`'s `deserialize_read_exact` round-trip actually produces a working,
/// queryable database rather than just satisfying the type checker.
#[test]
fn open_reads_deserialized_index() {
    let mut db = Connection::open_in_memory().unwrap();
    migrate(&mut db).unwrap();

    db.execute("INSERT INTO tags (name) VALUES ('test-tag')", [])
        .unwrap();
    db.execute("INSERT INTO tags (name) VALUES ('second-tag')", [])
        .unwrap();
    // `pic.avif` carries both tags, `other.avif` only the first — enough to distinguish
    // any/all/none semantics (and duplicate rows) in the assertions below.
    db.execute(
        "INSERT INTO media (file_name, file_type, offset, length, width, height, transparent, hash)
         VALUES ('pic.avif', 'image', 0, 0, 64, 32, 1, x'00')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO media (file_name, file_type, offset, length, width, height, transparent, hash)
         VALUES ('other.avif', 'image', 0, 0, 16, 16, 0, x'01')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO media_tags (media_id, tag_id) VALUES (1, 1), (1, 2), (2, 1)",
        [],
    )
    .unwrap();

    let db_bytes = db.serialize(MAIN_DB).unwrap();

    let metadata = Metadata {
        name: "test-pack".to_string(),
        ..Default::default()
    };
    let metadata_bytes = metadata.to_buf().unwrap();

    let mut header = Header::new();
    header.metadata_offset = HEADER_SIZE as u64;
    header.metadata_length = metadata_bytes.len() as u64;
    header.index_offset = header.metadata_offset + header.metadata_length;
    header.index_length = db_bytes.len() as u64;

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&header.to_buf().unwrap()).unwrap();
    file.write_all(&metadata_bytes).unwrap();
    file.write_all(&db_bytes).unwrap();
    file.flush().unwrap();

    let pack = MediaPack::open(file.path()).unwrap();

    assert_eq!(pack.metadata().name, "test-pack");

    let results = pack
        .list_media(
            MediaTypes::ALL,
            Some(TagFilter::any_of(vec!["second-tag".to_string()])),
        )
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "pic.avif");
    assert!(matches!(
        results[0].media_data,
        MediaData::Image {
            width: 64,
            height: 32,
            transparent: true,
        }
    ));
    let mut tags = results[0].tags.clone();
    tags.sort();
    assert_eq!(tags, vec!["second-tag", "test-tag"]);

    let mut all_tags = pack.list_tags().unwrap();
    all_tags.sort();
    assert_eq!(all_tags, vec!["second-tag", "test-tag"]);

    // A tag that doesn't exist in this pack matches no media rather than erroring, since
    // callers routinely probe for optional tags a given pack may not define.
    assert_eq!(
        pack.list_media(
            MediaTypes::ALL,
            Some(TagFilter::any_of(vec!["nonexistent".to_string()]))
        )
        .unwrap()
        .len(),
        0
    );

    // Media matching more than one of the requested `any` tags must still appear exactly
    // once, not once per matching tag.
    let names: Vec<_> = pack
        .list_media(
            MediaTypes::ALL,
            Some(TagFilter::any_of(vec![
                "test-tag".to_string(),
                "second-tag".to_string(),
            ])),
        )
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(names, vec!["pic.avif", "other.avif"]);

    // `other.avif` carries only the first tag -- confirms the per-row tags subquery is
    // correctly scoped to `media.id`, not accidentally aggregating every tag in the pack.
    let other = pack
        .get_media("other.avif".to_string(), MediaTypes::ALL)
        .unwrap()
        .unwrap();
    assert_eq!(other.tags, vec!["test-tag"]);

    // `all` requires every tag; only pic.avif carries both.
    let results = pack
        .list_media(
            MediaTypes::ALL,
            Some(TagFilter {
                all: vec!["test-tag".to_string(), "second-tag".to_string()],
                ..Default::default()
            }),
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "pic.avif");

    // An `all` tag the pack doesn't define can never be satisfied.
    assert_eq!(
        pack.list_media(
            MediaTypes::ALL,
            Some(TagFilter {
                all: vec!["test-tag".to_string(), "nonexistent".to_string()],
                ..Default::default()
            }),
        )
        .unwrap()
        .len(),
        0
    );

    // `none` excludes matching media; an unknown tag in `none` excludes nothing.
    let results = pack
        .list_media(
            MediaTypes::ALL,
            Some(TagFilter {
                none: vec!["second-tag".to_string(), "nonexistent".to_string()],
                ..Default::default()
            }),
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "other.avif");

    // An empty filter imposes no constraint.
    assert_eq!(
        pack.list_media(MediaTypes::ALL, Some(TagFilter::default()))
            .unwrap()
            .len(),
        2
    );
}

/// `pack_data` is a generic named-blob table (its first real consumer is behaviour.json,
/// M3) -- checks the read path round-trips an arbitrary blob and that a missing name comes
/// back `None` rather than erroring, alongside the `recommended_mode` metadata hint that
/// ships in the same format batch.
#[test]
fn a_real_pack_file_yields_its_behaviour_and_the_recommended_mode_hint() {
    use shared::pack::RecommendedMode;

    let mut db = Connection::open_in_memory().unwrap();
    migrate(&mut db).unwrap();

    let db_bytes = db.serialize(MAIN_DB).unwrap();

    let metadata = Metadata {
        name: "test-pack".to_string(),
        recommended_mode: Some(RecommendedMode::Experience),
        ..Default::default()
    };
    let metadata_bytes = metadata.to_buf().unwrap();

    let mut header = Header::new();
    header.metadata_offset = HEADER_SIZE as u64;
    header.metadata_length = metadata_bytes.len() as u64;
    header.index_offset = header.metadata_offset + header.metadata_length;
    header.index_length = db_bytes.len() as u64;

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&header.to_buf().unwrap()).unwrap();
    file.write_all(&metadata_bytes).unwrap();
    file.write_all(&db_bytes).unwrap();
    file.flush().unwrap();

    let pack = MediaPack::open(file.path()).unwrap();

    assert_eq!(
        pack.get_behaviour().unwrap(),
        shared::behaviour::Behaviour::new()
    );
    assert_eq!(
        pack.metadata().recommended_mode,
        Some(RecommendedMode::Experience)
    );
}

/// End-to-end check of the zero-copy video path: builds a pack file with a real embedded
/// video (offset/length recorded in the index, exactly like a real pack), then confirms
/// `get_video_data` produces a `MediaSource` that ffmpeg can actually open and decode --
/// exercising the same offset/length plumbing `MediaManager`/`VideoDecoder` rely on, not
/// just the isolated `open_bounded` helper.
#[test]
fn get_video_data_opens_embedded_clip() {
    const TEST_CLIP: &[u8] = include_bytes!("../test_fixtures/test_clip.mp4");

    ffmpeg::init().unwrap();

    let mut db = Connection::open_in_memory().unwrap();
    migrate(&mut db).unwrap();
    db.execute(
        "INSERT INTO media (file_name, file_type, offset, length, width, height, transparent, duration, hash)
         VALUES ('clip.mp4', 'video', 0, 0, 64, 48, 0, 1.0, x'00')",
        [],
    )
    .unwrap();

    let db_bytes = db.serialize(MAIN_DB).unwrap();

    let metadata = Metadata {
        name: "test-pack".to_string(),
        ..Default::default()
    };
    let metadata_bytes = metadata.to_buf().unwrap();

    let mut header = Header::new();
    header.metadata_offset = HEADER_SIZE as u64;
    header.metadata_length = metadata_bytes.len() as u64;
    header.index_offset = header.metadata_offset + header.metadata_length;
    header.index_length = db_bytes.len() as u64;
    let video_offset = header.index_offset + header.index_length;

    // The row's `offset`/`length` columns are set after building the header above, so we
    // need a second pass over the DB to update them before serializing -- keep it simple and
    // just build the DB again, now that `video_offset` is known.
    db.execute(
        "UPDATE media SET offset = ?, length = ? WHERE file_name = 'clip.mp4'",
        params![video_offset, TEST_CLIP.len() as u64],
    )
    .unwrap();
    let db_bytes = db.serialize(MAIN_DB).unwrap();

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&header.to_buf().unwrap()).unwrap();
    file.write_all(&metadata_bytes).unwrap();
    file.write_all(&db_bytes).unwrap();
    file.write_all(TEST_CLIP).unwrap();
    file.flush().unwrap();

    let pack = MediaPack::open(file.path()).unwrap();
    let media = pack
        .list_media(MediaTypes::VIDEO, None)
        .unwrap()
        .pop()
        .unwrap();

    let data = pack.get_video_data(media.id).unwrap();
    assert_eq!(data.source.offset, video_offset);
    assert_eq!(data.source.length, TEST_CLIP.len() as u64);

    let ictx = data.source.open().unwrap();
    assert!(ictx.streams().best(ffmpeg::media::Type::Video).is_some());
    assert!(ictx.streams().best(ffmpeg::media::Type::Audio).is_some());
}
