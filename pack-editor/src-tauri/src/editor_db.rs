use anyhow::{Result, bail};
use std::{path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, params};
use shared::read_pack::Metadata;
use uuid::Uuid;

const ARCHIVE_GENERATION_KEY: &str = "__pack_editor_archive_generation";
const ARCHIVE_STATE_KEY: &str = "__pack_editor_archive_state";

const MIGRATION_1: &str = r#"
CREATE TABLE IF NOT EXISTS editor_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    archive_generation TEXT,
    current_state_id TEXT NOT NULL,
    saved_state_id TEXT
) STRICT;
CREATE TABLE IF NOT EXISTS pack_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    metadata BLOB NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS media (
    id INTEGER PRIMARY KEY,
    file_name TEXT NOT NULL UNIQUE,
    file_type TEXT CHECK (file_type IN ('image', 'video', 'audio')) NOT NULL,
    width INTEGER,
    height INTEGER,
    transparent INTEGER,
    duration REAL,
    audio INTEGER,
    hash BLOB NOT NULL UNIQUE,
    thumbnail BLOB,
    source_url TEXT,
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1))
) STRICT;
CREATE INDEX IF NOT EXISTS media_hash_index ON media (hash);

CREATE TABLE IF NOT EXISTS pack_media (
    media_id INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    generation_id TEXT NOT NULL,
    "offset" INTEGER NOT NULL,
    length INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS loose_media (
    media_id INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    path TEXT NOT NULL UNIQUE,
    length INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY,
    name TEXT UNIQUE NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS media_tags (
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (media_id, tag_id)
) STRICT;

CREATE TABLE IF NOT EXISTS artists (
    id INTEGER PRIMARY KEY,
    name TEXT UNIQUE NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS media_artists (
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
    PRIMARY KEY (media_id, artist_id)
) STRICT;

CREATE TABLE IF NOT EXISTS modes (
    id INTEGER PRIMARY KEY,
    file BLOB NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS pack_data (
    name TEXT PRIMARY KEY,
    blob BLOB NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS history_entries (
    id INTEGER PRIMARY KEY,
    sequence INTEGER NOT NULL UNIQUE,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,
    forward_changeset BLOB,
    inverse_changeset BLOB,
    payload TEXT,
    storage_bytes INTEGER NOT NULL DEFAULT 0,
    before_state_id TEXT NOT NULL,
    after_state_id TEXT,
    status TEXT NOT NULL DEFAULT 'ready' CHECK (status IN ('pending', 'ready'))
) STRICT;
CREATE TABLE IF NOT EXISTS history_media_refs (
    history_id INTEGER NOT NULL REFERENCES history_entries(id) ON DELETE CASCADE,
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    PRIMARY KEY (history_id, media_id)
) STRICT;
CREATE INDEX IF NOT EXISTS history_media_refs_media_index ON history_media_refs(media_id);
CREATE TABLE IF NOT EXISTS history_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    cursor INTEGER NOT NULL DEFAULT 0
) STRICT;
INSERT OR IGNORE INTO history_state(singleton, cursor) VALUES (1, 0);

CREATE TABLE IF NOT EXISTS save_sessions (
    id TEXT PRIMARY KEY,
    generation_id TEXT NOT NULL,
    editor_state_id TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS snapshot_media (
    save_id TEXT NOT NULL REFERENCES save_sessions(id) ON DELETE CASCADE,
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    PRIMARY KEY (save_id, media_id)
) STRICT;
CREATE INDEX IF NOT EXISTS snapshot_media_media_index ON snapshot_media(media_id);

CREATE TABLE IF NOT EXISTS pending_file_deletions (
    path TEXT PRIMARY KEY
) STRICT;
"#;

const MIGRATIONS: &[&str] = &[MIGRATION_1];

fn configure_connection(connection: &Connection) -> Result<()> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.busy_timeout(Duration::from_millis(5000))?;
    Ok(())
}

fn dedicated_editor_connection(pooled: &Connection) -> Result<Connection> {
    let path = pooled
        .path()
        .ok_or_else(|| anyhow::anyhow!("editor database has no filesystem path"))?;
    let connection = Connection::open(path)?;
    configure_connection(&connection)?;
    Ok(connection)
}

fn detach_result<T>(result: Result<T>, detach: rusqlite::Result<usize>) -> Result<T> {
    match result {
        Ok(value) => {
            detach?;
            Ok(value)
        }
        Err(error) => {
            if let Err(detach_error) = detach {
                tracing::error!("could not detach database after operation failed: {detach_error}");
            }
            Err(error)
        }
    }
}

pub fn is_corrupt_database_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqliteFailure(sqlite, _))
                if matches!(
                    sqlite.code,
                    rusqlite::ffi::ErrorCode::DatabaseCorrupt
                        | rusqlite::ffi::ErrorCode::NotADatabase
                )
        )
    })
}

