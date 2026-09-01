use anyhow::{Result, bail};
use rusqlite::{Connection, Transaction, named_params, params};

use super::tags::{read_tags, write_tags};
use crate::behaviour::schema::{Caption, ContentGroup, Notification, Prompt, WebLink};

pub(crate) struct ContentEntryParts {
    value: String,
    tags: Vec<String>,
    timeout_seconds: Option<f64>,
    summary: Option<String>,
}

pub(crate) trait ContentEntry: Sized {
    const KIND: &'static str;

    fn try_from_parts(parts: ContentEntryParts) -> Result<Self>;
    fn value(&self) -> &str;
    fn tags(&self) -> &[String];
    fn timeout_seconds(&self) -> Option<f64> {
        None
    }
    fn summary(&self) -> Option<&str> {
        None
    }
}

impl ContentEntry for Caption {
    const KIND: &'static str = "caption";

    fn try_from_parts(parts: ContentEntryParts) -> Result<Self> {
        if parts.timeout_seconds.is_some() || parts.summary.is_some() {
            bail!("a caption row contains prompt or notification fields");
        }
        Ok(Self {
            text: parts.value,
            tags: parts.tags,
        })
    }

    fn value(&self) -> &str {
        &self.text
    }

    fn tags(&self) -> &[String] {
        &self.tags
    }
}

impl ContentEntry for Prompt {
    const KIND: &'static str = "prompt";

    fn try_from_parts(parts: ContentEntryParts) -> Result<Self> {
        if parts.summary.is_some() {
            bail!("a prompt row contains notification fields");
        }
        Ok(Self {
            text: parts.value,
            tags: parts.tags,
            timeout_seconds: parts.timeout_seconds,
        })
    }

    fn value(&self) -> &str {
        &self.text
    }

    fn tags(&self) -> &[String] {
        &self.tags
    }

    fn timeout_seconds(&self) -> Option<f64> {
        self.timeout_seconds
    }
}

impl ContentEntry for Notification {
    const KIND: &'static str = "notification";

    fn try_from_parts(parts: ContentEntryParts) -> Result<Self> {
        if parts.timeout_seconds.is_some() {
            bail!("a notification row contains prompt fields");
        }
        Ok(Self {
            text: parts.value,
            tags: parts.tags,
            summary: parts.summary,
        })
    }

    fn value(&self) -> &str {
        &self.text
    }

    fn tags(&self) -> &[String] {
        &self.tags
    }

    fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }
}

impl ContentEntry for WebLink {
    const KIND: &'static str = "web_link";

    fn try_from_parts(parts: ContentEntryParts) -> Result<Self> {
        if parts.timeout_seconds.is_some() || parts.summary.is_some() {
            bail!("a web-link row contains prompt or notification fields");
        }
        Ok(Self {
            url: parts.value,
            tags: parts.tags,
        })
    }

    fn value(&self) -> &str {
        &self.url
    }

    fn tags(&self) -> &[String] {
        &self.tags
    }
}

pub(crate) fn read_entries<T: ContentEntry>(conn: &Connection) -> Result<Vec<T>> {
    Ok(read_entry_rows(conn)?
        .into_iter()
        .map(|(_, item)| item)
        .collect())
}

pub(crate) fn read_entry_rows<T: ContentEntry>(conn: &Connection) -> Result<Vec<(i64, T)>> {
    let mut statement = conn.prepare(
        "SELECT id, value, timeout_seconds, summary FROM behaviour_content_entry WHERE kind = :kind \
         ORDER BY id",
    )?;

    statement
        .query_and_then(named_params![":kind": T::KIND], |row| {
            let id: i64 = row.get("id")?;

            Ok((
                id,
                T::try_from_parts(ContentEntryParts {
                    value: row.get("value")?,
                    tags: read_tags(conn, "behaviour_content_entry_tag", "entry_id", &id)?,
                    timeout_seconds: row.get("timeout_seconds")?,
                    summary: row.get("summary")?,
                })?,
            ))
        })?
        .collect()
}

pub(crate) fn insert_entry<T: ContentEntry>(tx: &Transaction<'_>, item: &T) -> Result<i64> {
    let id: i64 = tx.query_row(
        "INSERT INTO behaviour_content_entry (kind, value, timeout_seconds, summary)
         VALUES (?1, ?2, ?3, ?4)
         RETURNING id",
        params![
            T::KIND,
            item.value(),
            item.timeout_seconds(),
            item.summary()
        ],
        |row| row.get(0),
    )?;

    write_tags(
        tx,
        "behaviour_content_entry_tag",
        "entry_id",
        &id,
        item.tags(),
    )?;
    Ok(id)
}

pub(crate) fn write_entries<T: ContentEntry>(tx: &Transaction<'_>, items: &[T]) -> Result<()> {
    for item in items {
        insert_entry(tx, item)?;
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
