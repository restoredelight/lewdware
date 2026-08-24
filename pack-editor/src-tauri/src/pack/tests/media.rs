//! Importing media, removing it, and undo/redo over those edits.

use rusqlite::params;
use shared::pack::Metadata;
use tempfile::tempdir;

use super::*;
use rusqlite::TransactionBehavior;
use shared::behaviour::MediaSlot;
use shared::encode::EncodedFile;

#[tokio::test]
async fn embedded_modes_roundtrip_through_database_history() {
    let output = tempdir().unwrap();
    let data = tempdir().unwrap();
    let pack = new_test_pack(&output.path().join("modes.lwpack"), data.path(), "Modes").await;
    let bytes =
        include_bytes!("../../../../../default-modes/sandbox/build/Sandbox.lwmode").to_vec();

    let id = pack.add_mode(bytes.clone()).await.unwrap();
    assert_eq!(pack.get_modes().await.unwrap(), vec![(id, bytes.clone())]);

    pack.remove_mode(id).await.unwrap();
    assert!(pack.get_modes().await.unwrap().is_empty());
    pack.undo().await.unwrap();
    assert_eq!(pack.get_modes().await.unwrap().len(), 1);
    pack.redo().await.unwrap();
    assert!(pack.get_modes().await.unwrap().is_empty());
}

#[tokio::test]
async fn imported_media_undo_keeps_its_loose_source_for_redo() {
    let output = tempdir().unwrap();
    let data = tempdir().unwrap();
    let pack = new_test_pack(
        &output.path().join("import-history.lwpack"),
        data.path(),
        "Import history",
    )
    .await;
    let bytes = b"backend-owned import history";
    let staged = pack.dir.join("media").join("import-history.opus");
    tokio::fs::write(&staged, bytes).await.unwrap();
    let history_id = pack.begin_media_import().await.unwrap();
    let media = pack
        .add_file_to_import(
            EncodedFile {
                info: FileInfo::Audio { duration: 1.0 },
                thumbnail: None,
                path: staged.clone(),
                artists: vec![],
                source_url: None,
            },
            Path::new("import-history.wav"),
            blake3::hash(bytes),
            Some(history_id),
        )
        .await
        .unwrap()
        .unwrap();
    let status = pack.finish_media_import(history_id, None).await.unwrap();
    assert!(status.can_undo);

    pack.undo().await.unwrap();
    assert!(pack.get_files().await.unwrap().is_empty());
    assert!(staged.exists());
    pack.redo().await.unwrap();
    let restored = pack.get_files().await.unwrap();
    assert_eq!(restored[0].id, media.file().id);
    assert_eq!(
        pack.get_view()
            .unwrap()
            .get_file_data(media.file().id)
            .await
            .unwrap()
            .0,
        bytes
    );
}

#[tokio::test]
async fn reimporting_soft_deleted_content_restores_its_existing_media_row() {
    let output = tempdir().unwrap();
    let data = tempdir().unwrap();
    let pack = new_test_pack(
        &output.path().join("restore-duplicate.lwpack"),
        data.path(),
        "Restore duplicate",
    )
    .await;
    let bytes = b"same encoded media";
    let first_path = pack.dir.join("media").join("first.opus");
    tokio::fs::write(&first_path, bytes).await.unwrap();
    let first_history = pack.begin_media_import().await.unwrap();
    let first = pack
        .add_file_to_import(
            EncodedFile {
                info: FileInfo::Audio { duration: 1.0 },
                thumbnail: None,
                path: first_path,
                artists: vec![],
                source_url: None,
            },
            Path::new("first.wav"),
            blake3::hash(bytes),
            Some(first_history),
        )
        .await
        .unwrap()
        .unwrap();
    pack.finish_media_import(first_history, None).await.unwrap();
    pack.remove_files(vec![first.file().id]).await.unwrap();

    let redundant_path = pack.dir.join("media").join("second.opus");
    tokio::fs::write(&redundant_path, bytes).await.unwrap();
    let second_history = pack.begin_media_import().await.unwrap();
    let restored = pack
        .add_file_to_import(
            EncodedFile {
                info: FileInfo::Audio { duration: 1.0 },
                thumbnail: None,
                path: redundant_path.clone(),
                artists: vec![],
                source_url: None,
            },
            Path::new("second.wav"),
            blake3::hash(bytes),
            Some(second_history),
        )
        .await
        .unwrap()
        .unwrap();
    pack.finish_media_import(second_history, None)
        .await
        .unwrap();

    assert_eq!(restored.file().id, first.file().id);
    assert!(!redundant_path.exists());
    pack.undo().await.unwrap();
    assert!(pack.get_files().await.unwrap().is_empty());
    pack.redo().await.unwrap();
    assert_eq!(pack.get_files().await.unwrap()[0].id, first.file().id);
}

