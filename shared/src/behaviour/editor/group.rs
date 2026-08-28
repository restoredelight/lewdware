use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::common::{add_tag, remove_tag, set_column};
use crate::behaviour::db::{read_content_groups, write_tags};
use crate::behaviour::schema::ContentGroup;

// ── Content groups ───────────────────────────────────────────────────────────

/// Every content group, in author order.
pub fn content_groups(conn: &Connection) -> Result<Vec<ContentGroup>> {
    read_content_groups(conn)
}

/// Adds a content group, appending it to the list.
///
/// `ContentGroup::id` is the author-facing key the resolver turns into a mode option
/// (`content_group.<id>`), so a collision is the author's to resolve, not ours to paper over. The
/// `UNIQUE` constraint on `key` is what actually catches one; the check ahead of it is only there
/// to name the group in the message.
pub fn add_content_group(tx: &Transaction<'_>, group: &ContentGroup) -> Result<()> {
    let taken: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM behaviour_content_group WHERE key = ?)",
        params![group.id],
        |row| row.get(0),
    )?;
    if taken {
        bail!("a content group named “{}” already exists", group.id);
    }
    let row_id: i64 = tx.query_row(
        "INSERT INTO behaviour_content_group (key, label, description, enabled_by_default)
         VALUES (?1, ?2, ?3, ?4) RETURNING id",
        params![
            group.id,
            group.label,
            group.description,
            group.enabled_by_default
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
    Ok(())
}

/// Removes a content group, leaving the rest in the order they were already in. Its tag rows go
/// with it, by cascade.
pub fn remove_content_group(tx: &Transaction<'_>, id: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM behaviour_content_group WHERE key = ?",
        params![id],
    )?;
    Ok(())
}

// ── Content groups ───────────────────────────────────────────────────────────

const NO_GROUP: &str = "that content group is no longer in the pack";

pub fn set_content_group_label(tx: &Transaction<'_>, id: &str, label: &str) -> Result<()> {
    set_column(
        tx,
        "behaviour_content_group",
        "key",
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
) -> Result<()> {
    set_column(
        tx,
        "behaviour_content_group",
        "key",
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
) -> Result<()> {
    set_column(
        tx,
        "behaviour_content_group",
        "key",
        &id,
        "enabled_by_default",
        enabled,
        NO_GROUP,
    )
}

/// The surrogate `behaviour_content_group.id` for an author-facing key, which is what the tag
/// rows point at. Absent means the group has been removed under a stale editor.
fn group_row_id(tx: &Transaction<'_>, id: &str) -> Result<i64> {
    tx.query_row(
        "SELECT id FROM behaviour_content_group WHERE key = ?",
        params![id],
        |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("{NO_GROUP}"))
}

pub fn add_content_group_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<()> {
    let row_id = group_row_id(tx, id)?;
    add_tag(tx, "behaviour_content_group_tag", "group_id", &row_id, tag)
}

pub fn remove_content_group_tag(tx: &Transaction<'_>, id: &str, tag: &str) -> Result<()> {
    let row_id = group_row_id(tx, id)?;
    remove_tag(tx, "behaviour_content_group_tag", "group_id", &row_id, tag)
}
