use std::{io::Cursor, rc::Rc};

use anyhow::{bail, Result};
use rusqlite::{
    params,
    session::{invert_strm, ChangesetItem, ConflictAction, ConflictType, Session},
    types::Value,
    vtab::array::Array,
    Connection, OptionalExtension, Transaction, TransactionBehavior,
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

/// Records that `history_id` touches `media_id`, so `collect_garbage` keeps that media's bytes for
/// as long as the entry can still be undone. For media an entry *removed* rather than imported --
/// which has no storage of its own to account for; see `add_imported_media` for that.
pub fn add_media_ref(tx: &Transaction<'_>, history_id: i64, media_id: u64) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO history_media_refs(history_id, media_id) VALUES (?, ?)",
        params![history_id, media_id],
    )?;
    Ok(())
}

pub fn add_imported_media(
    tx: &Transaction<'_>,
    history_id: i64,
    media_id: u64,
    storage_bytes: u64,
) -> Result<()> {
    add_media_ref(tx, history_id, media_id)?;
    tx.execute(
        "UPDATE history_entries SET storage_bytes = storage_bytes + ? WHERE id = ? AND status = 'pending'",
        params![storage_bytes, history_id],
    )?;
    Ok(())
}

/// Folds one more operation into a *pending* media import, so the whole thing undoes as one
/// entry.
///
/// Importing a file and pointing a media slot at it is one action to the author ("set the
/// wallpaper"), and two undo steps for it would be a lie about what they did. The import half
/// can't be a changeset -- undoing an import soft-deletes the media row so its bytes survive for
/// redo (see `collect_garbage`), where an inverted changeset would hard-delete it and orphan the
/// loose file. So the other half rides along instead: the operation's changeset is stored on the
/// visibility entry and applied beside it (see `apply_at_cursor`).
///
/// Call once per pending entry, before `finish_media_import`.
pub fn record_into_pending<T>(
    connection: &mut Connection,
    history_id: i64,
    operation: impl FnOnce(&Transaction<'_>) -> Result<T>,
) -> Result<T> {
    let mut session = Session::new(connection)?;
    for table in TRACKED_TABLES {
        session.attach(Some(*table))?;
    }

    let tx = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let value = operation(&tx)?;
    let mut forward = Vec::new();
    session.changeset_strm(&mut forward)?;
    if forward.is_empty() {
        tx.commit()?;
        return Ok(value);
    }
    let mut inverse = Vec::new();
    invert_strm(&mut Cursor::new(&forward), &mut inverse)?;
    tx.execute(
        "UPDATE history_entries SET forward_changeset = ?, inverse_changeset = ?
         WHERE id = ? AND status = 'pending'",
        params![forward, inverse, history_id],
    )?;
    tx.commit()?;
    Ok(value)
}

/// Finalizes a pending media import. `label` overrides the default "Import N media items" for a
/// caller that folded something else in and wants the entry named after the whole action.
pub fn finish_media_import(
    connection: &mut Connection,
    history_id: i64,
    label: Option<&str>,
) -> Result<Status> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM history_media_refs WHERE history_id = ?",
        [history_id],
        |row| row.get(0),
    )?;
    // A folded operation with no media of its own -- filling a slot from a file the pack already
    // had. Nothing to make visible, but the changeset is still a real edit, so the entry stays as
    // an ordinary changeset rather than being discarded as an empty import.
    let folded: bool = tx.query_row(
        "SELECT forward_changeset IS NOT NULL FROM history_entries WHERE id = ?",
        [history_id],
        |row| row.get(0),
    )?;
    if count == 0 && folded {
        tx.execute(
            "UPDATE history_entries SET kind = 'changeset', label = ?, status = 'ready'
             WHERE id = ?",
            params![label.unwrap_or("Edit pack"), history_id],
        )?;
        trim(&tx)?;
        collect_garbage(&tx)?;
        tx.commit()?;
        return status(connection);
    }
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
        let label = match label {
            Some(label) => label.to_string(),
            None if count == 1 => "Import media item".to_string(),
            None => format!("Import {count} media items"),
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

/// What to do when a change in a history changeset doesn't fit the database it's being applied
/// to.
///
/// `NOTFOUND` is expected and benign: our tracked tables cascade, so an inverted changeset that
/// deletes a `tags` row takes the `media_tags` rows referencing it along, and the changeset's own
/// delete for those rows then finds nothing. The end state is the one the change was asking for,
/// so skipping it is right -- aborting instead made undo fail outright on any entry that created
/// a tag and attached it in one step (creating a tag from the media sidebar, or filling a media
/// slot, which applies its marker the same way).
///
/// Anything else is a real disagreement about the state -- a row that exists where the changeset
/// expected none, or different values than it recorded -- and must not be papered over.
fn on_conflict(conflict: ConflictType, _item: ChangesetItem) -> ConflictAction {
    match conflict {
        ConflictType::SQLITE_CHANGESET_NOTFOUND => ConflictAction::SQLITE_CHANGESET_OMIT,
        _ => ConflictAction::SQLITE_CHANGESET_ABORT,
    }
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
            tx.apply_strm(&mut changeset, None::<fn(&str) -> bool>, on_conflict)?;
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
            // An entry that folded a second operation in (see `record_into_pending`) carries its
            // changeset too, and the two are one action -- they move together or the author gets
            // a slot pointing at media this same step just hid.
            if let Some(changeset) = if redo { forward } else { inverse } {
                tx.apply_strm(
                    &mut Cursor::new(changeset),
                    None::<fn(&str) -> bool>,
                    on_conflict,
                )?;
            }
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
