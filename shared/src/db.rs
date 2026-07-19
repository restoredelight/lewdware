use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};

pub fn migrate(db: &rusqlite::Connection) -> Result<()> {
    // Table rebuilds need foreign-key enforcement disabled, but the schema and migration ledger
    // still move atomically. Restoring the connection setting outside the transaction is
    // important because SQLite ignores PRAGMA foreign_keys changes while a transaction is open.
    let foreign_keys: bool = db.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    db.pragma_update(None, "foreign_keys", false)?;
    let result = (|| -> Result<()> {
        let tx = db.unchecked_transaction()?;
        tx.execute(
            "CREATE TABLE IF NOT EXISTS migrations (
            migration_index INTEGER NOT NULL
        )",
            [],
        )?;

        let value = tx
            .query_row("SELECT migration_index FROM migrations", [], |row| {
                row.get("migration_index")
            })
            .optional()?;

        let migration_index = value.unwrap_or(0);
        if migration_index > MIGRATIONS.len() {
            bail!(
                "pack database schema {migration_index} is newer than supported schema {}",
                MIGRATIONS.len()
            );
        }

        tracing::info!("Migrating from {} to {}", migration_index, MIGRATIONS.len());

        for migration in &MIGRATIONS[migration_index..] {
            tx.execute_batch(migration)?;
        }

        tracing::info!("Executed migrations");

        if value.is_none() {
            tx.execute(
                "INSERT INTO migrations (migration_index) VALUES (?)",
                params![MIGRATIONS.len()],
            )?;
        } else {
            tx.execute(
                "UPDATE migrations SET migration_index = ?",
                params![MIGRATIONS.len()],
            )?;
        }

        let violation = tx
            .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
            .optional()?;
        if violation.is_some() {
            bail!("pack database migration produced a foreign-key violation");
        }
        tx.commit()?;
        Ok(())
    })();

    let restore_result = db.pragma_update(None, "foreign_keys", foreign_keys);
    result?;
    restore_result?;
    Ok(())
}

const MIGRATIONS: [&str; 3] = [
    include_str!("migrations/0001_init_schema.sql"),
    include_str!("migrations/0002_add_artists.sql"),
    include_str!("migrations/0003_archive_only_media.sql"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn version_two_database() -> rusqlite::Connection {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(MIGRATIONS[0]).unwrap();
        db.execute_batch(MIGRATIONS[1]).unwrap();
        db.execute(
            "CREATE TABLE migrations (migration_index INTEGER NOT NULL)",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO migrations VALUES (2)", []).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        db
    }

    #[test]
    fn archive_only_media_migration_removes_path_and_requires_a_range() {
        let db = version_two_database();
        db.execute(
            "INSERT INTO media
             (file_name, file_type, offset, length, path, hash)
             VALUES ('file.mp3', 'audio', 12, 34, '/tmp/old', x'01')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO tags(name) VALUES ('tag')", [])
            .unwrap();
        db.execute("INSERT INTO media_tags VALUES (1, 1)", [])
            .unwrap();

        migrate(&db).unwrap();

        let mut columns = db.prepare("PRAGMA table_info(media)").unwrap();
        let columns = columns
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, bool>(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(!columns.iter().any(|(name, _)| name == "path"));
        assert!(
            columns
                .iter()
                .any(|(name, not_null)| name == "offset" && *not_null)
        );
        assert!(
            columns
                .iter()
                .any(|(name, not_null)| name == "length" && *not_null)
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM media_tags", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
        assert!(
            db.pragma_query_value(None, "foreign_keys", |row| row.get::<_, bool>(0))
                .unwrap()
        );
    }

    #[test]
    fn a_failed_pack_migration_rolls_back_the_schema_and_ledger() {
        let db = version_two_database();
        db.execute(
            "INSERT INTO media (file_name, file_type, hash)
             VALUES ('loose.mp3', 'audio', x'01')",
            [],
        )
        .unwrap();

        assert!(migrate(&db).is_err());
        assert_eq!(
            db.query_row("SELECT migration_index FROM migrations", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            2
        );
        let path_exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('media') WHERE name = 'path')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(path_exists);
    }
}
