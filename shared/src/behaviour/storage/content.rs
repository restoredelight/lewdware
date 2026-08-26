use anyhow::Result;
use rusqlite::{Connection, Transaction, params};

use super::common::retain;
use super::tags::{read_tags, write_tags};
use crate::behaviour::schema::{ContentGroup, TextItem, WebLink};

// ── The content pools ────────────────────────────────────────────────────────

pub fn read_text_pool(conn: &Connection, kind: &str) -> Result<Vec<TextItem>> {
    let mut statement = conn.prepare(
        "SELECT id, text, timeout_seconds, summary FROM behaviour_text_item WHERE kind = ? \
         ORDER BY position",
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

pub fn write_text_pool(
    tx: &Transaction<'_>,
    kind: &str,
    items: &[TextItem],
) -> Result<()> {
    // Pool entries have no author-facing identity -- nothing outside the document refers to a
    // caption -- so they are keyed by position and rewritten in place, rather than matched up.
    tx.execute(
        "DELETE FROM behaviour_text_item WHERE kind = ?1 AND position >= ?2",
        params![kind, items.len() as i64],
    )?;
    for (position, item) in items.iter().enumerate() {
        let id: i64 = tx.query_row(
            "INSERT INTO behaviour_text_item (kind, position, text, timeout_seconds, summary)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(kind, position) DO UPDATE SET text = excluded.text,
                                                       timeout_seconds = excluded.timeout_seconds,
                                                       summary = excluded.summary
             RETURNING id",
            params![
                kind,
                position as i64,
                item.text,
                item.timeout_seconds,
                item.summary
            ],
            |row| row.get(0),
        )?;
        write_tags(tx, "behaviour_text_item_tag", "item_id", &id, &item.tags)?;
    }
    Ok(())
}

pub fn read_web_links(conn: &Connection) -> Result<Vec<WebLink>> {
    let mut statement = conn.prepare("SELECT id, url FROM behaviour_web_link ORDER BY position")?;
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
    tx.execute(
        "DELETE FROM behaviour_web_link WHERE position >= ?",
        params![links.len() as i64],
    )?;
    for (position, link) in links.iter().enumerate() {
        let id: i64 = tx.query_row(
            "INSERT INTO behaviour_web_link (position, url) VALUES (?1, ?2)
             ON CONFLICT(position) DO UPDATE SET url = excluded.url
             RETURNING id",
            params![position as i64, link.url],
            |row| row.get(0),
        )?;
        write_tags(tx, "behaviour_web_link_tag", "link_id", &id, &link.tags)?;
    }
    Ok(())
}

pub fn read_content_groups(conn: &Connection) -> Result<Vec<ContentGroup>> {
    let mut statement = conn.prepare(
        "SELECT id, label, description, enabled_by_default
         FROM behaviour_content_group ORDER BY position",
    )?;
    let rows: Vec<(String, String, Option<String>, bool)> = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, label, description, enabled_by_default)| {
            Ok(ContentGroup {
                tags: read_tags(conn, "behaviour_content_group_tag", "group_id", &id)?,
                id,
                label,
                description,
                enabled_by_default,
            })
        })
        .collect()
}

pub fn write_content_groups(
    tx: &Transaction<'_>,
    groups: &[ContentGroup],
) -> Result<()> {
    retain(
        tx,
        "behaviour_content_group",
        "id",
        groups.iter().map(|group| group.id.as_str()),
    )?;
    for (position, group) in groups.iter().enumerate() {
        tx.execute(
            "INSERT INTO behaviour_content_group
                 (id, position, label, description, enabled_by_default)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET position = excluded.position,
                                           label = excluded.label,
                                           description = excluded.description,
                                           enabled_by_default = excluded.enabled_by_default",
            params![
                group.id,
                position as i64,
                group.label,
                group.description,
                group.enabled_by_default,
            ],
        )?;
        write_tags(
            tx,
            "behaviour_content_group_tag",
            "group_id",
            &group.id,
            &group.tags,
        )?;
    }
    Ok(())
}
