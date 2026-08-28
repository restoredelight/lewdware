use std::rc::Rc;

use anyhow::{Result, bail};
use rusqlite::types::Value;
use rusqlite::vtab::array::Array;
use rusqlite::{Transaction, params};

pub fn integer_array(values: impl Iterator<Item = i64>) -> Array {
    Rc::new(values.map(Value::Integer).collect())
}

pub fn text_array<'a>(values: impl Iterator<Item = &'a str>) -> Array {
    Rc::new(values.map(|value| Value::Text(value.to_string())).collect())
}

pub fn sql_array<T: rusqlite::ToSql>(values: impl Iterator<Item = T>) -> Result<Array> {
    let mut array = Vec::new();
    for value in values {
        let sql_value = match value.to_sql()? {
            rusqlite::types::ToSqlOutput::Borrowed(vref) => match vref {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(i) => Value::Integer(i),
                rusqlite::types::ValueRef::Real(r) => Value::Real(r),
                rusqlite::types::ValueRef::Text(t) => {
                    Value::Text(String::from_utf8_lossy(t).into_owned())
                }
                rusqlite::types::ValueRef::Blob(b) => Value::Blob(b.to_vec()),
            },
            rusqlite::types::ToSqlOutput::Owned(val) => val,
            _ => bail!("unsupported ToSqlOutput variant for array"),
        };
        array.push(sql_value);
    }
    Ok(Rc::new(array))
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
        params![text_array(keep.into_iter())],
    )?;
    Ok(())
}