pub fn initialize(connection: &Connection) -> Result<()> {
    configure_connection(connection)?;
    let tx = connection.unchecked_transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS editor_migrations (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            migration_index INTEGER NOT NULL
         ) STRICT;",
    )?;
    let migration_index = tx
        .query_row(
            "SELECT migration_index FROM editor_migrations WHERE singleton = 1",
            [],
            |row| row.get::<_, usize>(0),
        )
        .optional()?
        .unwrap_or(0);
    if migration_index > MIGRATIONS.len() {
        bail!(
            "editor database schema {migration_index} is newer than supported schema {}",
            MIGRATIONS.len()
        );
    }
    for migration in &MIGRATIONS[migration_index..] {
        tx.execute_batch(migration)?;
    }
    tx.execute(
        "INSERT INTO editor_migrations(singleton, migration_index) VALUES (1, ?)
         ON CONFLICT(singleton) DO UPDATE SET migration_index = excluded.migration_index",
        [MIGRATIONS.len()],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn initialize_empty(connection: &Connection, metadata: &Metadata) -> Result<()> {
    initialize(connection)?;
    let state = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO editor_state
         (singleton, archive_generation, current_state_id, saved_state_id)
         VALUES (1, NULL, ?, NULL)",
        [state],
    )?;
    connection.execute(
        "INSERT INTO pack_metadata(singleton, metadata) VALUES (1, ?)",
        [metadata.to_buf()?],
    )?;
    Ok(())
}

pub fn import_runtime(
    pooled: &mut Connection,
    runtime_path: &Path,
    metadata: &Metadata,
) -> Result<String> {
    initialize(pooled)?;
    let mut connection = dedicated_editor_connection(pooled)?;
    let generation = runtime_marker(runtime_path, ARCHIVE_GENERATION_KEY)?
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let state = Uuid::new_v4().to_string();
    connection.execute(
        "ATTACH DATABASE ? AS runtime",
        [runtime_path.to_string_lossy().as_ref()],
    )?;
    let result = (|| -> Result<()> {
        let tx = connection.transaction()?;
        tx.execute(
            "INSERT INTO editor_state
             (singleton, archive_generation, current_state_id, saved_state_id)
             VALUES (1, ?, ?, ?)",
            params![generation, state, state],
        )?;
        tx.execute(
            "INSERT INTO pack_metadata(singleton, metadata) VALUES (1, ?)",
            [metadata.to_buf()?],
        )?;
        tx.execute(
            "INSERT INTO media
             (id, file_name, file_type, width, height, transparent, duration, audio,
              hash, thumbnail, source_url, deleted)
             SELECT id, file_name, file_type, width, height, transparent, duration, audio,
                    hash, thumbnail, source_url, 0
             FROM runtime.media",
            [],
        )?;
        tx.execute(
            "INSERT INTO pack_media(media_id, generation_id, \"offset\", length)
             SELECT id, ?, \"offset\", length FROM runtime.media",
            [&generation],
        )?;
        tx.execute("INSERT INTO tags SELECT * FROM runtime.tags", [])?;
        tx.execute(
            "INSERT INTO media_tags SELECT * FROM runtime.media_tags",
            [],
        )?;
        tx.execute("INSERT INTO artists SELECT * FROM runtime.artists", [])?;
        tx.execute(
            "INSERT INTO media_artists SELECT * FROM runtime.media_artists",
            [],
        )?;
        tx.execute("INSERT INTO modes SELECT * FROM runtime.modes", [])?;
        tx.execute("INSERT INTO pack_data SELECT * FROM runtime.pack_data", [])?;
        tx.commit()?;
        Ok(())
    })();
    detach_result(result, connection.execute("DETACH DATABASE runtime", []))?;
    Ok(generation)
}

