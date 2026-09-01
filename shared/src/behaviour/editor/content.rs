use anyhow::{Result, bail};
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

use super::common::{add_tag, remove_tag};
use crate::behaviour::db::{ContentEntry, insert_entry, read_entry_rows};
use crate::behaviour::schema::{Caption, Notification, Prompt, WebLink};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionRecord {
    pub id: i64,
    pub text: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptRecord {
    pub id: i64,
    pub text: String,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationRecord {
    pub id: i64,
    pub text: String,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebLinkRecord {
    pub id: i64,
    pub url: String,
    pub tags: Vec<String>,
}

pub fn captions(conn: &Connection) -> Result<Vec<CaptionRecord>> {
    Ok(read_entry_rows::<Caption>(conn)?
        .into_iter()
        .map(|(id, caption)| CaptionRecord {
            id,
            text: caption.text,
            tags: caption.tags,
        })
        .collect())
}

pub fn prompts(conn: &Connection) -> Result<Vec<PromptRecord>> {
    Ok(read_entry_rows::<Prompt>(conn)?
        .into_iter()
        .map(|(id, prompt)| PromptRecord {
            id,
            text: prompt.text,
            tags: prompt.tags,
            timeout_seconds: prompt.timeout_seconds,
        })
        .collect())
}

pub fn notifications(conn: &Connection) -> Result<Vec<NotificationRecord>> {
    Ok(read_entry_rows::<Notification>(conn)?
        .into_iter()
        .map(|(id, notification)| NotificationRecord {
            id,
            text: notification.text,
            tags: notification.tags,
            summary: notification.summary,
        })
        .collect())
}

pub fn web_links(conn: &Connection) -> Result<Vec<WebLinkRecord>> {
    Ok(read_entry_rows::<WebLink>(conn)?
        .into_iter()
        .map(|(id, link)| WebLinkRecord {
            id,
            url: link.url,
            tags: link.tags,
        })
        .collect())
}

pub fn add_caption(tx: &Transaction<'_>, caption: &Caption) -> Result<i64> {
    insert_entry(tx, caption)
}

pub fn add_prompt(tx: &Transaction<'_>, prompt: &Prompt) -> Result<i64> {
    insert_entry(tx, prompt)
}

pub fn add_notification(tx: &Transaction<'_>, notification: &Notification) -> Result<i64> {
    insert_entry(tx, notification)
}

pub fn add_web_link(tx: &Transaction<'_>, link: &WebLink) -> Result<i64> {
    insert_entry(tx, link)
}

fn remove_entry<T: ContentEntry>(tx: &Transaction<'_>, id: i64) -> Result<()> {
    tx.execute(
        "DELETE FROM behaviour_content_entry WHERE id = ?1 AND kind = ?2",
        params![id, T::KIND],
    )?;
    Ok(())
}

pub fn remove_caption(tx: &Transaction<'_>, id: i64) -> Result<()> {
    remove_entry::<Caption>(tx, id)
}

pub fn remove_prompt(tx: &Transaction<'_>, id: i64) -> Result<()> {
    remove_entry::<Prompt>(tx, id)
}

pub fn remove_notification(tx: &Transaction<'_>, id: i64) -> Result<()> {
    remove_entry::<Notification>(tx, id)
}

pub fn remove_web_link(tx: &Transaction<'_>, id: i64) -> Result<()> {
    remove_entry::<WebLink>(tx, id)
}

fn set_entry_column<T: ContentEntry, V: rusqlite::ToSql>(
    tx: &Transaction<'_>,
    id: i64,
    column: &str,
    value: V,
    missing: &str,
) -> Result<()> {
    let updated = tx.execute(
        &format!("UPDATE behaviour_content_entry SET {column} = ?3 WHERE id = ?1 AND kind = ?2"),
        params![id, T::KIND, value],
    )?;

    if updated == 0 {
        bail!("{missing}");
    }
    Ok(())
}

pub fn set_caption_text(tx: &Transaction<'_>, id: i64, text: &str) -> Result<()> {
    set_entry_column::<Caption, _>(
        tx,
        id,
        "value",
        text,
        "that caption is no longer in the pack",
    )
}

pub fn set_prompt_text(tx: &Transaction<'_>, id: i64, text: &str) -> Result<()> {
    set_entry_column::<Prompt, _>(
        tx,
        id,
        "value",
        text,
        "that prompt is no longer in the pack",
    )
}

pub fn set_notification_body(tx: &Transaction<'_>, id: i64, text: &str) -> Result<()> {
    set_entry_column::<Notification, _>(
        tx,
        id,
        "value",
        text,
        "that notification is no longer in the pack",
    )
}

pub fn set_notification_summary(
    tx: &Transaction<'_>,
    id: i64,
    summary: Option<&str>,
) -> Result<()> {
    set_entry_column::<Notification, _>(
        tx,
        id,
        "summary",
        summary.map(str::to_string),
        "that notification is no longer in the pack",
    )
}

pub fn set_prompt_timeout(tx: &Transaction<'_>, id: i64, seconds: Option<f64>) -> Result<()> {
    set_entry_column::<Prompt, _>(
        tx,
        id,
        "timeout_seconds",
        seconds,
        "that prompt is no longer in the pack",
    )
}

pub fn set_web_link_url(tx: &Transaction<'_>, id: i64, url: &str) -> Result<()> {
    set_entry_column::<WebLink, _>(
        tx,
        id,
        "value",
        url,
        "that web link is no longer in the pack",
    )
}

fn entry_exists<T: ContentEntry>(tx: &Transaction<'_>, id: i64) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM behaviour_content_entry WHERE id = ?1 AND kind = ?2)",
        params![id, T::KIND],
        |row| row.get(0),
    )?)
}

fn add_entry_tag<T: ContentEntry>(
    tx: &Transaction<'_>,
    id: i64,
    tag: &str,
    missing: &str,
) -> Result<()> {
    if !entry_exists::<T>(tx, id)? {
        bail!("{missing}");
    }
    add_tag(tx, "behaviour_content_entry_tag", "entry_id", &id, tag)
}

fn remove_entry_tag<T: ContentEntry>(
    tx: &Transaction<'_>,
    id: i64,
    tag: &str,
    missing: &str,
) -> Result<()> {
    if !entry_exists::<T>(tx, id)? {
        bail!("{missing}");
    }
    remove_tag(tx, "behaviour_content_entry_tag", "entry_id", &id, tag)
}

macro_rules! entry_tag_functions {
    ($add:ident, $remove:ident, $ty:ty, $missing:literal) => {
        pub fn $add(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<()> {
            add_entry_tag::<$ty>(tx, id, tag, $missing)
        }

        pub fn $remove(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<()> {
            remove_entry_tag::<$ty>(tx, id, tag, $missing)
        }
    };
}

entry_tag_functions!(
    add_caption_tag,
    remove_caption_tag,
    Caption,
    "that caption is no longer in the pack"
);
entry_tag_functions!(
    add_prompt_tag,
    remove_prompt_tag,
    Prompt,
    "that prompt is no longer in the pack"
);
entry_tag_functions!(
    add_notification_tag,
    remove_notification_tag,
    Notification,
    "that notification is no longer in the pack"
);
entry_tag_functions!(
    add_web_link_tag,
    remove_web_link_tag,
    WebLink,
    "that web link is no longer in the pack"
);
