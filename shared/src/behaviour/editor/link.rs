use super::*;

/// One web link, with the row id the editor addresses it by.
///
/// Its suffixes carry their own ids rather than arriving as a bare list, because nothing else
/// identifies one: the list is ordered and may repeat, so a value names no particular entry, and an
/// index into the rendered array shifts the moment any other suffix goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebLinkRow {
    pub id: i64,
    pub url: String,
    pub tags: Vec<String>,
    pub args: Vec<WebLinkArg>,
}

/// One suffix appended at random when a link is opened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebLinkArg {
    /// Names this suffix for as long as it exists, and is never given to another.
    pub id: i64,
    pub value: String,
}
// ── Web links ────────────────────────────────────────────────────────────────

/// Every web link, in author order.
pub fn web_links(conn: &Connection) -> Result<Vec<WebLinkRow>> {
    let mut statement = conn.prepare("SELECT id, url FROM behaviour_web_link ORDER BY position")?;
    let rows: Vec<(i64, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, url)| {
            Ok(WebLinkRow {
                id,
                url,
                tags: read_tags(conn, "behaviour_web_link_tag", "link_id", &id)?,
                args: numbered_link_args(conn, id)?,
            })
        })
        .collect()
}

fn numbered_link_args(conn: &Connection, id: i64) -> Result<Vec<WebLinkArg>> {
    let mut statement = conn.prepare(
        "SELECT id, value FROM behaviour_web_link_arg WHERE link_id = ? ORDER BY position",
    )?;
    let args = statement
        .query_map(params![id], |row| {
            Ok(WebLinkArg {
                id: row.get(0)?,
                value: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(args)
}

/// Appends a web link, returning its new id.
pub fn add_web_link(tx: &Transaction<'_>, link: &WebLink) -> Result<i64> {
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_web_link",
        [],
        |row| row.get(0),
    )?;
    let id: i64 = tx.query_row(
        "INSERT INTO behaviour_web_link (position, url) VALUES (?1, ?2) RETURNING id",
        params![position, link.url],
        |row| row.get(0),
    )?;
    write_link_args(tx, id, &link.args)?;
    write_tags(tx, "behaviour_web_link_tag", "link_id", &id, &link.tags)?;
    Ok(id)
}

fn write_link_args(tx: &Transaction<'_>, id: i64, args: &[String]) -> Result<()> {
    // A set of suffixes is ordered but not unique -- the same suffix twice is how a pack weights
    // it -- so `position` is part of the key and the tail is truncated rather than diffed.
    tx.execute(
        "DELETE FROM behaviour_web_link_arg WHERE link_id = ?1 AND position >= ?2",
        params![id, args.len() as i64],
    )?;
    for (position, value) in args.iter().enumerate() {
        tx.execute(
            "INSERT INTO behaviour_web_link_arg (link_id, position, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(link_id, position) DO UPDATE SET value = excluded.value",
            params![id, position as i64, value],
        )?;
    }
    Ok(())
}

/// Removes a web link and closes the gap its position left.
pub fn remove_web_link(tx: &Transaction<'_>, id: i64) -> Result<bool> {
    let position: Option<i64> = tx
        .query_row(
            "SELECT position FROM behaviour_web_link WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(position) = position else {
        return Ok(false);
    };
    tx.execute("DELETE FROM behaviour_web_link WHERE id = ?", params![id])?;
    tx.execute(
        "UPDATE behaviour_web_link SET position = position - 1 WHERE position > ?",
        params![position],
    )?;
    Ok(true)
}

// ── Web links ────────────────────────────────────────────────────────────────

const NO_WEB_LINK: &str = "that web link is no longer in the pack";

pub fn set_web_link_url(tx: &Transaction<'_>, id: i64, url: &str) -> Result<bool> {
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

pub fn add_web_link_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<bool> {
    exists(tx, "behaviour_web_link", "id", &id, NO_WEB_LINK)?;
    add_tag(tx, "behaviour_web_link_tag", "link_id", &id, tag)
}

pub fn remove_web_link_tag(tx: &Transaction<'_>, id: i64, tag: &str) -> Result<bool> {
    remove_tag(tx, "behaviour_web_link_tag", "link_id", &id, tag)
}

/// Appends one suffix to a web link's list.
pub fn add_web_link_arg(tx: &Transaction<'_>, id: i64, value: &str) -> Result<bool> {
    exists(tx, "behaviour_web_link", "id", &id, NO_WEB_LINK)?;
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_web_link_arg WHERE link_id = ?",
        params![id],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT INTO behaviour_web_link_arg (link_id, position, value) VALUES (?1, ?2, ?3)",
        params![id, position, value],
    )?;
    Ok(true)
}

/// Removes one suffix, by its own id.
///
/// Not by value, because the same suffix twice is a legitimate way to weight it — the list is
/// ordered and not a set. Not by position either: a position says where a suffix sits, and where it
/// sits changes. The id is the only thing about a suffix that does not move.
///
/// The gap the removal leaves in `position` is not closed. Nothing needs the numbers to be dense —
/// they only order the list — and closing it would move every suffix after this one, which is what
/// the id exists to stop mattering.
pub fn remove_web_link_arg(tx: &Transaction<'_>, link: i64, arg: i64) -> Result<bool> {
    let removed = tx.execute(
        "DELETE FROM behaviour_web_link_arg WHERE link_id = ?1 AND id = ?2",
        params![link, arg],
    )?;
    Ok(removed > 0)
}
