use anyhow::{bail, Result};
use std::{path::Path, time::Duration};

use rusqlite::{params, Connection, OptionalExtension};
use shared::read_pack::Metadata;
use uuid::Uuid;

const ARCHIVE_GENERATION_KEY: &str = "__pack_editor_archive_generation";
const ARCHIVE_STATE_KEY: &str = "__pack_editor_archive_state";

/// One migration, because nothing is released yet: a database is either current or absent, so
/// there is no upgrade path worth carrying. The ledger stays a list so that the first real
/// migration is an append rather than a rewrite of `initialize`.
///
/// Two files concatenated: the editor's own base schema, and the behaviour tables. The latter is
/// reached across into `shared` on purpose -- they are identical in the runtime pack and the
/// editor's working copy, and two hand-kept copies of that schema drifting apart would be a silent
/// corruption rather than a compile error.
const MIGRATION_1: &str = concat!(
    include_str!("migrations/0001_init_schema.sql"),
    include_str!("../../../shared/src/migrations/behaviour_schema.sql"),
);

const MIGRATIONS: &[&str] = &[MIGRATION_1];

/// The behaviour document's structural half (see `shared::behaviour::storage`), in an order that
/// satisfies its foreign keys: parents before children, stages before the transitions between them.
///
/// One list rather than three hand-written copies, because a table present in the export but
/// missing from an import is a pack that loses its timeline on reopen -- and that is exactly the
/// kind of thing three parallel SQL blocks drift into.
pub const BEHAVIOUR_TABLES: &[&str] = &[
    "behaviour_content",
    "behaviour_experience",
    "behaviour_stage",
    "behaviour_stage_tag",
    "behaviour_stage_end",
    "behaviour_stage_movement",
    "behaviour_stage_mitosis",
    "behaviour_stage_entry",
    "behaviour_stage_prompt",
    "behaviour_stage_event",
    "behaviour_transition",
    "behaviour_transition_category",
    "behaviour_text_item",
    "behaviour_text_item_tag",
    "behaviour_web_link",
    "behaviour_web_link_arg",
    "behaviour_web_link_tag",
    "behaviour_content_group",
    "behaviour_content_group_tag",
    "behaviour_popup_media",
    "behaviour_popup_audio_pair",
    "behaviour_audio_media",
];

/// The behaviour tables keyed by a media id, and which of their columns hold one.
///
/// They need a filter the rest do not. The editor *soft*-deletes media (`media.deleted`), so undo
/// can bring a file back, which means its `media` table holds rows the exported pack will not --
/// and an attribute row for one of them would land in the pack as a foreign-key violation. The
/// filter is expressed against the *destination* rather than against `deleted`, so the same SQL
/// serves both directions: every call site populates `main.media` before copying these, so "the
/// media actually made it across" is exactly the right question in each. The runtime pack has no
/// `deleted` column to ask about anyway.
const MEDIA_KEYED_BEHAVIOUR_TABLES: &[(&str, &[&str])] = &[
    ("behaviour_popup_media", &["media_id"]),
    (
        "behaviour_popup_audio_pair",
        &["popup_media_id", "audio_media_id"],
    ),
    ("behaviour_audio_media", &["media_id"]),
];

/// Copies every behaviour table across from the attached database `source`.
///
/// `behaviour_content` is a singleton the migration seeds, so it is replaced rather than inserted.
fn copy_behaviour_tables(tx: &rusqlite::Transaction<'_>, source: &str) -> Result<()> {
    for table in BEHAVIOUR_TABLES {
        let media_keys = MEDIA_KEYED_BEHAVIOUR_TABLES
            .iter()
            .find(|(name, _)| name == table)
            .map(|(_, columns)| *columns);
        let filter = match media_keys {
            None => String::new(),
            Some(columns) => {
                let conditions: Vec<String> = columns
                    .iter()
                    .map(|column| format!("t.{column} IN (SELECT id FROM main.media)"))
                    .collect();
                format!(" WHERE {}", conditions.join(" AND "))
            }
        };
        tx.execute(
            &format!("INSERT OR REPLACE INTO {table} SELECT t.* FROM {source}.{table} t{filter}"),
            [],
        )?;
    }
    Ok(())
}

