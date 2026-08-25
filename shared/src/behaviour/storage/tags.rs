use super::*;

// ── Tags ─────────────────────────────────────────────────────────────────────
//
// Every tag list in the document is a join to `tags`. That is what makes renaming a tag
// `UPDATE tags SET name`, merging one a re-point, and deleting one a cascade, instead of a pass
// over the whole document rewriting strings (the late `Behaviour::rewrite_tag`).

pub(in crate::behaviour) fn read_tags(
    conn: &Connection,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
) -> Result<Vec<String>> {
    let mut statement = conn.prepare(&format!(
        "SELECT tags.name FROM {table}
         JOIN tags ON tags.id = {table}.tag_id
         WHERE {table}.{key} = ? ORDER BY {table}.position"
    ))?;
    let tags = statement
        .query_map([owner], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tags)
}

pub(in crate::behaviour) fn write_tags(
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
             AND tag_id NOT IN (SELECT value FROM json_each(?2))"
        ),
        params![owner, serde_json::to_string(&ids)?],
    )?;
    for (position, id) in ids.iter().enumerate() {
        tx.execute(
            &format!(
                "INSERT INTO {table} ({key}, tag_id, position) VALUES (?1, ?2, ?3)
                 ON CONFLICT({key}, tag_id) DO UPDATE SET position = excluded.position"
            ),
            params![owner, id, position as i64],
        )?;
    }
    Ok(())
}

/// The id of `name`, creating the tag if the pack does not have it yet.
///
/// A tag the document names *is* a tag the pack has -- that is what the foreign key asserts. So a
/// tag typed into a caption or a content group becomes a real row, and shows up in the Tags tab
/// with no media attached, rather than existing only as a string inside the document.
pub(in crate::behaviour) fn tag_id(tx: &Transaction<'_>, name: &str) -> Result<i64> {
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
