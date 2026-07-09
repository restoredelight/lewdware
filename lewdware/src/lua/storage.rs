use std::{cell::RefCell, ffi::c_void, path::PathBuf, rc::Rc, time::Duration};

use anyhow::Context;
use indexmap::IndexMap;
use mlua::{Lua, LuaSerdeExt, Value as LuaValue};
use rusqlite::Connection;
use serde_json::Value as JsonValue;
use tokio::task::JoinHandle;

/// Total bytes a mode's storage may hold. Deliberately generous -- this is meant to catch modes
/// that accidentally accumulate unbounded state (e.g. logging every event forever), not to be a
/// tight budget mode authors need to think about day to day.
const MAX_TOTAL_BYTES: usize = 1_048_576;

/// A single value may not itself exceed the total quota -- if it did, no amount of evicting other
/// keys could make it fit, so this is a hard error rather than a silent eviction.
const MAX_ITEM_BYTES: usize = MAX_TOTAL_BYTES;

/// Persistent per-mode key/value storage backing `lewdware.storage`. Cheaply `Clone`-able (an
/// `Rc`), matching `MediaManager`/`Windows`/etc: every clone shares the same in-memory cache and
/// SQLite connection, which is fine since everything here only ever runs on the Lua thread.
#[derive(Clone)]
pub struct Storage {
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    conn: Connection,
    /// Authoritative in-memory copy of every stored key/value, in last-written order (oldest
    /// first) -- `set` moves a key to the end, so the front is always the best eviction
    /// candidate. Reads (`get`/`keys`) don't affect this order: only writes count as "used"
    /// for quota purposes, matching a mode's actual intent (e.g. read-heavy config keys
    /// shouldn't be protected from eviction just because they're read every frame).
    cache: IndexMap<String, JsonValue>,
    total_bytes: usize,
    flush_task: Option<JoinHandle<()>>,
}

fn storage_path(scope_key: &str) -> anyhow::Result<PathBuf> {
    let mut path = dirs::data_local_dir()
        .context("Could not find a valid data dir for this OS")?;

    path.push("lewdware");
    path.push("mode-storage");
    std::fs::create_dir_all(&path)?;

    path.push(format!("{scope_key}.db"));

    Ok(path)
}

fn byte_size(key: &str, value: &JsonValue) -> usize {
    key.len() + serde_json::to_string(value).map(|s| s.len()).unwrap_or(0)
}

impl Storage {
    /// Opens (creating if necessary) the storage file for `scope_key` -- a mode's own stable
    /// UUID for a standalone mode, or that UUID combined with the pack's UUID for a mode embedded
    /// in a pack (see `start_lua_thread`, the only caller, for exactly how `scope_key` is built).
    pub fn open(scope_key: &str) -> anyhow::Result<Self> {
        Self::open_at(storage_path(scope_key)?)
    }

