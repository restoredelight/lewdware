use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};

pub fn migrate(db: &rusqlite::Connection) -> Result<()> {
    // A migration that rebuilds a table needs foreign-key enforcement disabled, but the schema and
    // migration ledger still move atomically. Restoring the connection setting outside the
    // transaction is important because SQLite ignores PRAGMA foreign_keys changes while a
    // transaction is open.
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

/// One migration, because nothing is released yet: a database is either current or absent, so
/// there is no upgrade path worth carrying. The ledger stays a list so that the first real
/// migration is an append rather than a rewrite of `migrate`.
///
/// Two files concatenated: the base schema below, and the behaviour tables that the pack editor's
/// ledger includes from the same file (see `migrations/behaviour_schema.sql`).
const MIGRATIONS: [&str; 1] = [concat!(
    include_str!("migrations/0001_init_schema.sql"),
    include_str!("migrations/behaviour_schema.sql"),
)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_creates_the_expected_schema() {
        let db = rusqlite::Connection::open_in_memory().unwrap();

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
        // Against the ledger's own length rather than a literal, so adding a migration doesn't
        // need this test edited to keep passing.
        assert_eq!(
            db.query_row("SELECT migration_index FROM migrations", [], |row| row
                .get::<_, usize>(0))
                .unwrap(),
            MIGRATIONS.len()
        );
        assert!(
            db.pragma_query_value(None, "foreign_keys", |row| row.get::<_, bool>(0))
                .unwrap()
        );
    }

    #[test]
    fn migrating_an_already_current_database_leaves_its_rows_alone() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();

        db.execute(
            "INSERT INTO media (file_name, file_type, offset, length, hash)
             VALUES ('file.mp3', 'audio', 12, 34, x'01')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO tags(name) VALUES ('tag')", [])
            .unwrap();
        db.execute("INSERT INTO media_tags VALUES (1, 1)", [])
            .unwrap();

        migrate(&db).unwrap();

        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM media_tags", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
    }
}
