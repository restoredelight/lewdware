use anyhow::{Result, bail};
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

use super::common::{add_tag, exists, remove_tag, set_column};
use crate::behaviour::db::{read_tags, write_tags};
use crate::behaviour::schema::WebLink;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebLinkRecord {
    pub id: i64,
    pub url: String,
    pub tags: Vec<String>,
}

pub fn web_links(conn: &Connection) -> Result<Vec<WebLinkRecord>> {
    let mut statement = conn.prepare("SELECT id, url FROM behaviour_web_link ORDER BY id")?;
    statement
        .query_and_then([], |row| {
            let id: i64 = row.get("id")?;
            Ok(WebLinkRecord {
                id,
                url: row.get("url")?,
                tags: read_tags(conn, "behaviour_web_link_tag", "link_id", &id)?,
            })
        })?
        .collect()
}

pub fn add_web_link(tx: &Transaction<'_>, link: &WebLink) -> Result<i64> {
    let id: i64 = tx.query_row(
        "INSERT INTO behaviour_web_link (url) VALUES (?1) RETURNING id",
        params![link.url],
        |row| row.get(0),
    )?;
    write_tags(tx, "behaviour_web_link_tag", "link_id", &id, &link.tags)?;
    Ok(id)
}

pub fn remove_web_link(tx: &Transaction<'_>, id: i64) -> Result<()> {
    tx.execute("DELETE FROM behaviour_web_link WHERE id = ?", params![id])?;
    Ok(())
}

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