    /// The actual implementation behind `open`, taking a file path directly so tests don't need
    /// to touch the real per-OS data directory.
    pub(super) fn open_at(path: PathBuf) -> anyhow::Result<Self> {
        let conn = Connection::open(&path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;

        let mut cache = IndexMap::new();
        let mut total_bytes = 0;
        {
            let mut stmt = conn.prepare("SELECT key, value FROM kv ORDER BY rowid")?;
            let rows = stmt.query_map([], |row| {
                let key: String = row.get(0)?;
                let value: String = row.get(1)?;
                Ok((key, value))
            })?;

            for row in rows {
                let (key, value) = row?;
                match serde_json::from_str::<JsonValue>(&value) {
                    Ok(value) => {
                        total_bytes += byte_size(&key, &value);
                        cache.insert(key, value);
                    }
                    Err(err) => {
                        tracing::warn!(
                            "lewdware.storage: skipping unreadable value for key '{key}': {err}"
                        );
                    }
                }
            }
        }

        Ok(Self {
            inner: Rc::new(RefCell::new(Inner {
                conn,
                cache,
                total_bytes,
                flush_task: None,
            })),
        })
    }

    pub fn get(&self, lua: &Lua, key: &str) -> mlua::Result<LuaValue> {
        match self.inner.borrow().cache.get(key) {
            Some(value) => lua.to_value(value),
            None => Ok(LuaValue::Nil),
        }
    }

    pub fn keys(&self) -> Vec<String> {
        self.inner.borrow().cache.keys().cloned().collect()
    }

    pub fn set(&self, key: String, value: LuaValue) -> mlua::Result<()> {
        let json_value = lua_to_json(&value)?;
        let size = byte_size(&key, &json_value);

        if size > MAX_ITEM_BYTES {
            return Err(mlua::Error::RuntimeError(format!(
                "value for key '{key}' is {size} bytes, which is over the {MAX_ITEM_BYTES}-byte \
                 limit for a single lewdware.storage value"
            )));
        }

        let mut inner = self.inner.borrow_mut();

        // Remove any existing entry first: its old size shouldn't double-count, and re-inserting
        // moves the key to the end (most-recently-written), even when just overwriting a value.
        if let Some(old) = inner.cache.shift_remove(&key) {
            inner.total_bytes -= byte_size(&key, &old);
        }

        while inner.total_bytes + size > MAX_TOTAL_BYTES {
            let Some((evicted_key, evicted_value)) = inner.cache.shift_remove_index(0) else {
                break;
            };
            inner.total_bytes -= byte_size(&evicted_key, &evicted_value);
            tracing::warn!(
                "lewdware.storage: evicted key '{evicted_key}' to make room for '{key}' \
                 (total storage would otherwise exceed {MAX_TOTAL_BYTES} bytes)"
            );
        }

        inner.total_bytes += size;
        inner.cache.insert(key, json_value);
        drop(inner);

        schedule_flush(&self.inner);

        Ok(())
    }

    pub fn remove(&self, key: &str) -> bool {
        let mut inner = self.inner.borrow_mut();
        let Some(value) = inner.cache.shift_remove(key) else {
            return false;
        };
        inner.total_bytes -= byte_size(key, &value);
        drop(inner);

        schedule_flush(&self.inner);
        true
    }

    pub fn clear(&self) {
        let mut inner = self.inner.borrow_mut();
        if inner.cache.is_empty() {
            return;
        }
        inner.cache.clear();
        inner.total_bytes = 0;
        drop(inner);

        schedule_flush(&self.inner);
    }

    /// Synchronously flushes any pending write. Called once, on Lua thread shutdown, since a
    /// debounced flush scheduled on the (about to be dropped) `LocalSet` would otherwise never
    /// run, silently losing the last write in a session.
    pub fn flush_now(&self) -> anyhow::Result<()> {
        let mut inner = self.inner.borrow_mut();
        if let Some(task) = inner.flush_task.take() {
            task.abort();
        }
        flush(&mut inner)
    }
}

fn schedule_flush(inner_rc: &Rc<RefCell<Inner>>) {
    let mut inner = inner_rc.borrow_mut();
    if let Some(task) = inner.flush_task.take() {
        task.abort();
    }

    let inner_rc = inner_rc.clone();
    inner.flush_task = Some(tokio::task::spawn_local(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut inner = inner_rc.borrow_mut();
        inner.flush_task = None;
        if let Err(err) = flush(&mut inner) {
            tracing::warn!("Failed to flush lewdware.storage: {err}");
        }
    }));
}

