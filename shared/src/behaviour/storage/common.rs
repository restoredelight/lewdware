use std::rc::Rc;

use anyhow::{Result, bail};
use rusqlite::types::Value;
use rusqlite::vtab::array::Array;
use rusqlite::{Transaction, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub fn integer_array(values: impl Iterator<Item = i64>) -> Array {
    Rc::new(values.map(Value::Integer).collect())
}

pub fn text_array<'a>(values: impl Iterator<Item = &'a str>) -> Array {
    Rc::new(values.map(|value| Value::Text(value.to_string())).collect())
}

pub fn id_array(values: &[u64]) -> Result<Array> {
    Ok(integer_array(
        values
            .iter()
            .map(|&value| i64::try_from(value))
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter(),
    ))
}

/// [`retain`] for a numeric key column. Separate because the ids never exist as strings anywhere
/// else, and stringifying them just to satisfy one signature would invite the comparison to be
/// made against SQLite's text affinity by accident.
pub fn retain_ids(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    keep: &[u64],
) -> Result<()> {
    tx.execute(
        &format!("DELETE FROM {table} WHERE {key} NOT IN (SELECT value FROM rarray(?))"),
        params![id_array(keep)?],
    )?;
    Ok(())
}

/// Deletes every row of `table` whose `key` column is not in `keep`.
///
/// The delete half of the diff. Passing the surviving keys as an array rather than building an `IN
/// (?, ?, …)` keeps one prepared statement whatever the timeline's length.
pub fn retain<'a>(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    keep: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let keep: Vec<&str> = keep.collect();
    tx.execute(
        &format!("DELETE FROM {table} WHERE {key} NOT IN (SELECT value FROM rarray(?))"),
        params![text_array(keep.into_iter())],
    )?;
    Ok(())
}

/// An enum's database text, which is deliberately the same string its JSON uses.
///
/// Routed through serde rather than a hand-written match so the two representations cannot drift:
/// renaming a variant's `rename_all` spelling changes both at once.
pub fn to_text<T: Serialize>(value: &T) -> Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(text) => Ok(text),
        other => bail!("expected a string-valued enum, got {other}"),
    }
}

pub fn from_text<T: DeserializeOwned>(text: &str) -> Result<T> {
    Ok(serde_json::from_value(serde_json::Value::String(
        text.to_string(),
    ))?)
}
