use anyhow::{Result, bail};
use rusqlite::{Connection, Transaction, named_params, params};

use super::tags::{read_tags, write_tags};
use crate::behaviour::schema::{Caption, ContentGroup, Notification, Prompt, WebLink};

pub(crate) struct TextItemParts {
    text: String,
    tags: Vec<String>,
    timeout_seconds: Option<f64>,
    summary: Option<String>,
}

pub(crate) trait TextPoolItem: Sized {
    const KIND: &'static str;

    fn try_from_parts(parts: TextItemParts) -> Result<Self>;
    fn text(&self) -> &str;
    fn tags(&self) -> &[String];
    fn timeout_seconds(&self) -> Option<f64> {
        None
    }
    fn summary(&self) -> Option<&str> {
        None
    }
}

impl TextPoolItem for Caption {
    const KIND: &'static str = "caption";

    fn try_from_parts(parts: TextItemParts) -> Result<Self> {
        if parts.timeout_seconds.is_some() || parts.summary.is_some() {
            bail!("a caption row contains prompt or notification fields");
        }
        Ok(Self {
            text: parts.text,
            tags: parts.tags,
        })
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn tags(&self) -> &[String] {
        &self.tags
    }
}

impl TextPoolItem for Prompt {
    const KIND: &'static str = "prompt";

    fn try_from_parts(parts: TextItemParts) -> Result<Self> {
        if parts.summary.is_some() {
            bail!("a prompt row contains notification fields");
        }
        Ok(Self {
            text: parts.text,
            tags: parts.tags,
            timeout_seconds: parts.timeout_seconds,
        })
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn tags(&self) -> &[String] {
        &self.tags
    }

    fn timeout_seconds(&self) -> Option<f64> {
        self.timeout_seconds
    }
}

impl TextPoolItem for Notification {
    const KIND: &'static str = "notification";

    fn try_from_parts(parts: TextItemParts) -> Result<Self> {
        if parts.timeout_seconds.is_some() {
            bail!("a notification row contains prompt fields");
        }
        Ok(Self {
            text: parts.text,
            tags: parts.tags,
            summary: parts.summary,
        })
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn tags(&self) -> &[String] {
        &self.tags
    }

    fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }
}

pub(crate) fn read_text_pool<T: TextPoolItem>(conn: &Connection) -> Result<Vec<T>> {
    Ok(read_text_pool_rows(conn)?
        .into_iter()
        .map(|(_, item)| item)
        .collect())
}

pub(crate) fn read_text_pool_rows<T: TextPoolItem>(conn: &Connection) -> Result<Vec<(i64, T)>> {
    let mut statement = conn.prepare(
        "SELECT id, text, timeout_seconds, summary FROM behaviour_text_item WHERE kind = :kind \
         ORDER BY id",
    )?;

    statement
        .query_and_then(named_params![":kind": T::KIND], |row| {
            let id: i64 = row.get("id")?;

            Ok((
                id,
                T::try_from_parts(TextItemParts {
                    text: row.get("text")?,
                    tags: read_tags(conn, "behaviour_text_item_tag", "item_id", &id)?,
                    timeout_seconds: row.get("timeout_seconds")?,
                    summary: row.get("summary")?,
                })?,
            ))
        })?
        .collect()
}

pub(crate) fn write_text_pool<T: TextPoolItem>(tx: &Transaction<'_>, items: &[T]) -> Result<()> {
    for item in items {
        let id: i64 = tx.query_row(
            "INSERT INTO behaviour_text_item (kind, text, timeout_seconds, summary)
             VALUES (?1, ?2, ?3, ?4)
             RETURNING id",
            params![T::KIND, item.text(), item.timeout_seconds(), item.summary()],
            |row| row.get(0),
        )?;

        write_tags(tx, "behaviour_text_item_tag", "item_id", &id, item.tags())?;
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
