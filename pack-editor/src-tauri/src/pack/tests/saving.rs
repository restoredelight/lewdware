//! The save pipeline: snapshots, generations, in-place compaction, and recovery.

use std::sync::{Condvar, Mutex};

use rusqlite::params;
use shared::pack::Metadata;
use tempfile::tempdir;

use super::*;
use rusqlite::TransactionBehavior;
use shared::encode::EncodedFile;
use std::{
    io::{Seek, Write},
    time::Duration,
};

#[tokio::test]
async fn draft_lock_blocks_concurrent_recovery_and_normal_close_preserves_the_draft() {
    let data = tempdir().unwrap();
    let pack = MediaPack::new_unsaved(data.path(), "Locked draft")
        .await
        .unwrap();
    let id = pack.id();
    let dir = pack.dir().to_path_buf();
    let lock_path = draft_lock_path(data.path(), id);

    assert!(lock_path.is_file());
    assert!(!dir.join(".lock").exists());

    assert!(MediaPack::recover_unsaved(data.path(), id).await.is_err());
    assert!(MediaPack::remove_recoverable_draft(data.path(), id).is_err());
    assert!(dir.join("UNSAVED").is_file());

    pack.close().await;
    drop(pack);
    assert!(dir.join("UNSAVED").is_file());
    assert!(!lock_path.exists());

    let recovered = MediaPack::recover_unsaved(data.path(), id).await.unwrap();
    assert!(lock_path.is_file());
    recovered.close_and_discard().await;
    drop(recovered);
    assert!(!dir.exists());
    assert!(!lock_path.exists());
}

