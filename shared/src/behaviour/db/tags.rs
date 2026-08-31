use anyhow::Result;
use rusqlite::{Connection, Transaction, params};

use crate::behaviour::db::common::sql_array;

pub fn read_tags(
    conn: &Connection,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
) -> Result<Vec<String>> {
    let mut statement = conn.prepare(&format!(
        "SELECT tags.name FROM {table}
         JOIN tags ON tags.id = {table}.tag_id
         WHERE {table}.{key} = ? ORDER BY {table}.id"
    ))?;
    let tags = statement
        .query_map([owner], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tags)
}

pub fn write_tags(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
    tags: &[String],
) -> Result<()> {
    let ids = tags
        .iter()
        .map(|tag| tag_id(tx, tag))
        .collect::<Result<Vec<_>>>()?;

    tx.execute(
        &format!(
            "DELETE FROM {table} WHERE {key} = ?1
             AND tag_id NOT IN (SELECT value FROM rarray(?2))"
        ),
        params![owner, sql_array(ids.clone())?],
    )?;

    for id in ids {
        tx.execute(
            &format!(
                "INSERT INTO {table} ({key}, tag_id) VALUES (?1, ?2)
                 ON CONFLICT({key}, tag_id) DO NOTHING"
            ),
            params![owner, id],
        )?;
    }

    Ok(())
}

/// The id of `name`, creating the tag if the pack does not have it yet.
pub fn tag_id(tx: &Transaction<'_>, name: &str) -> Result<i64> {
    tx.execute(
        "INSERT OR IGNORE INTO tags (name) VALUES (?)",
        params![name],
    )?;

    Ok(
        tx.query_row("SELECT id FROM tags WHERE name = ?", params![name], |row| {
            row.get(0)
        })?,
    )
}