pub fn replace_from_runtime(
    pooled: &mut Connection,
    runtime_path: &Path,
    metadata: &Metadata,
) -> Result<String> {
    let mut connection = dedicated_editor_connection(pooled)?;
    let generation = runtime_marker(runtime_path, ARCHIVE_GENERATION_KEY)?
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let state = Uuid::new_v4().to_string();
    connection.execute(
        "ATTACH DATABASE ? AS runtime",
        [runtime_path.to_string_lossy().as_ref()],
    )?;
    let result = (|| -> Result<()> {
        let tx = connection.transaction()?;
        tx.execute_batch(
            "DELETE FROM save_sessions;
             DELETE FROM history_entries;
             DELETE FROM media;
             DELETE FROM tags;
             DELETE FROM artists;
             DELETE FROM modes;
             DELETE FROM pack_data;",
        )?;
        tx.execute(
            "UPDATE editor_state SET archive_generation = ?, current_state_id = ?,
             saved_state_id = ? WHERE singleton = 1",
            params![generation, state, state],
        )?;
        tx.execute(
            "UPDATE pack_metadata SET metadata = ? WHERE singleton = 1",
            [metadata.to_buf()?],
        )?;
        tx.execute(
            "INSERT INTO media
             (id, file_name, file_type, width, height, transparent, duration, audio,
              hash, thumbnail, source_url, deleted)
             SELECT id, file_name, file_type, width, height, transparent, duration, audio,
                    hash, thumbnail, source_url, 0 FROM runtime.media",
            [],
        )?;
        tx.execute(
            "INSERT INTO pack_media(media_id, generation_id, \"offset\", length)
             SELECT id, ?, \"offset\", length FROM runtime.media",
            [&generation],
        )?;
        tx.execute("INSERT INTO tags SELECT * FROM runtime.tags", [])?;
        tx.execute(
            "INSERT INTO media_tags SELECT * FROM runtime.media_tags",
            [],
        )?;
        tx.execute("INSERT INTO artists SELECT * FROM runtime.artists", [])?;
        tx.execute(
            "INSERT INTO media_artists SELECT * FROM runtime.media_artists",
            [],
        )?;
        tx.execute("INSERT INTO modes SELECT * FROM runtime.modes", [])?;
        tx.execute("INSERT INTO pack_data SELECT * FROM runtime.pack_data", [])?;
        tx.commit()?;
        Ok(())
    })();
    detach_result(result, connection.execute("DETACH DATABASE runtime", []))?;
    Ok(generation)
}

