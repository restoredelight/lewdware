use super::*;

// ── Setting one field ────────────────────────────────────────────────────────
//
// Everything below sets a single field, and that is the point rather than an optimisation. A
// command that writes a whole entity cannot be applied twice concurrently without one write
// carrying a stale copy of whatever the other changed — which is why the editor used to keep a
// draft, merge changes into it, and count generations to know when it could let go. Two writes that
// each touch one column commute, so none of that is needed for correctness.
//
// It also removes the "was this field set, or merely not mentioned?" problem. A partial update has
// to tell an absent field from a null one, which is what `PopupChanges` needed `double_option` for;
// a setter that names its field has nothing to not-mention, so a plain `Option<T>` says it all.
//
// **Every setter reports whether it changed anything.** An `oninput` that fires without changing a
// value is not an edit, and recording one would spend an entry out of the hundred undo keeps.

/// Writes one column of an existing row, reporting whether the value actually moved.
///
/// The row must exist: an edit arriving for something that has since been removed is a stale editor
/// acting on a document that has moved on, and re-creating the row would undo the removal.
///
/// `value` is owned rather than borrowed because the same type has to come back out of the database
/// to be compared against — and `&str` cannot, only `String` can.
pub(super) fn set_column<T>(
    tx: &Transaction<'_>,
    table: &str,
    key_column: &str,
    key: &dyn rusqlite::ToSql,
    column: &str,
    value: T,
    missing: &str,
) -> Result<bool>
where
    T: rusqlite::ToSql + rusqlite::types::FromSql + PartialEq,
{
    let current: Option<T> = tx
        .query_row(
            &format!("SELECT {column} FROM {table} WHERE {key_column} = ?"),
            params![key],
            |row| row.get(0),
        )
        .optional()?;
    let Some(current) = current else {
        bail!("{missing}");
    };
    if current == value {
        return Ok(false);
    }
    tx.execute(
        &format!("UPDATE {table} SET {column} = ?2 WHERE {key_column} = ?1"),
        params![key, value],
    )?;
    Ok(true)
}

/// Refuses if the row is gone. See [`set_column`] for why that is an error rather than a no-op.
pub(super) fn exists(
    tx: &Transaction<'_>,
    table: &str,
    key_column: &str,
    key: &dyn rusqlite::ToSql,
    missing: &str,
) -> Result<()> {
    let found: bool = tx.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {key_column} = ?)"),
        params![key],
        |row| row.get(0),
    )?;
    if !found {
        bail!("{missing}");
    }
    Ok(())
}

// ── One item of a collection ─────────────────────────────────────────────────
//
// A tag chip and a URL suffix are *items*, and adding or removing one is the whole of what the
// author did. Setting the resulting list instead has the same flaw whole-entity writes had, one
// level down: the list is built from what was last fetched, so two removals in quick succession
// both say "everything except the one I clicked" and the second puts the first one back.
//
// Addressing the item makes the two edits commute, which is the same reason the field setters
// above exist.

/// Adds `tag` to an owner's selection, at the end. Reports whether it was not already there.
pub(super) fn add_tag(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
    tag: &str,
) -> Result<bool> {
    let id = crate::behaviour::storage::tag_id(tx, tag)?;
    let present: bool = tx.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {key} = ?1 AND tag_id = ?2)"),
        params![owner, id],
        |row| row.get(0),
    )?;
    if present {
        return Ok(false);
    }
    let position: i64 = tx.query_row(
        &format!("SELECT COALESCE(MAX(position) + 1, 0) FROM {table} WHERE {key} = ?"),
        params![owner],
        |row| row.get(0),
    )?;
    tx.execute(
        &format!("INSERT INTO {table} ({key}, tag_id, position) VALUES (?1, ?2, ?3)"),
        params![owner, id, position],
    )?;
    Ok(true)
}

/// Removes `tag` from an owner's selection. Reports whether it was there.
///
/// The gap it leaves in `position` is not closed: positions only order the list, and the primary
/// key is the owner and the tag, so nothing collides with a hole.
pub(super) fn remove_tag(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
    tag: &str,
) -> Result<bool> {
    let removed = tx.execute(
        &format!(
            "DELETE FROM {table} WHERE {key} = ?1
             AND tag_id = (SELECT id FROM tags WHERE name = ?2)"
        ),
        params![owner, tag],
    )?;
    Ok(removed > 0)
}
