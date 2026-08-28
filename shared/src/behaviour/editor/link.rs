use anyhow::{Result, bail};
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

use super::common::{add_tag, exists, remove_tag, set_column};
use crate::behaviour::db::{read_tags, write_tags};
use crate::behaviour::schema::WebLink;

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
    let mut statement = conn.prepare("SELECT id, url FROM behaviour_web_link ORDER BY id")?;
    statement
        .query_and_then([], |row| {
            let id: i64 = row.get(0)?;
            Ok(WebLinkRow {
                id,
                url: row.get(1)?,
                tags: read_tags(conn, "behaviour_web_link_tag", "link_id", &id)?,
            })
        })?
        .collect()
}

/// Appends a web link, returning its new id.
///
/// Appending is the only way in: the id the insert mints is what orders the list, so a new link
/// lands at the end by construction.
pub fn add_web_link(tx: &Transaction<'_>, link: &WebLink) -> Result<i64> {
    let id: i64 = tx.query_row(
        "INSERT INTO behaviour_web_link (url) VALUES (?1) RETURNING id",
        params![link.url],
        |row| row.get(0),
    )?;
    write_tags(tx, "behaviour_web_link_tag", "link_id", &id, &link.tags)?;
    Ok(id)
}

/// Removes a web link, leaving the rest in the order they were already in.
pub fn remove_web_link(tx: &Transaction<'_>, id: i64) -> Result<()> {
    tx.execute("DELETE FROM behaviour_web_link WHERE id = ?", params![id])?;
    Ok(())
}

// ── Web links ────────────────────────────────────────────────────────────────

const NO_WEB_LINK: &str = "that web link is no longer in the pack";

pub fn set_web_link_url(tx: &Transaction<'_>, id: i64, url: &str) -> Result<()> {
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

pub fn add_web_link_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<()> {
    if !exists(tx, "behaviour_web_link", "id", &id)? {
        bail!("{NO_WEB_LINK}");
    }
    add_tag(tx, "behaviour_web_link_tag", "link_id", &id, tag)
}

pub fn remove_web_link_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<()> {
    remove_tag(tx, "behaviour_web_link_tag", "link_id", &id, tag)
}
