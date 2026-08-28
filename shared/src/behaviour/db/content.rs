use anyhow::Result;
use rusqlite::{Connection, Transaction, named_params, params};

use super::tags::{read_tags, write_tags};
use crate::behaviour::schema::{ContentGroup, TextItem, WebLink};

pub fn read_text_pool(conn: &Connection, kind: &str) -> Result<Vec<TextItem>> {
    let mut statement = conn.prepare(
        "SELECT id, text, timeout_seconds, summary FROM behaviour_text_item WHERE kind = :kind \
         ORDER BY id",
    )?;

    statement
        .query_and_then(named_params![":kind": kind], |row| {
            let id: i64 = row.get("id")?;

            Ok(TextItem {
                text: row.get("text")?,
                tags: read_tags(conn, "behaviour_text_item_tag", "item_id", &id)?,
                timeout_seconds: row.get("timeout_seconds")?,
                summary: row.get("summary")?,
            })
        })?
        .collect()
}

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
    statement
        .query_and_then([], |row| {
            let id: i64 = row.get("id")?;
            Ok(WebLink {
                url: row.get("url")?,
                tags: read_tags(conn, "behaviour_web_link_tag", "link_id", &id)?,
            })
        })?
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

pub fn read_content_groups(conn: &Connection) -> Result<Vec<ContentGroup>> {
    let mut statement = conn.prepare(
        "SELECT id, key, label, description, enabled_by_default
         FROM behaviour_content_group ORDER BY id",
    )?;

    statement
        .query_and_then([], |row| {
            let row_id: i64 = row.get("id")?;

            Ok(ContentGroup {
                tags: read_tags(conn, "behaviour_content_group_tag", "group_id", &row_id)?,
                id: row.get("key")?,
                label: row.get("label")?,
                description: row.get("description")?,
                enabled_by_default: row.get("enabled_by_default")?,
            })
        })?
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