pub fn export_runtime(editor: &Connection, runtime_path: &Path) -> Result<()> {
    let _ = std::fs::remove_file(runtime_path);
    let mut runtime = Connection::open(runtime_path)?;
    configure_connection(&runtime)?;
    shared::db::migrate(&runtime)?;
    runtime.execute("ATTACH DATABASE ? AS editor", [editor.path().unwrap()])?;
    let result = (|| -> Result<()> {
        let tx = runtime.transaction()?;
        tx.execute(
            "INSERT INTO media
             (id, file_name, file_type, \"offset\", length, width, height,
              transparent, duration, audio, hash, thumbnail, source_url)
             SELECT m.id, m.file_name, m.file_type, p.\"offset\", p.length,
                    m.width, m.height, m.transparent, m.duration, m.audio, m.hash,
                    m.thumbnail, m.source_url
             FROM editor.media m
             JOIN editor.pack_media p ON p.media_id = m.id
             WHERE m.deleted = 0",
            [],
        )?;
        tx.execute("INSERT INTO tags SELECT * FROM editor.tags", [])?;
        tx.execute(
            "INSERT INTO media_tags
             SELECT mt.* FROM editor.media_tags mt
             JOIN editor.media m ON m.id = mt.media_id WHERE m.deleted = 0",
            [],
        )?;
        tx.execute("INSERT INTO artists SELECT * FROM editor.artists", [])?;
        tx.execute(
            "INSERT INTO media_artists
             SELECT ma.* FROM editor.media_artists ma
             JOIN editor.media m ON m.id = ma.media_id WHERE m.deleted = 0",
            [],
        )?;
        tx.execute("INSERT INTO modes SELECT * FROM editor.modes", [])?;
        tx.execute(
            "INSERT INTO pack_data SELECT * FROM editor.pack_data
             WHERE name NOT IN (?, ?)",
            params![ARCHIVE_GENERATION_KEY, ARCHIVE_STATE_KEY],
        )?;
        let (generation, state): (Option<String>, String) = tx.query_row(
            "SELECT archive_generation, current_state_id FROM editor.editor_state
             WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let generation =
            generation.ok_or_else(|| anyhow::anyhow!("snapshot has no archive generation"))?;
        tx.execute(
            "INSERT INTO pack_data(name, blob) VALUES (?, ?)",
            params![ARCHIVE_GENERATION_KEY, generation.as_bytes()],
        )?;
        tx.execute(
            "INSERT INTO pack_data(name, blob) VALUES (?, ?)",
            params![ARCHIVE_STATE_KEY, state.as_bytes()],
        )?;
        tx.commit()?;
        Ok(())
    })();
    detach_result(result, runtime.execute("DETACH DATABASE editor", []))?;
    runtime.execute_batch("VACUUM")?;
    Ok(())
}

pub fn reconcile_archive(connection: &mut Connection, runtime_path: &Path) -> Result<()> {
    let mut dedicated = dedicated_editor_connection(connection)?;
    let generation = runtime_marker(runtime_path, ARCHIVE_GENERATION_KEY)?
        .ok_or_else(|| anyhow::anyhow!("pack has no archive generation marker"))?;
    let archived_state = runtime_marker(runtime_path, ARCHIVE_STATE_KEY)?;
    dedicated.execute(
        "ATTACH DATABASE ? AS runtime",
        [runtime_path.to_string_lossy().as_ref()],
    )?;
    let result = (|| -> Result<()> {
        let tx = dedicated.transaction()?;
        let current_generation: Option<String> = tx.query_row(
            "SELECT archive_generation FROM editor_state WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        if current_generation.as_deref() != Some(&generation) {
            tx.execute("DELETE FROM pack_media", [])?;
            tx.execute(
                "INSERT INTO pack_media(media_id, generation_id, \"offset\", length)
                 SELECT m.id, ?, r.\"offset\", r.length
                 FROM media m JOIN runtime.media r ON r.id = m.id AND r.hash = m.hash",
                [&generation],
            )?;
        }
        tx.execute(
            "UPDATE editor_state SET archive_generation = ?,
             saved_state_id = COALESCE(?, saved_state_id) WHERE singleton = 1",
            params![generation, archived_state],
        )?;
        // Save sessions cannot survive a process exit. Cascading this delete releases all
        // abandoned snapshot reachability before the normal loose-file sweep runs.
        tx.execute("DELETE FROM save_sessions", [])?;
        tx.commit()?;
        Ok(())
    })();
    detach_result(result, dedicated.execute("DETACH DATABASE runtime", []))
}

pub fn reset_session(connection: &mut Connection, dirty: bool) -> Result<()> {
    let tx = connection.transaction()?;
    tx.execute("DELETE FROM history_entries", [])?;
    tx.execute(
        "UPDATE history_state SET cursor = 0 WHERE singleton = 1",
        [],
    )?;
    tx.execute("DELETE FROM save_sessions", [])?;
    tx.execute(
        "INSERT OR IGNORE INTO pending_file_deletions(path)
         SELECT l.path FROM loose_media l JOIN media m ON m.id = l.media_id
         WHERE m.deleted = 1",
        [],
    )?;
    tx.execute("DELETE FROM media WHERE deleted = 1", [])?;
    if dirty {
        tx.execute(
            "UPDATE editor_state SET current_state_id = ? WHERE singleton = 1",
            [Uuid::new_v4().to_string()],
        )?;
    } else {
        tx.execute(
            "UPDATE editor_state SET current_state_id = saved_state_id WHERE singleton = 1",
            [],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn runtime_marker(path: &Path, key: &str) -> Result<Option<String>> {
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    configure_connection(&connection)?;
    let value: Option<Vec<u8>> = connection
        .query_row("SELECT blob FROM pack_data WHERE name = ?", [key], |row| {
            row.get(0)
        })
        .optional()?;
    value
        .map(|value| String::from_utf8(value).map_err(Into::into))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_adopts_an_existing_pre_migration_editor_database() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(MIGRATION_1).unwrap();

        initialize(&connection).unwrap();
        initialize(&connection).unwrap();

        assert_eq!(
            connection
                .query_row(
                    "SELECT migration_index FROM editor_migrations WHERE singleton = 1",
                    [],
                    |row| row.get::<_, usize>(0),
                )
                .unwrap(),
            MIGRATIONS.len()
        );
    }

    #[test]
    fn initialize_rejects_a_database_from_a_newer_editor() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE editor_migrations (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    migration_index INTEGER NOT NULL
                 ) STRICT;
                 INSERT INTO editor_migrations VALUES (1, 99);",
            )
            .unwrap();

        assert!(
            initialize(&connection)
                .unwrap_err()
                .to_string()
                .contains("newer")
        );
    }
}
