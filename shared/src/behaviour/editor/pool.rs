use super::*;

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
         WHERE kind = ? ORDER BY position",
    )?;
    let rows: Vec<(i64, String, Option<f64>, Option<String>)> = statement
        .query_map(params![kind.as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, text, timeout_seconds, summary)| {
            Ok(TextItemRow {
                id,
                item: TextItem {
                    text,
                    tags: read_tags(conn, "behaviour_text_item_tag", "item_id", &id)?,
                    timeout_seconds,
                    summary,
                },
            })
        })
        .collect()
}

/// Appends an entry to `kind`'s pool, returning its new id.
pub fn add_text_item(tx: &Transaction<'_>, kind: PoolKind, item: &TextItem) -> Result<i64> {
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_text_item WHERE kind = ?",
        params![kind.as_str()],
        |row| row.get(0),
    )?;
    let id: i64 = tx.query_row(
        "INSERT INTO behaviour_text_item (kind, position, text, timeout_seconds, summary)
         VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
        params![
            kind.as_str(),
            position,
            item.text,
            item.timeout_seconds,
            item.summary
        ],
        |row| row.get(0),
    )?;
    write_tags(tx, "behaviour_text_item_tag", "item_id", &id, &item.tags)?;
    Ok(id)
}

/// Removes an entry and closes the gap its position left.
pub fn remove_text_item(tx: &Transaction<'_>, id: i64) -> Result<bool> {
    let found: Option<(String, i64)> = tx
        .query_row(
            "SELECT kind, position FROM behaviour_text_item WHERE id = ?",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((kind, position)) = found else {
        return Ok(false);
    };
    tx.execute("DELETE FROM behaviour_text_item WHERE id = ?", params![id])?;
    // Positions are a dense sequence the pool's order depends on, and `(kind, position)` is
    // unique -- so the gap has to close now rather than at the next full write.
    tx.execute(
        "UPDATE behaviour_text_item SET position = position - 1
         WHERE kind = ?1 AND position > ?2",
        params![kind, position],
    )?;
    Ok(true)
}

// ── Text pool entries ────────────────────────────────────────────────────────

const NO_TEXT_ITEM: &str = "that entry is no longer in the pack";

pub fn set_text_item_text(tx: &Transaction<'_>, id: i64, text: &str) -> Result<bool> {
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
pub fn set_text_item_summary(tx: &Transaction<'_>, id: i64, summary: Option<&str>) -> Result<bool> {
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
pub fn set_text_item_timeout(tx: &Transaction<'_>, id: i64, seconds: Option<f64>) -> Result<bool> {
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

pub fn add_text_item_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<bool> {
    exists(tx, "behaviour_text_item", "id", &id, NO_TEXT_ITEM)?;
    add_tag(tx, "behaviour_text_item_tag", "item_id", &id, tag)
}

pub fn remove_text_item_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<bool> {
    remove_tag(tx, "behaviour_text_item_tag", "item_id", &id, tag)
}
