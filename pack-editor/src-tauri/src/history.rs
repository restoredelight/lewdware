use std::{io::Cursor, rc::Rc};

use anyhow::{Result, bail};
use rusqlite::{
    Connection, OptionalExtension, Transaction, TransactionBehavior, params,
    session::{ConflictAction, Session, invert_strm},
    types::Value,
    vtab::array::Array,
};
use serde::Serialize;
use uuid::Uuid;

const TRACKED_TABLES: &[&str] = &[
    "media",
    "tags",
    "media_tags",
    "artists",
    "media_artists",
    "modes",
    "pack_data",
    "pack_metadata",
];

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub at_saved_state: bool,
}

pub fn record<T>(
    connection: &mut Connection,
    label: &str,
    storage_bytes: u64,
    media_refs: &[u64],
    operation: impl FnOnce(&Transaction<'_>) -> Result<T>,
) -> Result<T> {
    record_with_media_refs(connection, label, storage_bytes, |tx| {
        Ok((operation(tx)?, media_refs.to_vec()))
    })
}

/// Records an operation whose affected media IDs can only be known once its write transaction
/// has begun. This keeps association-wide tag/artist edits and their reachability references at
/// one precise database state.
pub fn record_with_media_refs<T>(
    connection: &mut Connection,
    label: &str,
    storage_bytes: u64,
    operation: impl FnOnce(&Transaction<'_>) -> Result<(T, Vec<u64>)>,
) -> Result<T> {
    let mut session = Session::new(connection)?;
    session.set_enabled(false);
    for table in TRACKED_TABLES {
        session.attach(Some(*table))?;
    }

    let tx = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let cursor: i64 = tx.query_row(
        "SELECT cursor FROM history_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    tx.execute("DELETE FROM history_entries WHERE sequence >= ?", [cursor])?;
    collect_garbage(&tx)?;
    let before_state: String = tx.query_row(
        "SELECT current_state_id FROM editor_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;

    // Redo-branch disposal and its garbage collection are bookkeeping for the new branch,
    // not part of the user's command. Recording them would make undo resurrect garbage rows
    // after their loose sources had already been released.
    session.set_enabled(true);
    let (value, media_refs) = operation(&tx)?;
    let mut forward = Vec::new();
    session.changeset_strm(&mut forward)?;
    if forward.is_empty() {
        tx.commit()?;
        return Ok(value);
    }
    let mut inverse = Vec::new();
    invert_strm(&mut Cursor::new(&forward), &mut inverse)?;
    let after_state = Uuid::new_v4().to_string();
    let history_id: i64 = tx.query_row(
        "INSERT INTO history_entries
         (sequence, label, kind, forward_changeset, inverse_changeset, payload,
          storage_bytes, before_state_id, after_state_id, status)
         VALUES (?, ?, 'changeset', ?, ?, NULL, ?, ?, ?, 'ready') RETURNING id",
        params![
            cursor,
            label,
            forward,
            inverse,
            storage_bytes,
            before_state,
            after_state
        ],
        |row| row.get(0),
    )?;
    for media_id in media_refs {
        tx.execute(
            "INSERT OR IGNORE INTO history_media_refs(history_id, media_id) VALUES (?, ?)",
            params![history_id, media_id],
        )?;
    }
    tx.execute(
        "UPDATE history_state SET cursor = ? WHERE singleton = 1",
        [cursor + 1],
    )?;
    tx.execute(
        "UPDATE editor_state SET current_state_id = ? WHERE singleton = 1",
        [&after_state],
    )?;
    trim(&tx)?;
    collect_garbage(&tx)?;
    tx.commit()?;
    Ok(value)
}

pub fn undo(connection: &mut Connection) -> Result<Status> {
    apply_at_cursor(connection, false)
}

pub fn redo(connection: &mut Connection) -> Result<Status> {
    apply_at_cursor(connection, true)
}

pub fn begin_media_import(connection: &mut Connection) -> Result<i64> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let cursor: i64 = tx.query_row(
        "SELECT cursor FROM history_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    tx.execute("DELETE FROM history_entries WHERE sequence >= ?", [cursor])?;
    collect_garbage(&tx)?;
    let before_state: String = tx.query_row(
        "SELECT current_state_id FROM editor_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    let after_state = Uuid::new_v4().to_string();
    let id = tx.query_row(
        "INSERT INTO history_entries
         (sequence, label, kind, storage_bytes, before_state_id, after_state_id, status)
         VALUES (?, 'Import still in progress', 'media_visibility', 0, ?, ?, 'pending')
         RETURNING id",
        params![cursor, before_state, after_state],
        |row| row.get(0),
    )?;
    tx.execute(
        "UPDATE history_state SET cursor = ? WHERE singleton = 1",
        [cursor + 1],
    )?;
    tx.execute(
        "UPDATE editor_state SET current_state_id = ? WHERE singleton = 1",
        [&after_state],
    )?;
    tx.commit()?;
    Ok(id)
}

pub fn record_media_visibility(
    connection: &mut Connection,
    label: &str,
    media_ids: &[u64],
    deleted: bool,
) -> Result<()> {
    if media_ids.is_empty() {
        return Ok(());
    }
    let selected: Array = Rc::new(
        media_ids
            .iter()
            .map(|id| Value::Integer(*id as i64))
            .collect(),
    );
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let cursor: i64 = tx.query_row(
        "SELECT cursor FROM history_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    tx.execute("DELETE FROM history_entries WHERE sequence >= ?", [cursor])?;
    collect_garbage(&tx)?;
    let before_state: String = tx.query_row(
        "SELECT current_state_id FROM editor_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    let storage_bytes: i64 = tx.query_row(
        "SELECT COALESCE(SUM(COALESCE(l.length, p.length, 0)), 0)
         FROM media m LEFT JOIN loose_media l ON l.media_id = m.id
         LEFT JOIN pack_media p ON p.media_id = m.id
         WHERE m.id IN (SELECT value FROM rarray(?))",
        params![&selected],
        |row| row.get(0),
    )?;
    let after_state = Uuid::new_v4().to_string();
    let history_id: i64 = tx.query_row(
        "INSERT INTO history_entries
         (sequence, label, kind, payload, storage_bytes, before_state_id, after_state_id, status)
         VALUES (?, ?, 'media_visibility', ?, ?, ?, ?, 'ready') RETURNING id",
        params![
            cursor,
            label,
            if deleted { "1" } else { "0" },
            storage_bytes,
            before_state,
            after_state
        ],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT INTO history_media_refs(history_id, media_id)
         SELECT ?1, value FROM rarray(?2)",
        params![history_id, &selected],
    )?;
    tx.execute(
        "UPDATE media SET deleted = ?1 WHERE id IN (SELECT value FROM rarray(?2))",
        params![deleted, &selected],
    )?;
    tx.execute(
        "UPDATE history_state SET cursor = ? WHERE singleton = 1",
        [cursor + 1],
    )?;
    tx.execute(
        "UPDATE editor_state SET current_state_id = ? WHERE singleton = 1",
        [&after_state],
    )?;
    trim(&tx)?;
    collect_garbage(&tx)?;
    tx.commit()?;
    Ok(())
}

pub fn add_imported_media(
    tx: &Transaction<'_>,
    history_id: i64,
    media_id: u64,
    storage_bytes: u64,
) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO history_media_refs(history_id, media_id) VALUES (?, ?)",
        params![history_id, media_id],
    )?;
    tx.execute(
        "UPDATE history_entries SET storage_bytes = storage_bytes + ? WHERE id = ? AND status = 'pending'",
        params![storage_bytes, history_id],
    )?;
    Ok(())
}

pub fn finish_media_import(connection: &mut Connection, history_id: i64) -> Result<Status> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM history_media_refs WHERE history_id = ?",
        [history_id],
        |row| row.get(0),
    )?;
    if count == 0 {
        let entry: Option<(i64, String, Option<String>)> = tx
            .query_row(
                "SELECT sequence, before_state_id, after_state_id
                 FROM history_entries WHERE id = ?",
                [history_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((sequence, before_state, after_state)) = entry {
            tx.execute("DELETE FROM history_entries WHERE id = ?", [history_id])?;
            tx.execute(
                "UPDATE history_entries SET before_state_id = ? WHERE sequence = ?",
                params![before_state, sequence + 1],
            )?;
            shift_sequences_down(&tx, Some(sequence))?;
            tx.execute(
                "UPDATE history_state SET cursor = MAX(0, cursor - 1) WHERE singleton = 1 AND cursor > ?",
                [sequence],
            )?;
            if let Some(after_state) = after_state {
                tx.execute(
                    "UPDATE editor_state SET
                       current_state_id = CASE WHEN current_state_id = ? THEN ? ELSE current_state_id END,
                       saved_state_id = CASE WHEN saved_state_id = ? THEN ? ELSE saved_state_id END
                     WHERE singleton = 1",
                    params![after_state, before_state, after_state, before_state],
                )?;
            }
        }
    } else {
        let label = if count == 1 {
            "Import media item".to_string()
        } else {
            format!("Import {count} media items")
        };
        tx.execute(
            "UPDATE history_entries SET label = ?, status = 'ready' WHERE id = ?",
            params![label, history_id],
        )?;
        trim(&tx)?;
        collect_garbage(&tx)?;
    }
    tx.commit()?;
    status(connection)
}

fn apply_at_cursor(connection: &mut Connection, redo: bool) -> Result<Status> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let cursor: i64 = tx.query_row(
        "SELECT cursor FROM history_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    let sequence = if redo { cursor } else { cursor - 1 };
    if sequence < 0 {
        return status(&tx);
    }
    let row = tx.query_row(
        "SELECT kind, forward_changeset, inverse_changeset, before_state_id,
                after_state_id, status, payload
         FROM history_entries WHERE sequence = ?",
        [sequence],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<Vec<u8>>>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        },
    );
    let (kind, forward, inverse, before_state, after_state, entry_status, payload) = match row {
        Ok(row) => row,
        Err(rusqlite::Error::QueryReturnedNoRows) => return status(&tx),
        Err(error) => return Err(error.into()),
    };
    if entry_status == "pending" {
        bail!("history operation is still in progress");
    }
    match kind.as_str() {
        "changeset" => {
            let mut changeset = Cursor::new(
                if redo { forward } else { inverse }
                    .ok_or_else(|| anyhow::anyhow!("missing changeset"))?,
            );
            tx.apply_strm(&mut changeset, None::<fn(&str) -> bool>, |_, _| {
                ConflictAction::SQLITE_CHANGESET_ABORT
            })?;
        }
        "media_visibility" => {
            let applied_deleted = payload.as_deref().unwrap_or("0") == "1";
            let deleted = if redo {
                applied_deleted
            } else {
                !applied_deleted
            };
            tx.execute(
                "UPDATE media SET deleted = ? WHERE id IN
                 (SELECT media_id FROM history_media_refs
                  WHERE history_id = (SELECT id FROM history_entries WHERE sequence = ?))",
                params![deleted, sequence],
            )?;
        }
        _ => bail!("unsupported history entry kind {kind}"),
    }
    let next_cursor = if redo { cursor + 1 } else { cursor - 1 };
    let state = if redo {
        after_state.ok_or_else(|| anyhow::anyhow!("history entry has no after state"))?
    } else {
        before_state
    };
    tx.execute(
        "UPDATE history_state SET cursor = ? WHERE singleton = 1",
        [next_cursor],
    )?;
    tx.execute(
        "UPDATE editor_state SET current_state_id = ? WHERE singleton = 1",
        [&state],
    )?;
    tx.commit()?;
    status(connection)
}

pub fn status(connection: &Connection) -> Result<Status> {
    let cursor: i64 = connection.query_row(
        "SELECT cursor FROM history_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    let undo = if cursor > 0 {
        connection
            .query_row(
                "SELECT label, status FROM history_entries WHERE sequence = ?",
                [cursor - 1],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .ok()
    } else {
        None
    };
    let redo = connection
        .query_row(
            "SELECT label, status FROM history_entries WHERE sequence = ?",
            [cursor],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .ok();
    let at_saved_state: bool = connection.query_row(
        "SELECT current_state_id IS saved_state_id FROM editor_state WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    Ok(Status {
        can_undo: undo.as_ref().is_some_and(|(_, status)| status == "ready"),
        can_redo: redo.as_ref().is_some_and(|(_, status)| status == "ready"),
        undo_label: undo.map(|(label, _)| label),
        redo_label: redo.map(|(label, _)| label),
        at_saved_state,
    })
}

fn trim(tx: &Transaction<'_>) -> Result<()> {
    loop {
        let (cursor, entries, bytes): (i64, i64, i64) = tx.query_row(
            "SELECT hs.cursor,
                    (SELECT COUNT(*) FROM history_entries WHERE sequence < hs.cursor),
                    COALESCE((SELECT SUM(storage_bytes) FROM history_entries
                              WHERE sequence < hs.cursor), 0)
             FROM history_state hs WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if cursor == 0 || (entries <= 100 && bytes <= 2 * 1024 * 1024 * 1024i64) {
            break;
        }
        let oldest_status: Option<String> = tx
            .query_row(
                "SELECT status FROM history_entries WHERE sequence = 0",
                [],
                |row| row.get(0),
            )
            .optional()?;
        // History can only be trimmed as an oldest-prefix operation. Removing a later ready
        // entry from underneath a pending import would leave a state/changeset gap, so allow a
        // temporary overage until that import finalizes and becomes trimmable.
        if oldest_status.as_deref() != Some("ready") {
            break;
        }
        tx.execute("DELETE FROM history_entries WHERE sequence = 0", [])?;
        shift_sequences_down(tx, None)?;
        tx.execute(
            "UPDATE history_state SET cursor = cursor - 1 WHERE singleton = 1",
            [],
        )?;
    }
    Ok(())
}

/// Shift sequence values without depending on SQLite's row-update order under the UNIQUE
/// constraint. The negative intermediate values cannot collide with live history sequences.
fn shift_sequences_down(tx: &Transaction<'_>, after: Option<i64>) -> Result<()> {
    if let Some(after) = after {
        tx.execute(
            "UPDATE history_entries SET sequence = -sequence - 1 WHERE sequence > ?",
            [after],
        )?;
        tx.execute(
            "UPDATE history_entries SET sequence = -sequence - 2 WHERE sequence < ?",
            [-after - 1],
        )?;
    } else {
        tx.execute("UPDATE history_entries SET sequence = -sequence - 1", [])?;
        tx.execute(
            "UPDATE history_entries SET sequence = -sequence - 2 WHERE sequence < 0",
            [],
        )?;
    }
    Ok(())
}

fn collect_garbage(tx: &Transaction<'_>) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO pending_file_deletions(path)
         SELECT l.path FROM loose_media l JOIN media m ON m.id = l.media_id
         WHERE m.deleted = 1
           AND NOT EXISTS (SELECT 1 FROM snapshot_media s WHERE s.media_id = m.id)
           AND NOT EXISTS (SELECT 1 FROM history_media_refs h WHERE h.media_id = m.id)",
        [],
    )?;
    tx.execute(
        "DELETE FROM media WHERE deleted = 1
           AND NOT EXISTS (SELECT 1 FROM snapshot_media s WHERE s.media_id = media.id)
           AND NOT EXISTS (SELECT 1 FROM history_media_refs h WHERE h.media_id = media.id)",
        [],
    )?;
    Ok(())
}