fn flush(inner: &mut Inner) -> anyhow::Result<()> {
    let tx = inner.conn.transaction()?;
    tx.execute("DELETE FROM kv", [])?;
    {
        let mut stmt = tx.prepare("INSERT INTO kv (key, value) VALUES (?, ?)")?;
        for (key, value) in &inner.cache {
            stmt.execute(rusqlite::params![key, serde_json::to_string(value)?])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Converts a Lua value into JSON for storage, recursively validating along the way: only
/// booleans, numbers, strings and tables (with string/number keys, no cycles) are storable --
/// functions, userdata, threads, etc. raise a Lua error, as does a table that contains itself.
fn lua_to_json(value: &LuaValue) -> mlua::Result<JsonValue> {
    let mut ancestors = Vec::new();
    convert(value, &mut ancestors)
}

fn convert(value: &LuaValue, ancestors: &mut Vec<*const c_void>) -> mlua::Result<JsonValue> {
    match value {
        LuaValue::Nil => Ok(JsonValue::Null),
        LuaValue::Boolean(b) => Ok(JsonValue::Bool(*b)),
        LuaValue::Integer(i) => Ok(JsonValue::from(*i)),
        LuaValue::Number(n) => Ok(JsonValue::from(*n)),
        LuaValue::String(s) => Ok(JsonValue::String(s.to_str()?.to_string())),
        LuaValue::Table(table) => {
            let ptr = table.to_pointer();
            if ancestors.contains(&ptr) {
                return Err(mlua::Error::RuntimeError(
                    "lewdware.storage values may not contain cyclic tables".to_string(),
                ));
            }
            ancestors.push(ptr);

            let result = table_to_json(table, ancestors);

            ancestors.pop();
            result
        }
        _ => Err(mlua::Error::RuntimeError(format!(
            "lewdware.storage values may only contain booleans, numbers, strings and tables of \
             those -- got {}",
            value.type_name()
        ))),
    }
}

fn table_to_json(
    table: &mlua::Table,
    ancestors: &mut Vec<*const c_void>,
) -> mlua::Result<JsonValue> {
    let raw_len = table.raw_len();
    let pair_count = table.pairs::<LuaValue, LuaValue>().count();

    // A table whose only keys are a contiguous 1..=n sequence is encoded as a JSON array;
    // anything else (string keys, sparse/non-sequential number keys, or a mix) becomes an
    // object, with number keys stringified -- the standard, if lossy, Lua<->JSON convention (the
    // same one cjson and friends use).
    if pair_count == raw_len && raw_len > 0 {
        let mut items = Vec::with_capacity(raw_len);
        for i in 1..=raw_len {
            let value: LuaValue = table.get(i)?;
            items.push(convert(&value, ancestors)?);
        }
        return Ok(JsonValue::Array(items));
    }

    let mut map = serde_json::Map::with_capacity(pair_count);
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair?;
        let key = match key {
            LuaValue::String(s) => s.to_str()?.to_string(),
            LuaValue::Integer(i) => i.to_string(),
            LuaValue::Number(n) => n.to_string(),
            _ => {
                return Err(mlua::Error::RuntimeError(
                    "lewdware.storage table keys must be strings or numbers".to_string(),
                ));
            }
        };
        map.insert(key, convert(&value, ancestors)?);
    }
    Ok(JsonValue::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_storage() -> (Storage, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open_at(dir.path().join("test.db")).unwrap();
        (storage, dir)
    }

    fn eval(lua: &Lua, src: &str) -> LuaValue {
        lua.load(src).eval().unwrap()
    }

    #[tokio::test]
    async fn set_then_get_roundtrips_primitives() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (storage, _dir) = temp_storage();
                let lua = Lua::new();

                storage
                    .set("bool".to_string(), LuaValue::Boolean(true))
                    .unwrap();
                storage
                    .set("num".to_string(), LuaValue::Number(1.5))
                    .unwrap();
                storage
                    .set("str".to_string(), eval(&lua, "\"hello\""))
                    .unwrap();

                assert_eq!(storage.get(&lua, "bool").unwrap(), LuaValue::Boolean(true));
                assert_eq!(storage.get(&lua, "num").unwrap(), LuaValue::Number(1.5));
                assert!(matches!(
                    storage.get(&lua, "str").unwrap(),
                    LuaValue::String(_)
                ));
                assert_eq!(storage.get(&lua, "missing").unwrap(), LuaValue::Nil);
            })
            .await;
    }

    #[tokio::test]
    async fn set_then_get_roundtrips_arrays_and_maps() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (storage, _dir) = temp_storage();
                let lua = Lua::new();

                storage
                    .set("arr".to_string(), eval(&lua, "{1, 2, 3}"))
                    .unwrap();
                storage
                    .set("map".to_string(), eval(&lua, r#"{ a = 1, b = "two" }"#))
                    .unwrap();

                let arr = storage.get(&lua, "arr").unwrap();
                let table = match arr {
                    LuaValue::Table(t) => t,
                    _ => panic!("expected a table"),
                };
                assert_eq!(table.raw_len(), 3);
                assert_eq!(table.get::<i64>(1).unwrap(), 1);
                assert_eq!(table.get::<i64>(3).unwrap(), 3);

                let map = storage.get(&lua, "map").unwrap();
                let table = match map {
                    LuaValue::Table(t) => t,
                    _ => panic!("expected a table"),
                };
                assert_eq!(table.get::<i64>("a").unwrap(), 1);
                assert_eq!(table.get::<String>("b").unwrap(), "two");
            })
            .await;
    }

    #[test]
    fn rejects_functions() {
        let (storage, _dir) = temp_storage();
        let lua = Lua::new();

        let func = eval(&lua, "function() end");
        let err = storage.set("f".to_string(), func).unwrap_err();
        assert!(err.to_string().contains("booleans, numbers, strings"));
    }

    #[test]
    fn rejects_cyclic_tables() {
        let (storage, _dir) = temp_storage();
        let lua = Lua::new();

        let cyclic = eval(
            &lua,
            r#"
                local t = {}
                t.self = t
                return t
            "#,
        );
        let err = storage.set("cyclic".to_string(), cyclic).unwrap_err();
        assert!(err.to_string().contains("cyclic"));
    }

    #[test]
    fn rejects_a_single_value_over_the_limit() {
        let (storage, _dir) = temp_storage();
        let lua = Lua::new();

        let huge = eval(&lua, &format!("\"{}\"", "x".repeat(MAX_ITEM_BYTES + 1)));
        let err = storage.set("huge".to_string(), huge).unwrap_err();
        assert!(err.to_string().contains("too large") || err.to_string().contains("bytes"));
    }

    #[tokio::test]
    async fn remove_and_clear() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (storage, _dir) = temp_storage();
                let lua = Lua::new();

                storage
                    .set("a".to_string(), LuaValue::Boolean(true))
                    .unwrap();
                storage
                    .set("b".to_string(), LuaValue::Boolean(false))
                    .unwrap();

                assert!(storage.remove("a"));
                assert!(!storage.remove("a"));
                assert_eq!(storage.get(&lua, "a").unwrap(), LuaValue::Nil);
                assert_eq!(storage.keys(), vec!["b".to_string()]);

                storage.clear();
                assert!(storage.keys().is_empty());
            })
            .await;
    }

    #[tokio::test]
    async fn eviction_drops_oldest_keys_first_when_over_quota() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (storage, _dir) = temp_storage();
                let lua = Lua::new();

                let chunk = "x".repeat(MAX_TOTAL_BYTES / 3);
                for i in 0..5 {
                    let value = eval(&lua, &format!("\"{chunk}\""));
                    storage.set(format!("key{i}"), value).unwrap();
                }

                // Only ~2 chunks fit under the quota at once; the earliest keys should have been
                // evicted.
                assert_eq!(storage.get(&lua, "key0").unwrap(), LuaValue::Nil);
                assert!(!storage.keys().contains(&"key0".to_string()));
                assert!(storage.keys().contains(&"key4".to_string()));

                let inner = storage.inner.borrow();
                assert!(inner.total_bytes <= MAX_TOTAL_BYTES);
            })
            .await;
    }

    #[tokio::test]
    async fn flush_now_persists_to_disk_and_reopen_reads_it_back() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (storage, dir) = temp_storage();
                let lua = Lua::new();

                storage
                    .set("persisted".to_string(), eval(&lua, "42"))
                    .unwrap();
                storage.flush_now().unwrap();

                let reopened = Storage::open_at(dir.path().join("test.db")).unwrap();
                assert_eq!(
                    reopened.get(&lua, "persisted").unwrap(),
                    LuaValue::Integer(42)
                );
            })
            .await;
    }
}