#[tokio::test]
async fn destinationless_pack_uses_its_generated_id_on_first_save() {
    let output = tempdir().unwrap();
    let data = tempdir().unwrap();
    let pack = MediaPack::new_unsaved(data.path(), "Untitled Pack")
        .await
        .unwrap();
    let id = pack.header.read().unwrap().id;
    assert!(pack.path().is_none());
    assert!(pack.dir().ends_with(id.to_string()));

    let destination = output.path().join("first-save.lwpack");
    let opened = pack
        .save_as(&destination, |_, _| {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(opened.header.read().unwrap().id, id);
    assert_eq!(opened.path(), Some(destination.as_path()));
    assert!(destination.is_file());
}

#[tokio::test]
async fn reopening_reconciles_a_generation_committed_before_the_working_db() {
    let output = tempdir().unwrap();
    let data = tempdir().unwrap();
    let path = output.path().join("generation-recovery.lwpack");
    let pack = new_test_pack(&path, data.path(), "Generation recovery").await;
    let bytes = b"generation recovery bytes";
    let id = insert_staged_audio(&pack, bytes).await;
    pack.save(|_, _| {}).await.unwrap();
    pack.set_metadata(&Metadata {
        name: "Unsaved metadata".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    pack.db_execute(move |conn| {
        conn.execute(
            "UPDATE pack_media SET generation_id = 'stale', \"offset\" = 999999 WHERE media_id = ?",
            [id],
        )?;
        conn.execute(
            "UPDATE editor_state SET archive_generation = 'stale' WHERE singleton = 1",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    drop(pack);

    let reopened = MediaPack::open(path, data.path()).await.unwrap();
    assert_eq!(reopened.name(), "Unsaved metadata");
    assert_eq!(
        reopened
            .get_view()
            .unwrap()
            .get_file_data(id)
            .await
            .unwrap()
            .0,
        bytes
    );
    let generation: (String, String) = reopened
        .db_execute(move |conn| {
            conn.query_row(
                "SELECT p.generation_id, e.archive_generation FROM pack_media p
                 CROSS JOIN editor_state e WHERE p.media_id = ? AND e.singleton = 1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(Into::into)
        })
        .await
        .unwrap();
    assert_eq!(generation.0, generation.1);
    assert_ne!(generation.0, "stale");
}

#[tokio::test]
async fn destinationless_pack_can_be_recovered_from_its_working_directory() {
    let data = tempdir().unwrap();
    let pack = MediaPack::new_unsaved(data.path(), "Crash draft")
        .await
        .unwrap();
    let id = pack.id();
    drop(pack);

    let recovered = MediaPack::recover_unsaved(data.path(), id).await.unwrap();
    assert_eq!(recovered.name(), "Crash draft");
    assert_eq!(recovered.id(), id);
    assert!(recovered.path().is_none());
    assert!(!recovered.is_saved().await);
}

#[tokio::test]
async fn save_and_reopen_preserves_metadata() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "My Pack").await;
    pack.save(|_, _| {}).await.unwrap();
    drop(pack);

    let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    assert_eq!(pack2.name(), "My Pack");
    assert!(pack2.is_saved().await);
}

#[tokio::test]
async fn edits_during_snapshot_save_remain_dirty_and_are_not_written_into_that_snapshot() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("snapshot.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "Before snapshot").await;
    insert_staged_audio(&pack, &vec![7; 2 * 1024 * 1024]).await;

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let save = pack.save({
        let started_tx = started_tx.clone();
        move |_, _| {
            if let Some(tx) = started_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
        }
    });
    let edit = async {
        started_rx.await.unwrap();
        pack.set_metadata(&Metadata {
            name: "Edited while saving".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    };
    let (save_result, ()) = tokio::join!(save, edit);
    save_result.unwrap();

    assert!(
        !pack.is_saved().await,
        "a post-snapshot edit must remain dirty"
    );
    assert_eq!(pack.name(), "Edited while saving");

    let mut file = File::open(&pack_path).await.unwrap();
    let mut header_buf = [0; HEADER_SIZE];
    file.read_exact(&mut header_buf).await.unwrap();
    let header = Header::from_buf(header_buf).unwrap();
    file.seek(SeekFrom::Start(header.metadata_offset))
        .await
        .unwrap();
    let mut metadata_buf = vec![0; header.metadata_length as usize];
    file.read_exact(&mut metadata_buf).await.unwrap();
    assert_eq!(
        Metadata::from_buf(&metadata_buf).unwrap().name,
        "Before snapshot"
    );
}

#[tokio::test]
async fn discard_waits_for_an_in_flight_generation_save() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("discard-save-race.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "Discard race").await;
    insert_staged_audio(&pack, b"initial").await;
    pack.save(|_, _| {}).await.unwrap();
    insert_staged_audio(&pack, &vec![3; 4 * 1024 * 1024]).await;
    pack.mark_unsaved().await.unwrap();

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let save = pack.save({
        let started_tx = started_tx.clone();
        move |_, _| {
            if let Some(tx) = started_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
        }
    });
    let discard = async {
        started_rx.await.unwrap();
        pack.discard_changes().await.unwrap();
    };
    let (save_result, ()) = tokio::join!(save, discard);
    save_result.unwrap();

    assert!(pack.is_saved().await);
    assert_eq!(
        pack.db_execute(move |connection| {
            connection
                .query_row("SELECT COUNT(*) FROM save_sessions", [], |row| {
                    row.get::<_, u64>(0)
                })
                .map_err(Into::into)
        })
        .await
        .unwrap(),
        0
    );
    for file in pack.get_files().await.unwrap() {
        pack.get_view()
            .unwrap()
            .get_file_data(file.id)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn removing_loose_media_during_save_keeps_undo_bytes_alive() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("snapshot-remove.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "Snapshot remove").await;
    let bytes = vec![9; 2 * 1024 * 1024];
    let path = pack.dir.join("media").join("snapshot-remove.opus");
    tokio::fs::write(&path, &bytes).await.unwrap();
    let media = pack
        .add_file(
            EncodedFile {
                info: FileInfo::Audio { duration: 1.0 },
                thumbnail: None,
                path: path.clone(),
                artists: vec![],
                source_url: None,
            },
            Path::new("snapshot-remove.wav"),
            blake3::hash(&bytes),
        )
        .await
        .unwrap()
        .unwrap();

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let save = pack.save({
        let started_tx = started_tx.clone();
        move |_, _| {
            if let Some(tx) = started_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
        }
    });
    let remove = async {
        started_rx.await.unwrap();
        pack.remove_files(vec![media.id]).await.unwrap();
    };
    let (save_result, ()) = tokio::join!(save, remove);
    save_result.unwrap();

    assert!(!pack.is_saved().await);
    pack.undo().await.unwrap();
    let (restored, _) = pack
        .get_view()
        .unwrap()
        .get_file_data(media.id)
        .await
        .unwrap();
    assert_eq!(restored, bytes);
}

#[tokio::test]
async fn generation_save_does_not_block_archive_previews_while_copying() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("generation-preview.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "Generation preview").await;
    let first = insert_staged_audio(&pack, &[1; 1024]).await;
    let second_bytes = vec![2; 2 * 1024 * 1024];
    let second = insert_staged_audio(&pack, &second_bytes).await;
    pack.save(|_, _| {}).await.unwrap();
    pack.remove_files(vec![first]).await.unwrap();

    let view = pack.get_view().unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let save = pack.save({
        let started_tx = started_tx.clone();
        let gate = gate.clone();
        move |_, _| {
            if let Some(tx) = started_tx.lock().unwrap().take() {
                let _ = tx.send(());
                let (released, wake) = &*gate;
                let mut released = released.lock().unwrap();
                while !*released {
                    released = wake.wait(released).unwrap();
                }
            }
        }
    });
    let preview = async {
        started_rx.await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(1), view.get_file_data(second)).await;
        let (released, wake) = &*gate;
        *released.lock().unwrap() = true;
        wake.notify_all();
        let data = result
            .expect("preview should read the old generation while the new one is written")
            .unwrap()
            .0;
        assert_eq!(data, second_bytes);
    };
    let (saved, ()) = tokio::join!(save, preview);
    saved.unwrap();
}

#[tokio::test]
async fn synchronous_in_place_fallback_compacts_in_source_order() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("in-place-fallback.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "In-place fallback").await;
    let contents: Vec<Vec<u8>> = (0..8u8)
        .map(|value| vec![value; 1024 + value as usize * 127])
        .collect();
    let mut ids = Vec::new();
    for content in &contents {
        ids.push(insert_staged_audio(&pack, content).await);
    }
    pack.save(|_, _| {}).await.unwrap();
    pack.remove_files(vec![ids[1], ids[4]]).await.unwrap();

    let snapshot = pack.create_save_snapshot().await.unwrap();
    pack.save_in_place(&snapshot, Arc::new(|_, _| {}))
        .await
        .unwrap();
    drop(pack);

    let reopened = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    let view = reopened.get_view().unwrap();
    for (index, expected) in contents.iter().enumerate() {
        if index == 1 || index == 4 {
            continue;
        }
        assert_eq!(view.get_file_data(ids[index]).await.unwrap().0, *expected);
    }
}

#[tokio::test]
async fn file_content_survives_save_and_reopen() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let content = b"raw audio bytes for testing";

    let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
    let file_id = insert_staged_audio(&pack, content).await;
    pack.save(|_, _| {}).await.unwrap();
    drop(pack);

    let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    let view = pack2.get_view().unwrap();
    let (data, _) = view.get_file_data(file_id).await.unwrap();
    assert_eq!(data.as_slice(), content.as_slice());
}

#[tokio::test]
async fn unsaved_recovery_prefers_dir_metadata() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "Original Name").await;
    pack.set_metadata(&Metadata {
        name: "Modified Name".to_string(),
        ..Default::default()
    })
    .await
    .unwrap();
    pack.save_metadata().await.unwrap();
    // Drop without calling save() — UNSAVED marker remains and dir is not cleaned up.
    drop(pack);

    let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    assert_eq!(pack2.name(), "Modified Name");
    assert!(!pack2.is_saved().await);
}

#[tokio::test]
async fn unsaved_recovery_tolerates_an_interrupted_in_place_archive_tail() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("interrupted-in-place.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "Interrupted in place").await;
    let bytes = b"media remains before the damaged database tail";
    let media_id = insert_staged_audio(&pack, bytes).await;
    pack.save(|_, _| {}).await.unwrap();
    pack.set_pack_data("unsaved", vec![1]).await.unwrap();

    let index_offset = pack.header.read().unwrap().index_offset;
    fs::OpenOptions::new()
        .write(true)
        .open(&pack_path)
        .unwrap()
        .set_len(index_offset)
        .unwrap();
    drop(pack);

    let reopened = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    assert!(!reopened.is_saved().await);
    assert_eq!(
        reopened
            .get_view()
            .unwrap()
            .get_file_data(media_id)
            .await
            .unwrap()
            .0,
        bytes
    );
}

#[tokio::test]
async fn unsaved_recovery_tolerates_a_readable_but_corrupt_archive_index() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("corrupt-in-place-index.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "Corrupt in-place index").await;
    let bytes = b"media remains valid before the overwritten database";
    let media_id = insert_staged_audio(&pack, bytes).await;
    pack.save(|_, _| {}).await.unwrap();
    pack.set_pack_data("unsaved", vec![1]).await.unwrap();

    let (index_offset, index_length) = {
        let header = pack.header.read().unwrap();
        (header.index_offset, header.index_length)
    };
    let mut archive = fs::OpenOptions::new().write(true).open(&pack_path).unwrap();
    archive.seek(SeekFrom::Start(index_offset)).unwrap();
    archive
        .write_all(&vec![0xa5; index_length as usize])
        .unwrap();
    archive.sync_data().unwrap();
    drop(pack);

    let reopened = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    assert!(!reopened.is_saved().await);
    assert_eq!(
        reopened
            .get_view()
            .unwrap()
            .get_file_data(media_id)
            .await
            .unwrap()
            .0,
        bytes
    );
}

#[tokio::test]
async fn damaged_metadata_backup_uses_the_authoritative_editor_database() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "Original Name").await;
    pack.save(|_, _| {}).await.unwrap();
    pack.set_metadata(&Metadata {
        name: "Modified Name".to_string(),
        ..Default::default()
    })
    .await
    .unwrap();
    tokio::fs::write(pack.dir().join("Metadata"), b"damaged")
        .await
        .unwrap();
    drop(pack);

    let recovered = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    assert_eq!(recovered.name(), "Modified Name");
    assert!(!recovered.is_saved().await);
}

#[tokio::test]
async fn staged_file_recoverable_before_save() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let content = b"critical data that must survive a crash";

    let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
    let file_id = insert_staged_audio(&pack, content).await;
    // Drop without saving — simulates crash before or during save.
    // The staging file and DB row (path-based) are intact.
    drop(pack);

    let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    let view = pack2.get_view().unwrap();
    let (data, _) = view.get_file_data(file_id).await.unwrap();
    assert_eq!(data.as_slice(), content.as_slice());
}

#[tokio::test]
async fn all_files_survive_multi_file_save_and_reopen() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let payloads: &[&[u8]] = &[b"file one", b"file two", b"file three"];

    let pack = new_test_pack(&pack_path, data_dir.path(), "Multi").await;
    let mut ids = Vec::new();
    for payload in payloads {
        ids.push(insert_staged_audio(&pack, payload).await);
    }
    pack.save(|_, _| {}).await.unwrap();
    drop(pack);

    let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    let view = pack2.get_view().unwrap();
    for (i, expected) in payloads.iter().enumerate() {
        let (data, _) = view.get_file_data(ids[i]).await.unwrap();
        assert_eq!(data.as_slice(), *expected);
    }
}

#[tokio::test]
async fn a_missing_loose_source_is_dropped_without_aborting_other_media() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("missing-loose.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "Missing loose").await;
    let missing = insert_staged_audio(&pack, b"will disappear").await;
    pack.set_source_url(missing, Some("https://example.com/missing".into()))
        .await
        .unwrap();
    let survivor_bytes = b"must still save";
    let survivor = insert_staged_audio(&pack, survivor_bytes).await;
    let missing_path: String = pack
        .db_execute(move |conn| {
            conn.query_row(
                "SELECT path FROM loose_media WHERE media_id = ?",
                [missing],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
        .await
        .unwrap();
    tokio::fs::remove_file(pack.dir().join("media").join(missing_path))
        .await
        .unwrap();

    pack.save(|_, _| {}).await.unwrap();
    let files = pack.get_files().await.unwrap();
    assert_eq!(
        files.iter().map(|file| file.id).collect::<Vec<_>>(),
        vec![survivor]
    );
    assert_eq!(
        pack.get_view()
            .unwrap()
            .get_file_data(survivor)
            .await
            .unwrap()
            .0,
        survivor_bytes
    );
    let retained: (bool, bool) = pack
        .db_execute(move |connection| {
            connection
                .query_row(
                    "SELECT m.deleted,
                            EXISTS(SELECT 1 FROM history_media_refs h WHERE h.media_id = m.id)
                     FROM media m WHERE m.id = ?",
                    [missing],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(Into::into)
        })
        .await
        .unwrap();
    assert_eq!(retained, (true, true));
}

// Exercises compact generation layout after scattered deletions. Varying
// content and lengths expose a bad run boundary, offset, or truncation.
#[tokio::test]
async fn deleting_files_then_resaving_preserves_surviving_content() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "Compaction").await;

    let contents: Vec<Vec<u8>> = (0..24u8)
        .map(|i| {
            let len = 200 + (i as usize) * 53;
            (0..len).map(|b| i.wrapping_add(b as u8)).collect()
        })
        .collect();

    let mut ids = Vec::new();
    for content in &contents {
        ids.push(insert_staged_audio(&pack, content).await);
    }

    // First save: embeds everything contiguously, no gaps yet.
    pack.save(|_, _| {}).await.unwrap();

    // Delete a scattered subset (near the start, middle, and end) so the
    // writer has to shift several contiguous runs by different amounts.
    let deleted_indices = [2usize, 3, 9, 15, 20];
    let deleted_ids: Vec<u64> = deleted_indices.iter().map(|&i| ids[i]).collect();
    pack.remove_files(deleted_ids).await.unwrap();

    // Second save creates a gapless replacement generation.
    pack.save(|_, _| {}).await.unwrap();
    let expected_index_offset = HEADER_SIZE as u64
        + contents
            .iter()
            .enumerate()
            .filter(|(index, _)| !deleted_indices.contains(index))
            .map(|(_, content)| content.len() as u64)
            .sum::<u64>();
    assert_eq!(
        pack.header.read().unwrap().index_offset,
        expected_index_offset,
        "generation saves must leave no gaps between embedded media"
    );
    drop(pack);

    let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    let view = pack2.get_view().unwrap();

    for (i, content) in contents.iter().enumerate() {
        if deleted_indices.contains(&i) {
            continue;
        }
        let (data, _) = view.get_file_data(ids[i]).await.unwrap();
        assert_eq!(
            &data, content,
            "content mismatch for surviving file index {i}"
        );
    }
}

// Exercises the sequential newly-staged-file layout: varying lengths expose
// any bad cumulative offset or truncation before the snapshot tail is written.
#[tokio::test]
async fn many_new_files_survive_sequential_first_save() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "NewFiles").await;

    let contents: Vec<Vec<u8>> = (0..40u8)
        .map(|i| {
            let len = 150 + (i as usize) * 41;
            (0..len).map(|b| i.wrapping_add(b as u8)).collect()
        })
        .collect();

    let mut ids = Vec::new();
    for content in &contents {
        ids.push(insert_staged_audio(&pack, content).await);
    }

    pack.save(|_, _| {}).await.unwrap();
    drop(pack);

    let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    let view = pack2.get_view().unwrap();

    for (i, content) in contents.iter().enumerate() {
        let (data, _) = view.get_file_data(ids[i]).await.unwrap();
        assert_eq!(&data, content, "content mismatch for new file index {i}");
    }
}

/// Manual reproduction for high-file-count generation-save performance without running the
/// encoder/import pipeline. Run with:
/// `cargo test -p lewdware-pack-editor profile_generation_save_many_staged_files -- --ignored --nocapture`
/// Override the defaults with `PACK_EDITOR_PERF_MEDIA_COUNT`,
/// `PACK_EDITOR_PERF_MEDIA_BYTES`, and `PACK_EDITOR_PERF_ROOT`.
#[tokio::test]
#[ignore = "manual save performance profile"]
async fn profile_generation_save_many_staged_files() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let count = std::env::var("PACK_EDITOR_PERF_MEDIA_COUNT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(25_000);
    let bytes_per_file = std::env::var("PACK_EDITOR_PERF_MEDIA_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(64 * 1024)
        .max(std::mem::size_of::<u64>());
    let total_media_bytes = count as u64 * bytes_per_file as u64;
    let perf_root = std::env::var_os("PACK_EDITOR_PERF_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap().join("target"));
    fs::create_dir_all(&perf_root).unwrap();
    // Do not use tempfile's default /tmp: it is commonly a tmpfs and would make the archive
    // sync timings and filesystem cleanup unrealistically cheap.
    let tmp = tempfile::tempdir_in(&perf_root).unwrap();
    let data_dir = tempfile::tempdir_in(&perf_root).unwrap();
    let pack_path = tmp.path().join("many-staged-files.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "Many staged files").await;
    // Establish a normal existing archive before adding the profiled files.
    pack.save(|_, _| {}).await.unwrap();

    let seed_started = Instant::now();
    let media_dir = pack.dir.join("media");
    pack.db_execute(move |mut connection| {
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut bytes = vec![0xa5; bytes_per_file];
        {
            let mut insert_media = tx.prepare(
                "INSERT INTO media (file_name, file_type, duration, hash)
                 VALUES (?, 'audio', 1.0, ?)",
            )?;
            let mut insert_loose =
                tx.prepare("INSERT INTO loose_media(media_id, path, length) VALUES (?, ?, ?)")?;
            for index in 0..count {
                let filename = format!("perf-{index:08}.opus");
                bytes[..std::mem::size_of::<u64>()].copy_from_slice(&(index as u64).to_le_bytes());
                fs::write(media_dir.join(&filename), &bytes)?;
                let mut hash = [0u8; 32];
                hash[..8].copy_from_slice(&(index as u64).to_le_bytes());
                insert_media.execute(params![filename, hash.as_slice()])?;
                let id = tx.last_insert_rowid();
                insert_loose.execute(params![id, filename, bytes.len() as u64])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await
    .unwrap();
    pack.mark_unsaved().await.unwrap();
    eprintln!(
        "seeded {count} staged files ({:.2} GiB total, {bytes_per_file} bytes each) in {:.3}s",
        total_media_bytes as f64 / 1024.0_f64.powi(3),
        seed_started.elapsed().as_secs_f64()
    );

    let progress_finished = Arc::new(Mutex::new(None::<Instant>));
    let progress_for_callback = progress_finished.clone();
    let save_started = Instant::now();
    pack.save(move |completed, total| {
        if total > 0 && completed == total {
            progress_for_callback
                .lock()
                .unwrap()
                .get_or_insert_with(Instant::now);
        }
    })
    .await
    .unwrap();
    let elapsed = save_started.elapsed();
    let after_progress = progress_finished
        .lock()
        .unwrap()
        .map(|finished| finished.elapsed())
        .unwrap_or_default();
    eprintln!(
        "saved {count} staged files ({:.2} GiB) in {:.3}s; {:.3}s elapsed after file progress completed",
        total_media_bytes as f64 / 1024.0_f64.powi(3),
        elapsed.as_secs_f64(),
        after_progress.as_secs_f64()
    );
    let garbage_collection_started = Instant::now();
    pack.process_pending_file_deletions().await.unwrap();
    eprintln!(
        "background loose-file garbage collection completed in {:.3}s",
        garbage_collection_started.elapsed().as_secs_f64()
    );
    pack.close().await;
}

// Diagnostic stress test: many files, then a large scattered deletion, then a
// re-save - matching the scale where a "save spinner freezes near the end"
// report came from. Wrapped in a timeout so a hang fails the test instead of
// hanging the suite.
#[tokio::test]
async fn stress_large_pack_resave_does_not_hang() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "Stress").await;

    let n = 3065usize;
    let mut ids = Vec::new();
    for i in 0..n {
        let content = vec![(i % 256) as u8; 40 + (i % 50)];
        ids.push(insert_staged_audio(&pack, &content).await);
    }

    tokio::time::timeout(std::time::Duration::from_secs(30), pack.save(|_, _| {}))
        .await
        .expect("first save timed out (hang)")
        .unwrap();

    // Scatter deletions every 7th file, matching the "delete scattered files"
    // scenario the parallel gap-closing loop is meant for.
    let deleted: Vec<u64> = (0..n).step_by(7).map(|i| ids[i]).collect();
    pack.remove_files(deleted).await.unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(30), pack.save(|_, _| {}))
        .await
        .expect("second save timed out (hang)")
        .unwrap();
}

// Same idea but with bigger per-file content (to stretch out per-job I/O
// duration and widen any race window) and repeated save/delete cycles (to
// surface anything that only shows up after the parallel machinery has run
// several times in the same process).
#[tokio::test]
async fn stress_repeated_cycles_with_larger_files_does_not_hang() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "StressBig").await;

    let n = 600usize;
    let mut ids = Vec::new();
    for i in 0..n {
        // 1KB to ~1.5MB, varying, to widen per-job I/O duration.
        let len = 1024 + (i * 2503) % (1536 * 1024);
        let content = vec![(i % 256) as u8; len];
        ids.push(insert_staged_audio(&pack, &content).await);
    }

    for cycle in 0..5 {
        tokio::time::timeout(std::time::Duration::from_secs(60), pack.save(|_, _| {}))
            .await
            .unwrap_or_else(|_| panic!("save timed out (hang) on cycle {cycle}"))
            .unwrap();

        // Delete a scattered ~10% of whatever remains.
        let remaining = tokio::time::timeout(std::time::Duration::from_secs(10), pack.get_files())
            .await
            .unwrap()
            .unwrap();
        if remaining.len() < 20 {
            break;
        }
        let to_delete: Vec<u64> = remaining.iter().step_by(9).map(|f| f.id).collect();
        pack.remove_files(to_delete).await.unwrap();
    }
    let _ = ids;
}

#[tokio::test]
async fn referenced_loose_file_survives_pack_drop_and_orphans_are_swept_on_reopen() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("leases.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "Leases").await;

    let encoded_path = pack.dir.join("media").join("referenced.opus");
    tokio::fs::write(&encoded_path, b"referenced")
        .await
        .unwrap();
    let media = pack
        .add_file(
            EncodedFile {
                info: FileInfo::Audio { duration: 1.0 },
                thumbnail: None,
                path: encoded_path.clone(),
                artists: vec![],
                source_url: None,
            },
            Path::new("referenced.wav"),
            blake3::hash(b"referenced"),
        )
        .await
        .unwrap()
        .unwrap();

    let orphan_path = pack.dir.join("media").join("orphan.opus");
    tokio::fs::write(&orphan_path, b"orphan").await.unwrap();
    let abandoned_staging = pack.dir.join("pack-editor-upload-abandoned");
    tokio::fs::create_dir(&abandoned_staging).await.unwrap();
    tokio::fs::write(abandoned_staging.join("partial.opus"), b"partial")
        .await
        .unwrap();

    drop(pack);
    assert!(
        encoded_path.exists(),
        "a durable live row must survive application teardown"
    );

    let reopened = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    assert!(encoded_path.exists());
    assert!(!orphan_path.exists());
    assert!(!abandoned_staging.exists());
    assert_eq!(reopened.get_files().await.unwrap()[0].id, media.id);
    reopened.remove_files(vec![media.id]).await.unwrap();
    assert!(
        encoded_path.exists(),
        "undo history must retain the loose file"
    );
    reopened
        .db_execute(|conn| {
            conn.execute("DELETE FROM history_entries", [])?;
            Ok(())
        })
        .await
        .unwrap();
    reopened.purge_history_files(vec![media.id]).await.unwrap();
    assert!(
        !encoded_path.exists(),
        "history eviction must release the final durable owner"
    );
}
