use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::common::{add_tag, exists, remove_tag, set_column};
use crate::behaviour::schema::WebLink;
use crate::behaviour::storage::{read_tags, write_tags};

/// One web link, with the row id the editor addresses it by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebLinkRow {
    pub id: i64,
    pub url: String,
    pub tags: Vec<String>,
}
// ── Web links ────────────────────────────────────────────────────────────────

/// Every web link, in author order.
pub fn web_links(conn: &Connection) -> Result<Vec<WebLinkRow>> {
    let mut statement = conn.prepare("SELECT id, url FROM behaviour_web_link ORDER BY position")?;
    let rows: Vec<(i64, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, url)| {
            Ok(WebLinkRow {
                id,
                url,
                tags: read_tags(conn, "behaviour_web_link_tag", "link_id", &id)?,
            })
        })
        .collect()
}

/// Appends a web link, returning its new id.
pub fn add_web_link(tx: &Transaction<'_>, link: &WebLink) -> Result<i64> {
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_web_link",
        [],
        |row| row.get(0),
    )?;
    let id: i64 = tx.query_row(
        "INSERT INTO behaviour_web_link (position, url) VALUES (?1, ?2) RETURNING id",
        params![position, link.url],
        |row| row.get(0),
    )?;
    write_tags(tx, "behaviour_web_link_tag", "link_id", &id, &link.tags)?;
    Ok(id)
}

/// Removes a web link and closes the gap its position left.
pub fn remove_web_link(tx: &Transaction<'_>, id: i64) -> Result<bool> {
    let position: Option<i64> = tx
        .query_row(
            "SELECT position FROM behaviour_web_link WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(position) = position else {
        return Ok(false);
    };
    tx.execute("DELETE FROM behaviour_web_link WHERE id = ?", params![id])?;
    tx.execute(
        "UPDATE behaviour_web_link SET position = position - 1 WHERE position > ?",
        params![position],
    )?;
    Ok(true)
}

// ── Web links ────────────────────────────────────────────────────────────────

const NO_WEB_LINK: &str = "that web link is no longer in the pack";

pub fn set_web_link_url(tx: &Transaction<'_>, id: i64, url: &str) -> Result<bool> {
    set_column(
        tx,
        "behaviour_web_link",
        "id",
        &id,
        "url",
        url.to_string(),
        NO_WEB_LINK,
    )
}

pub fn add_web_link_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<bool> {
    exists(tx, "behaviour_web_link", "id", &id, NO_WEB_LINK)?;
    add_tag(tx, "behaviour_web_link_tag", "link_id", &id, tag)
}

pub fn remove_web_link_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<bool> {
    remove_tag(tx, "behaviour_web_link_tag", "link_id", &id, tag)
}
