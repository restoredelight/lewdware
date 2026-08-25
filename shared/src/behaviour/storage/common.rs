use super::*;

/// [`retain`] for a numeric key column. Separate because the ids never exist as strings anywhere
/// else, and stringifying them just to satisfy one signature would invite the comparison to be
/// made against SQLite's text affinity by accident.
pub(in crate::behaviour) fn retain_ids(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    keep: &[u64],
) -> Result<()> {
    tx.execute(
        &format!("DELETE FROM {table} WHERE {key} NOT IN (SELECT value FROM json_each(?))"),
        params![serde_json::to_string(keep)?],
    )?;
    Ok(())
}

/// Deletes every row of `table` whose `key` column is not in `keep`.
///
/// The delete half of the diff. Passing the surviving keys as JSON rather than building an `IN
/// (?, ?, …)` keeps one prepared statement whatever the timeline's length.
pub(in crate::behaviour) fn retain<'a>(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    keep: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let keep: Vec<&str> = keep.collect();
    tx.execute(
        &format!("DELETE FROM {table} WHERE {key} NOT IN (SELECT value FROM json_each(?))"),
        params![serde_json::to_string(&keep)?],
    )?;
    Ok(())
}

/// An enum's database text, which is deliberately the same string its JSON uses.
///
/// Routed through serde rather than a hand-written match so the two representations cannot drift:
/// renaming a variant's `rename_all` spelling changes both at once.
pub(in crate::behaviour) fn to_text<T: Serialize>(value: &T) -> Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(text) => Ok(text),
        other => bail!("expected a string-valued enum, got {other}"),
    }
}

pub(in crate::behaviour) fn from_text<T: DeserializeOwned>(text: &str) -> Result<T> {
    Ok(serde_json::from_value(serde_json::Value::String(
        text.to_string(),
    ))?)
}