#[tokio::test]
async fn pending_import_is_not_trimmed_by_concurrent_history_entries() {
    let output = tempdir().unwrap();
    let data = tempdir().unwrap();
    let pack = new_test_pack(
        &output.path().join("pending-trim.lwpack"),
        data.path(),
        "Pending trim",
    )
    .await;
    let history_id = pack.begin_media_import().await.unwrap();
    for index in 0u64..105 {
        pack.set_pack_data("test", index.to_le_bytes().to_vec())
            .await
            .unwrap();
    }
    let pending_exists = pack
        .db_execute(move |connection| {
            connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM history_entries
                     WHERE id = ? AND status = 'pending')",
                    [history_id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(Into::into)
        })
        .await
        .unwrap();
    assert!(pending_exists);
    pack.finish_media_import(history_id, None).await.unwrap();
}

#[tokio::test]
async fn undo_does_not_resurrect_garbage_from_a_discarded_redo_branch() {
    let output = tempdir().unwrap();
    let data = tempdir().unwrap();
    let pack = new_test_pack(
        &output.path().join("redo-garbage.lwpack"),
        data.path(),
        "Redo garbage",
    )
    .await;
    let bytes = b"discarded redo bytes";
    let staged = pack.dir.join("media").join("redo-garbage.opus");
    tokio::fs::write(&staged, bytes).await.unwrap();
    let history_id = pack.begin_media_import().await.unwrap();
    pack.add_file_to_import(
        EncodedFile {
            info: FileInfo::Audio { duration: 1.0 },
            thumbnail: None,
            path: staged,
            artists: vec![],
            source_url: None,
        },
        Path::new("redo-garbage.wav"),
        blake3::hash(bytes),
        Some(history_id),
    )
    .await
    .unwrap();
    pack.finish_media_import(history_id, None).await.unwrap();
    pack.undo().await.unwrap();

    pack.set_metadata(&Metadata {
        name: "A new branch".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    pack.undo().await.unwrap();
    assert!(pack.get_files().await.unwrap().is_empty());
    assert_eq!(
        pack.history_status().await.unwrap().redo_label.as_deref(),
        Some("Edit pack metadata")
    );
}

#[tokio::test]
async fn bulk_remove_and_restore_exceeds_sqlite_variable_limit() {
    const COUNT: u64 = 35_000;
    let output = tempdir().unwrap();
    let data = tempdir().unwrap();
    let pack = new_test_pack(
        &output.path().join("large-removal.lwpack"),
        data.path(),
        "Large removal",
    )
    .await;
    pack.db_execute(|mut conn| {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO media
                 (id, file_name, file_type, duration, hash)
                 VALUES (?, ?, 'audio', 1.0, ?)",
            )?;
            for id in 1..=COUNT {
                insert.execute(params![
                    id,
                    format!("bulk-{id}.opus"),
                    blake3::hash(&id.to_le_bytes()).as_bytes().to_vec(),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await
    .unwrap();

    let ids = (1..=COUNT).collect::<Vec<_>>();
    pack.remove_files(ids.clone()).await.unwrap();
    assert!(pack.get_files().await.unwrap().is_empty());
    assert!(pack.history_status().await.unwrap().can_undo);
    pack.undo().await.unwrap();
    assert_eq!(pack.get_files().await.unwrap().len() as u64, COUNT);
}

/// Creating a tag and attaching it in one step used to be un-undoable: inverting that
/// changeset deletes the `tags` row, which cascades to the `media_tags` row referencing it,
/// and the changeset's own delete for that row then found nothing -- reported as a conflict
/// and aborted, failing the whole undo. Our own cascade removing a row we were going to
/// remove anyway is not a disagreement about state (see `history::on_conflict`).
#[tokio::test]
async fn undo_survives_a_changeset_whose_own_cascade_beat_it_to_a_row() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(
        &tmp.path().join("cascade.lwpack"),
        data_dir.path(),
        "Cascade",
    )
    .await;
    let media = insert_staged_audio(&pack, b"tagged").await;

    pack.create_and_add_tag(media, "fresh".to_string())
        .await
        .unwrap();
    assert_eq!(pack.get_tags(media).await.unwrap(), vec!["fresh"]);

    pack.undo().await.unwrap();
    assert!(pack.get_tags(media).await.unwrap().is_empty());
    assert!(pack.get_all_tags().await.unwrap().is_empty());

    pack.redo().await.unwrap();
    assert_eq!(pack.get_tags(media).await.unwrap(), vec!["fresh"]);
}

#[tokio::test]
async fn removed_media_can_be_restored_with_its_row_tags_and_bytes_after_save() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("history.lwpack");
    let bytes = b"undoable media bytes";
    let pack = new_test_pack(&pack_path, data_dir.path(), "History").await;
    let id = insert_staged_audio(&pack, bytes).await;
    pack.add_tags(id, vec!["one".into(), "two".into()])
        .await
        .unwrap();
    let original = pack.get_files().await.unwrap().remove(0);

    pack.remove_files(vec![id]).await.unwrap();
    assert!(pack.get_files().await.unwrap().is_empty());
    pack.save(|_, _| {}).await.unwrap();

    let history_tables_in_pack_index: u64 = pack.db_execute(|conn| {
        conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('history_media', 'history_media_tags')", [], |row| row.get(0)).map_err(Into::into)
    }).await.unwrap();
    assert_eq!(history_tables_in_pack_index, 0);

    pack.undo().await.unwrap();
    let restored = pack.get_files().await.unwrap().remove(0);
    assert_eq!(restored.id, original.id);
    assert_eq!(restored.file_name, original.file_name);
    assert_eq!(restored.hash, original.hash);
    let mut tags = pack.get_tags(id).await.unwrap();
    tags.sort();
    assert_eq!(tags, vec!["one", "two"]);
    let (restored_bytes, _) = pack.get_view().unwrap().get_file_data(id).await.unwrap();
    assert_eq!(restored_bytes, bytes);

    pack.remove_files(vec![id]).await.unwrap();
    assert!(pack.get_files().await.unwrap().is_empty());
}

#[tokio::test]
async fn purging_evicted_media_history_removes_only_staged_rows_and_bytes() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("purge.lwpack"), data_dir.path(), "Purge").await;
    let staged = insert_staged_audio(&pack, b"staged").await;
    let active = insert_staged_audio(&pack, b"active").await;
    let staged_path: String = pack
        .db_execute(move |conn| {
            conn.query_row(
                "SELECT path FROM loose_media WHERE media_id = ?",
                params![staged],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
        .await
        .unwrap();
    pack.remove_files(vec![staged]).await.unwrap();
    pack.db_execute(|conn| {
        conn.execute("DELETE FROM history_entries", [])?;
        Ok(())
    })
    .await
    .unwrap();
    pack.purge_history_files(vec![staged]).await.unwrap();

    assert!(!pack.dir().join("media").join(staged_path).exists());
    assert_eq!(
        pack.get_files()
            .await
            .unwrap()
            .iter()
            .map(|file| file.id)
            .collect::<Vec<_>>(),
        vec![active]
    );
    assert_eq!(
        pack.get_files()
            .await
            .unwrap()
            .iter()
            .map(|file| file.id)
            .collect::<Vec<_>>(),
        vec![active]
    );
}

#[tokio::test]
async fn closing_a_saved_pack_removes_staged_history_files() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(
        &tmp.path().join("cleanup.lwpack"),
        data_dir.path(),
        "Cleanup",
    )
    .await;
    let id = insert_staged_audio(&pack, b"temporary").await;
    pack.remove_files(vec![id]).await.unwrap();
    pack.save(|_, _| {}).await.unwrap();
    let working_dir = pack.dir().to_path_buf();
    assert!(working_dir.exists());
    drop(pack);
    assert!(!working_dir.exists());
}

// Simulates the upload race: two uploads for identical content, both
// reaching add_file with the same hash before either had inserted its row
// (encode.rs's own pre-check can't prevent this). The DB's UNIQUE
// constraint on hash - not just the pre-check - is what has to catch it.
#[tokio::test]
async fn duplicate_hash_upload_is_rejected_by_constraint() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "Dedup").await;

    let content = b"identical content, uploaded twice";
    let hash = blake3::hash(content);

    let encoded_path_1 = pack.dir.join("media").join("upload-1");
    tokio::fs::write(&encoded_path_1, content).await.unwrap();
    let encoded_1 = EncodedFile {
        info: FileInfo::Audio { duration: 1.0 },
        thumbnail: None,
        artists: vec![],
        source_url: None,
        path: encoded_path_1,
    };

    let encoded_path_2 = pack.dir.join("media").join("upload-2");
    tokio::fs::write(&encoded_path_2, content).await.unwrap();
    let encoded_2 = EncodedFile {
        info: FileInfo::Audio { duration: 1.0 },
        thumbnail: None,
        artists: vec![],
        source_url: None,
        path: encoded_path_2,
    };

    let first = pack
        .add_file(encoded_1, Path::new("a.wav"), hash)
        .await
        .unwrap();
    assert!(
        first.is_some(),
        "first upload of new content should succeed"
    );

    let second = pack
        .add_file(encoded_2, Path::new("b.wav"), hash)
        .await
        .unwrap();
    assert!(
        second.is_none(),
        "duplicate hash should be rejected, not inserted"
    );
    assert!(
        !pack.dir.join("media").join("upload-2").exists(),
        "an unreferenced encoded file should be deleted on final drop"
    );

    let files = pack.get_files().await.unwrap();
    assert_eq!(
        files.len(),
        1,
        "only one row should exist for the duplicate hash"
    );
}

// Two distinct uploads that happen to share a requested file name -- unlike a hash collision
// (rejected outright), this should still add the file, just under an adjusted name.
#[tokio::test]
async fn colliding_file_name_on_add_is_renamed_not_rejected() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "Names").await;

    let encoded_path_1 = pack.dir.join("media").join("upload-1");
    tokio::fs::write(&encoded_path_1, b"first").await.unwrap();
    let encoded_1 = EncodedFile {
        info: FileInfo::Audio { duration: 1.0 },
        thumbnail: None,
        artists: vec![],
        source_url: None,
        path: encoded_path_1,
    };

    let encoded_path_2 = pack.dir.join("media").join("upload-2");
    tokio::fs::write(&encoded_path_2, b"second").await.unwrap();
    let encoded_2 = EncodedFile {
        info: FileInfo::Audio { duration: 1.0 },
        thumbnail: None,
        artists: vec![],
        source_url: None,
        path: encoded_path_2,
    };

    let encoded_path_3 = pack.dir.join("media").join("upload-3");
    tokio::fs::write(&encoded_path_3, b"third").await.unwrap();
    let encoded_3 = EncodedFile {
        info: FileInfo::Audio { duration: 1.0 },
        thumbnail: None,
        artists: vec![],
        source_url: None,
        path: encoded_path_3,
    };

    let first = pack
        .add_file(encoded_1, Path::new("clip.wav"), blake3::hash(b"first"))
        .await
        .unwrap()
        .unwrap();
    let second = pack
        .add_file(encoded_2, Path::new("clip.wav"), blake3::hash(b"second"))
        .await
        .unwrap()
        .unwrap();
    let third = pack
        .add_file(encoded_3, Path::new("clip.wav"), blake3::hash(b"third"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(first.file_name, "clip.wav");
    assert_eq!(second.file_name, "clip (1).wav");
    assert_eq!(third.file_name, "clip (2).wav");

    let files = pack.get_files().await.unwrap();
    assert_eq!(files.len(), 3, "all three distinct files should be added");
}

// Unlike add_file's auto-rename, an explicit rename to an existing name is a deliberate user
// action and should be rejected with a clear error, not silently adjusted.
#[tokio::test]
async fn set_title_rejects_a_colliding_name() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "Rename").await;

    let encoded_path_1 = pack.dir.join("media").join("upload-1");
    tokio::fs::write(&encoded_path_1, b"first").await.unwrap();
    let encoded_1 = EncodedFile {
        info: FileInfo::Audio { duration: 1.0 },
        thumbnail: None,
        artists: vec![],
        source_url: None,
        path: encoded_path_1,
    };
    let first = pack
        .add_file(encoded_1, Path::new("a.wav"), blake3::hash(b"first"))
        .await
        .unwrap()
        .unwrap();

    let encoded_path_2 = pack.dir.join("media").join("upload-2");
    tokio::fs::write(&encoded_path_2, b"second").await.unwrap();
    let encoded_2 = EncodedFile {
        info: FileInfo::Audio { duration: 1.0 },
        thumbnail: None,
        artists: vec![],
        source_url: None,
        path: encoded_path_2,
    };
    let second = pack
        .add_file(encoded_2, Path::new("b.wav"), blake3::hash(b"second"))
        .await
        .unwrap()
        .unwrap();

    let err = pack
        .set_title(second.id, first.file_name.clone())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already exists"));

    // The rejected rename must not have applied.
    let files = pack.get_files().await.unwrap();
    let renamed = files.iter().find(|f| f.id == second.id).unwrap();
    assert_eq!(renamed.file_name, "b.wav");
}

/// Media references are ids, so only *deletion* can invalidate one. A rename is invisible to
/// the document -- the round trip that referencing by id exists to remove.
#[tokio::test]
async fn renaming_media_leaves_the_slots_pointing_at_it_alone() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("refs.lwpack"), data_dir.path(), "Refs").await;
    let media = insert_staged_audio(&pack, b"wallpaper").await;
    pack.fill_media_slot(MediaSlot::Wallpaper, media, true)
        .await
        .unwrap();

    pack.set_title(media, "renamed.png".to_string())
        .await
        .unwrap();

    let behaviour = read_pack_behaviour(&pack).await;
    assert_eq!(behaviour.content.wallpaper, Some(media));
    assert_eq!(
        pack.get_files().await.unwrap()[0].file_name,
        "renamed.png",
        "the rename itself still happened"
    );

    // Deleting it is the one thing that does have to clear the slot, or it is left pointing
    // at media the pack no longer has.
    pack.remove_files(vec![media]).await.unwrap();
    let behaviour = read_pack_behaviour(&pack).await;
    assert_eq!(behaviour.content.wallpaper, None);
}
