use anyhow::{Result, bail};
use rusqlite::{Transaction, params};

/// Writes one column of an existing row.
pub fn set_column<T: rusqlite::ToSql>(
    tx: &Transaction<'_>,
    table: &str,
    key_column: &str,
    key: &dyn rusqlite::ToSql,
    column: &str,
    value: T,
    missing: &str,
) -> Result<()> {
    let updated = tx.execute(
        &format!("UPDATE {table} SET {column} = ?2 WHERE {key_column} = ?1"),
        params![key, value],
    )?;

    if updated == 0 {
        bail!("{missing}");
    }

    Ok(())
}

/// Reports whether a row exists for `key`.
pub fn exists(
    tx: &Transaction<'_>,
    table: &str,
    key_column: &str,
    key: &dyn rusqlite::ToSql,
) -> Result<bool> {
    let found = tx.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {key_column} = ?)"),
        params![key],
        |row| row.get(0),
    )?;

    Ok(found)
}

/// Adds `tag` to an owner's selection, at the end. Already having it is not an error, and not a
/// change either -- the conflict clause makes it a write that writes nothing.
pub fn add_tag(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
    tag: &str,
) -> Result<()> {
    let id = crate::behaviour::db::tag_id(tx, tag)?;
    tx.execute(
        &format!(
            "INSERT INTO {table} ({key}, tag_id) VALUES (?1, ?2)
             ON CONFLICT({key}, tag_id) DO NOTHING"
        ),
        params![owner, id],
    )?;

    Ok(())
}

/// Removes `tag` from an owner's selection. Not having it is not an error.
pub fn remove_tag(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
    tag: &str,
) -> Result<()> {
    tx.execute(
        &format!(
            "DELETE FROM {table} WHERE {key} = ?1
             AND tag_id = (SELECT id FROM tags WHERE name = ?2)"
        ),
        params![owner, tag],
    )?;

    Ok(())
}
