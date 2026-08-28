use anyhow::Result;
use rusqlite::{Connection, Transaction, params};

use super::tags::{read_tags, write_tags};
use crate::behaviour::schema::{ContentGroup, TextItem, WebLink};

// ── The content pools ────────────────────────────────────────────────────────

pub fn read_text_pool(conn: &Connection, kind: &str) -> Result<Vec<TextItem>> {
    let mut statement = conn.prepare(
        "SELECT id, text, timeout_seconds, summary FROM behaviour_text_item WHERE kind = ? \
         ORDER BY id",
    )?;
    let rows: Vec<(i64, String, Option<f64>, Option<String>)> = statement
        .query_map(params![kind], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, text, timeout_seconds, summary)| {
            Ok(TextItem {
                text,
                tags: read_tags(conn, "behaviour_text_item_tag", "item_id", &id)?,
                timeout_seconds,
                summary,
            })
        })
        .collect()
}

/// Writes a pool into a table the caller has already emptied, in document order -- which the
/// ascending ids the inserts mint then preserve, so the read above can order by id alone.
pub fn write_text_pool(tx: &Transaction<'_>, kind: &str, items: &[TextItem]) -> Result<()> {
    for item in items {
        let id: i64 = tx.query_row(
            "INSERT INTO behaviour_text_item (kind, text, timeout_seconds, summary)
             VALUES (?1, ?2, ?3, ?4)
             RETURNING id",
            params![kind, item.text, item.timeout_seconds, item.summary],
            |row| row.get(0),
        )?;
        write_tags(tx, "behaviour_text_item_tag", "item_id", &id, &item.tags)?;
    }
    Ok(())
}

pub fn read_web_links(conn: &Connection) -> Result<Vec<WebLink>> {
    let mut statement = conn.prepare("SELECT id, url FROM behaviour_web_link ORDER BY id")?;
    let rows: Vec<(i64, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;

    rows.into_iter()
        .map(|(id, url)| {
            Ok(WebLink {
                url,
                tags: read_tags(conn, "behaviour_web_link_tag", "link_id", &id)?,
            })
        })
        .collect()
}

pub fn write_web_links(tx: &Transaction<'_>, links: &[WebLink]) -> Result<()> {
    for link in links {
        let id: i64 = tx.query_row(
            "INSERT INTO behaviour_web_link (url) VALUES (?1)
             RETURNING id",
            params![link.url],
            |row| row.get(0),
        )?;
        write_tags(tx, "behaviour_web_link_tag", "link_id", &id, &link.tags)?;
    }
    Ok(())
}

/// Every group, in author order -- which is the surrogate id, since the rows are written in
/// document order and only ever appended to afterwards. `ContentGroup::id` is the document's
/// `key`; the surrogate never leaves this module and its neighbours.
pub fn read_content_groups(conn: &Connection) -> Result<Vec<ContentGroup>> {
    let mut statement = conn.prepare(
        "SELECT id, key, label, description, enabled_by_default
         FROM behaviour_content_group ORDER BY id",
    )?;
    let rows: Vec<(i64, String, String, Option<String>, bool)> = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(row_id, key, label, description, enabled_by_default)| {
            Ok(ContentGroup {
                tags: read_tags(conn, "behaviour_content_group_tag", "group_id", &row_id)?,
                id: key,
                label,
                description,
                enabled_by_default,
            })
        })
        .collect()
}

pub fn write_content_groups(tx: &Transaction<'_>, groups: &[ContentGroup]) -> Result<()> {
    for group in groups {
        let row_id: i64 = tx.query_row(
            "INSERT INTO behaviour_content_group
                 (key, label, description, enabled_by_default)
             VALUES (?, ?, ?, ?) RETURNING id",
            params![
                group.id,
                group.label,
                group.description,
                group.enabled_by_default,
            ],
            |row| row.get(0),
        )?;
        write_tags(
            tx,
            "behaviour_content_group_tag",
            "group_id",
            &row_id,
            &group.tags,
        )?;
    }
    Ok(())
}