/// Empties every behaviour table, for a copy that replaces the whole database.
fn clear_behaviour_tables(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    // Reverse order: children before the parents they cascade from, so nothing is deleted twice.
    for table in BEHAVIOUR_TABLES.iter().rev() {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    }
    Ok(())
}

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
        copy_behaviour_tables(&tx, "runtime")?;
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
        clear_behaviour_tables(&tx)?;
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
        copy_behaviour_tables(&tx, "runtime")?;
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
        copy_behaviour_tables(&tx, "editor")?;
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
    fn initialize_is_idempotent() {
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

    /// The editor soft-deletes media so undo can bring a file back, which means its `media` table
    /// holds rows the exported pack will not. A per-item attribute row for one of those would land
    /// in the pack as a dangling foreign key -- a corrupt pack, produced by a successful save.
    #[test]
    fn exporting_drops_attributes_for_media_the_pack_will_not_contain() {
        // File-backed, not in-memory: `export_runtime` attaches the editor database by path.
        let directory = tempfile::tempdir().unwrap();
        let editor = Connection::open(directory.path().join("editor.db")).unwrap();
        configure_connection(&editor).unwrap();
        initialize_empty(
            &editor,
            &Metadata {
                name: "Test pack".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        editor
            .execute_batch(
                "INSERT INTO media (id, file_name, file_type, hash, deleted)
                 VALUES (1, 'kept.png', 'image', x'00', 0),
                        (2, 'removed.png', 'image', x'01', 1),
                        (3, 'kept.ogg', 'audio', x'02', 0);
                 INSERT INTO pack_media (media_id, generation_id, \"offset\", length)
                 VALUES (1, 'g', 0, 0), (2, 'g', 0, 0), (3, 'g', 0, 0);
                 INSERT INTO behaviour_popup_media (media_id, scale)
                 VALUES (1, 2.0), (2, 3.0);
                 INSERT INTO behaviour_audio_media (media_id, volume) VALUES (3, 0.5);
                 -- A pairing from a surviving popup to a file that is going away, and one that
                 -- stays: the pair table is filtered on both of its media columns.
                 INSERT INTO behaviour_popup_audio_pair (popup_media_id, audio_media_id)
                 VALUES (1, 3);
                 -- Stands in for the import/save that would normally have set this; the export
                 -- refuses a snapshot with no archive generation.
                 UPDATE editor_state SET archive_generation = 'g' WHERE singleton = 1;",
            )
            .unwrap();

        let runtime_path = directory.path().join("pack.lwpack");
        export_runtime(&editor, &runtime_path).unwrap();

        let runtime = Connection::open(&runtime_path).unwrap();
        configure_connection(&runtime).unwrap();

        let popup_ids: Vec<u64> = runtime
            .prepare("SELECT media_id FROM behaviour_popup_media ORDER BY media_id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            popup_ids,
            vec![1],
            "the deleted file's attributes stay behind"
        );

        let audio_ids: Vec<u64> = runtime
            .prepare("SELECT media_id FROM behaviour_audio_media")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(audio_ids, vec![3]);

        let pairs: u32 = runtime
            .query_row(
                "SELECT COUNT(*) FROM behaviour_popup_audio_pair",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pairs, 1, "the surviving pairing came across");

        // The check that would have caught this whatever the counts said.
        assert!(
            runtime
                .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
                .optional()
                .unwrap()
                .is_none(),
            "the exported pack must have no dangling references",
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

        assert!(initialize(&connection)
            .unwrap_err()
            .to_string()
            .contains("newer"));
    }
}
