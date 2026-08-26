use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::common::{add_tag, exists, remove_tag, set_column};
use crate::behaviour::schema::ContentGroup;
use crate::behaviour::storage::{read_content_groups, write_tags};

// ── Content groups ───────────────────────────────────────────────────────────

/// Every content group, in author order.
pub fn content_groups(conn: &Connection) -> Result<Vec<ContentGroup>> {
    read_content_groups(conn)
}

/// Adds a content group. Its `id` is the author-facing key the resolver turns into a mode option
/// (`content_group.<id>`), so a collision is the author's to resolve, not ours to paper over.
pub fn add_content_group(tx: &Transaction<'_>, group: &ContentGroup) -> Result<()> {
    let taken: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM behaviour_content_group WHERE id = ?)",
        params![group.id],
        |row| row.get(0),
    )?;
    if taken {
        bail!("a content group named “{}” already exists", group.id);
    }
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_content_group",
        [],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT INTO behaviour_content_group (id, position, label, description, enabled_by_default)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            group.id,
            position,
            group.label,
            group.description,
            group.enabled_by_default
        ],
    )?;
    write_tags(
        tx,
        "behaviour_content_group_tag",
        "group_id",
        &group.id,
        &group.tags,
    )?;
    Ok(())
}

/// Removes a content group and closes the gap its position left.
pub fn remove_content_group(tx: &Transaction<'_>, id: &str) -> Result<bool> {
    let position: Option<i64> = tx
        .query_row(
            "SELECT position FROM behaviour_content_group WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(position) = position else {
        return Ok(false);
    };
    tx.execute(
        "DELETE FROM behaviour_content_group WHERE id = ?",
        params![id],
    )?;
    tx.execute(
        "UPDATE behaviour_content_group SET position = position - 1 WHERE position > ?",
        params![position],
    )?;
    Ok(true)
}

// ── Content groups ───────────────────────────────────────────────────────────

const NO_GROUP: &str = "that content group is no longer in the pack";

pub fn set_content_group_label(tx: &Transaction<'_>, id: &str, label: &str) -> Result<bool> {
    set_column(
        tx,
        "behaviour_content_group",
        "id",
        &id,
        "label",
        label.to_string(),
        NO_GROUP,
    )
}

pub fn set_content_group_description(
    tx: &Transaction<'_>,
    id: &str,
    description: Option<&str>,
) -> Result<bool> {
    set_column(
        tx,
        "behaviour_content_group",
        "id",
        &id,
        "description",
        description.map(str::to_string),
        NO_GROUP,
    )
}

pub fn set_content_group_enabled_by_default(
    tx: &Transaction<'_>,
    id: &str,
    enabled: bool,
) -> Result<bool> {
    set_column(
        tx,
        "behaviour_content_group",
        "id",
        &id,
        "enabled_by_default",
        enabled,
        NO_GROUP,
    )
}

pub fn add_content_group_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<bool> {
    exists(tx, "behaviour_content_group", "id", &id, NO_GROUP)?;
    add_tag(tx, "behaviour_content_group_tag", "group_id", &id, tag)
}

pub fn remove_content_group_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<bool> {
    remove_tag(tx, "behaviour_content_group_tag", "group_id", &id, tag)
}
