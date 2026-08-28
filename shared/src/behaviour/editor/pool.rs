use anyhow::{Result, bail};
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

use super::common::{add_tag, exists, remove_tag, set_column};
use crate::behaviour::db::{read_tags, write_tags};
use crate::behaviour::schema::TextItem;

/// Which of the three text pools an item belongs to.
///
/// The pools share a table and differ only in which one a mode draws from, so this is the `kind`
/// column rather than three near-identical types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolKind {
    Caption,
    Prompt,
    Notification,
}

impl PoolKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Caption => "caption",
            Self::Prompt => "prompt",
            Self::Notification => "notification",
        }
    }
}

impl rusqlite::ToSql for PoolKind {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::from(self.as_str()))
    }
}

/// One text-pool entry, with the row id the editor addresses it by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextItemRow {
    pub id: i64,
    #[serde(flatten)]
    pub item: TextItem,
}
// ── Text pools ───────────────────────────────────────────────────────────────

/// One pool's entries, in author order.
pub fn text_pool(conn: &Connection, kind: PoolKind) -> Result<Vec<TextItemRow>> {
    let mut statement = conn.prepare(
        "SELECT id, text, timeout_seconds, summary FROM behaviour_text_item
         WHERE kind = ? ORDER BY id",
    )?;
    statement
        .query_and_then(params![kind], |row| {
            let id: i64 = row.get(0)?;
            Ok(TextItemRow {
                id,
                item: TextItem {
                    text: row.get(1)?,
                    tags: read_tags(conn, "behaviour_text_item_tag", "item_id", &id)?,
                    timeout_seconds: row.get(2)?,
                    summary: row.get(3)?,
                },
            })
        })?
        .collect()
}

/// Appends an entry to `kind`'s pool, returning its new id.
///
/// Appending is the only way in: the id the insert mints is what orders the pool, so a new entry
/// lands at the end by construction.
pub fn add_text_item(tx: &Transaction<'_>, kind: PoolKind, item: &TextItem) -> Result<i64> {
    let id: i64 = tx.query_row(
        "INSERT INTO behaviour_text_item (kind, text, timeout_seconds, summary)
         VALUES (?1, ?2, ?3, ?4) RETURNING id",
        params![kind, item.text, item.timeout_seconds, item.summary],
        |row| row.get(0),
    )?;
    write_tags(tx, "behaviour_text_item_tag", "item_id", &id, &item.tags)?;
    Ok(id)
}

/// Removes an entry, leaving the rest of the pool in the order it was already in.
pub fn remove_text_item(tx: &Transaction<'_>, id: i64) -> Result<()> {
    tx.execute("DELETE FROM behaviour_text_item WHERE id = ?", params![id])?;
    Ok(())
}

// ── Text pool entries ────────────────────────────────────────────────────────

const NO_TEXT_ITEM: &str = "that entry is no longer in the pack";

pub fn set_text_item_text(tx: &Transaction<'_>, id: i64, text: &str) -> Result<()> {
    set_column(
        tx,
        "behaviour_text_item",
        "id",
        &id,
        "text",
        text.to_string(),
        NO_TEXT_ITEM,
    )
}

/// The notification's title. `None` is a notification shown with no title, which is what every
/// converted Edgeware pack gets — a blank one is the absence of a title, not a title of "".
pub fn set_text_item_summary(tx: &Transaction<'_>, id: i64, summary: Option<&str>) -> Result<()> {
    set_column(
        tx,
        "behaviour_text_item",
        "id",
        &id,
        "summary",
        summary.map(str::to_string),
        NO_TEXT_ITEM,
    )
}

/// A prompt's answer deadline. `None` asks the mode to derive one from the prompt's length.
pub fn set_text_item_timeout(tx: &Transaction<'_>, id: i64, seconds: Option<f64>) -> Result<()> {
    set_column(
        tx,
        "behaviour_text_item",
        "id",
        &id,
        "timeout_seconds",
        seconds,
        NO_TEXT_ITEM,
    )
}

pub fn add_text_item_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<()> {
    if !exists(tx, "behaviour_text_item", "id", &id)? {
        bail!("{NO_TEXT_ITEM}");
    }
    add_tag(tx, "behaviour_text_item_tag", "item_id", &id, tag)
}

pub fn remove_text_item_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<()> {
    remove_tag(tx, "behaviour_text_item_tag", "item_id", &id, tag)
}
