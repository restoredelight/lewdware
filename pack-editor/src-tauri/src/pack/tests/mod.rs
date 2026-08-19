mod behaviour;
mod labels;
mod media;
mod save_property;
mod saving;
mod view;

use rusqlite::params;
use shared::behaviour::Behaviour;

use super::*;
use rusqlite::TransactionBehavior;

pub(super) async fn new_test_pack(pack_path: &Path, data_dir: &Path, name: &str) -> MediaPack {
    MediaPack::new(pack_path.to_path_buf(), data_dir, name)
        .await
        .unwrap()
}

// Insert a minimal audio row backed by a staging file in dir/media/. Reuses the (already
// unique, UUID-based) staged file name as the DB file_name too, since callers routinely
// insert several of these against the same pack and `media.file_name` is UNIQUE.
// Returns the assigned media id.
pub(super) async fn insert_staged_audio(pack: &MediaPack, content: &[u8]) -> u64 {
    let filename = Uuid::new_v4().to_string();
    tokio::fs::write(pack.dir.join("media").join(&filename), content)
        .await
        .unwrap();

    let hash_bytes = *blake3::hash(content).as_bytes();
    let filename_clone = filename.clone();
    let length = content.len() as u64;
    pack.db_execute(move |mut conn| {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id: u64 = tx.query_row(
            "INSERT INTO media (file_name, file_type, duration, hash) \
             VALUES (?, 'audio', 1.0, ?) RETURNING id",
            params![filename_clone, hash_bytes],
            |row| row.get("id"),
        )?;
        tx.execute(
            "INSERT INTO loose_media(media_id, path, length) VALUES (?, ?, ?)",
            params![id, filename, length],
        )?;
        tx.commit()?;
        Ok(id)
    })
    .await
    .unwrap()
}

pub(super) async fn read_pack_behaviour(pack: &MediaPack) -> Behaviour {
    pack.get_behaviour().await.unwrap()
}

/// Like `insert_staged_audio`, but every call is guaranteed a unique hash and file name (the
/// `media` table has a UNIQUE constraint on both) by appending a never-repeated counter to
/// the content, and reusing the already-unique on-disk staged file name (a fresh UUID) as the
/// DB `file_name` too, regardless of what content the caller passes in -- proptest-generated
/// byte vectors are short and likely to collide otherwise.
pub(super) async fn add_staged_file(
    pack: &MediaPack,
    content: &[u8],
    unique: u64,
) -> (u64, Vec<u8>) {
    let mut content = content.to_vec();
    content.extend_from_slice(&unique.to_le_bytes());

    let filename = Uuid::new_v4().to_string();
    tokio::fs::write(pack.dir.join("media").join(&filename), &content)
        .await
        .unwrap();

    let hash_bytes = *blake3::hash(&content).as_bytes();
    let length = content.len() as u64;
    let filename_clone = filename.clone();
    let id = pack
        .db_execute(move |mut conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let id: u64 = tx.query_row(
                "INSERT INTO media (file_name, file_type, duration, hash) \
                 VALUES (?, 'audio', 1.0, ?) RETURNING id",
                params![filename_clone, hash_bytes],
                |row| row.get("id"),
            )?;
            tx.execute(
                "INSERT INTO loose_media(media_id, path, length) VALUES (?, ?, ?)",
                params![id, filename, length],
            )?;
            tx.commit()?;
            Ok(id)
        })
        .await
        .unwrap();

    (id, content)
}

/// Reads back a surviving file's current on-disk bytes -- whether it's still a loose staged
/// file (`path` set) or already compacted into the pack file (`offset`/`length` set) -- via
/// the same code path the app's file preview/download uses.
pub(super) async fn read_back(pack: &MediaPack, id: u64) -> Vec<u8> {
    pack.get_view().unwrap().get_file_data(id).await.unwrap().0
}
