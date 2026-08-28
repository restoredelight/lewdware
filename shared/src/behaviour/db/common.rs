use std::rc::Rc;

use anyhow::{Result, bail};
use rusqlite::types::{ToSqlOutput, Value, ValueRef};
use rusqlite::vtab::array::Array;
use rusqlite::{Transaction, params};

pub fn sql_array<T: rusqlite::ToSql>(values: impl IntoIterator<Item = T>) -> Result<Array> {
    let mut array = Vec::new();

    for value in values {
        let sql_value = match value.to_sql()? {
            ToSqlOutput::Borrowed(vref) => match vref {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(i) => Value::Integer(i),
                ValueRef::Real(r) => Value::Real(r),
                ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
                ValueRef::Blob(b) => Value::Blob(b.to_vec()),
            },
            ToSqlOutput::Owned(val) => val,
            _ => bail!("unsupported ToSqlOutput variant for array"),
        };

        array.push(sql_value);
    }

    Ok(Rc::new(array))
}

/// Deletes every row of `table` whose `key` column is not in `keep`.
pub fn retain<'a>(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    keep: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let keep: Vec<&str> = keep.collect();
    tx.execute(
        &format!("DELETE FROM {table} WHERE {key} NOT IN (SELECT value FROM rarray(?))"),
        params![sql_array(keep)?],
    )?;
    Ok(())
}
