use anyhow::{Result, bail};
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

use super::common::{add_tag, exists, remove_tag};
use crate::behaviour::db::{read_text_pool_rows, write_tags};
use crate::behaviour::schema::{Caption, Notification, Prompt};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextItemRow {
    pub id: i64,
    pub text: String,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// The shared table's flat insert shape. Only the concrete public add functions can construct it.
struct TextItemDraft<'a> {
    kind: PoolKind,
    text: &'a str,
    tags: &'a [String],
    timeout_seconds: Option<f64>,
    summary: Option<&'a str>,
}

/// One pool's entries, in author order.
pub fn text_pool(conn: &Connection, kind: PoolKind) -> Result<Vec<TextItemRow>> {
    match kind {
        PoolKind::Caption => Ok(read_text_pool_rows::<Caption>(conn)?
            .into_iter()
            .map(|(id, item)| TextItemRow {
                id,
                text: item.text,
                tags: item.tags,
                timeout_seconds: None,
                summary: None,
            })
            .collect()),
        PoolKind::Prompt => Ok(read_text_pool_rows::<Prompt>(conn)?
            .into_iter()
            .map(|(id, item)| TextItemRow {
                id,
                text: item.text,
                tags: item.tags,
                timeout_seconds: item.timeout_seconds,
                summary: None,
            })
            .collect()),
        PoolKind::Notification => Ok(read_text_pool_rows::<Notification>(conn)?
            .into_iter()
            .map(|(id, item)| TextItemRow {
                id,
                text: item.text,
                tags: item.tags,
                timeout_seconds: None,
                summary: item.summary,
            })
            .collect()),
    }
}

fn add_text_item(tx: &Transaction<'_>, item: TextItemDraft<'_>) -> Result<i64> {
    let id: i64 = tx.query_row(
        "INSERT INTO behaviour_text_item (kind, text, timeout_seconds, summary)
         VALUES (?1, ?2, ?3, ?4) RETURNING id",
        params![item.kind, item.text, item.timeout_seconds, item.summary],
        |row| row.get(0),
    )?;

    write_tags(tx, "behaviour_text_item_tag", "item_id", &id, item.tags)?;

    Ok(id)
}

/// Appends a caption, returning its new editor row id.
pub fn add_caption(tx: &Transaction<'_>, caption: &Caption) -> Result<i64> {
    add_text_item(
        tx,
        TextItemDraft {
            kind: PoolKind::Caption,
            text: &caption.text,
            tags: &caption.tags,
            timeout_seconds: None,
            summary: None,
        },
    )
}

/// Appends a prompt, returning its new editor row id.
pub fn add_prompt(tx: &Transaction<'_>, prompt: &Prompt) -> Result<i64> {
    add_text_item(
        tx,
        TextItemDraft {
            kind: PoolKind::Prompt,
            text: &prompt.text,
            tags: &prompt.tags,
            timeout_seconds: prompt.timeout_seconds,
            summary: None,
        },
    )
}

/// Appends a notification, returning its new editor row id.
pub fn add_notification(tx: &Transaction<'_>, notification: &Notification) -> Result<i64> {
    add_text_item(
        tx,
        TextItemDraft {
            kind: PoolKind::Notification,
            text: &notification.text,
            tags: &notification.tags,
            timeout_seconds: None,
            summary: notification.summary.as_deref(),
        },
    )
}

/// Removes an entry, leaving the rest of the pool in the order it was already in.
pub fn remove_text_item(tx: &Transaction<'_>, id: i64) -> Result<()> {
    tx.execute("DELETE FROM behaviour_text_item WHERE id = ?", params![id])?;
    Ok(())
}

// ── Text pool entries ────────────────────────────────────────────────────────

const NO_TEXT_ITEM: &str = "that entry is no longer in the pack";

fn set_pool_item_column<T: rusqlite::ToSql>(
    tx: &Transaction<'_>,
    kind: PoolKind,
    id: i64,
    column: &str,
    value: T,
) -> Result<()> {
    let updated = tx.execute(
        &format!("UPDATE behaviour_text_item SET {column} = ?3 WHERE id = ?1 AND kind = ?2"),
        params![id, kind, value],
    )?;

    if updated == 0 {
        bail!("{NO_TEXT_ITEM}");
    }

    Ok(())
}

pub fn set_caption_text(tx: &Transaction<'_>, id: i64, text: &str) -> Result<()> {
    set_pool_item_column(tx, PoolKind::Caption, id, "text", text)
}

pub fn set_prompt_text(tx: &Transaction<'_>, id: i64, text: &str) -> Result<()> {
    set_pool_item_column(tx, PoolKind::Prompt, id, "text", text)
}

pub fn set_notification_body(tx: &Transaction<'_>, id: i64, text: &str) -> Result<()> {
    set_pool_item_column(tx, PoolKind::Notification, id, "text", text)
}

/// The notification's title
pub fn set_notification_summary(
    tx: &Transaction<'_>,
    id: i64,
    summary: Option<&str>,
) -> Result<()> {
    set_pool_item_column(
        tx,
        PoolKind::Notification,
        id,
        "summary",
        summary.map(str::to_string),
    )
}

/// A prompt's answer deadline. `None` asks the mode to derive one from the prompt's length.
pub fn set_prompt_timeout(tx: &Transaction<'_>, id: i64, seconds: Option<f64>) -> Result<()> {
    set_pool_item_column(tx, PoolKind::Prompt, id, "timeout_seconds", seconds)
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
