use anyhow::Result;
use rusqlite::{OptionalExtension, params};

pub fn migrate(db: &rusqlite::Connection) -> Result<()> {
    db.execute(
        "CREATE TABLE IF NOT EXISTS migrations (
            migration_index INTEGER NOT NULL
        )",
        [],
    )?;

    let value = db
        .query_row("SELECT migration_index FROM migrations", [], |row| {
            row.get("migration_index")
        })
        .optional()?;

    tracing::info!("{:?}", value);

    for i in value.unwrap_or(0)..MIGRATIONS.len() {
        db.execute_batch(MIGRATIONS[i])?;
    }

    tracing::info!("Executed migrations");

    if value.is_none() {
        db.execute(
            "INSERT INTO migrations (migration_index) VALUES (?)",
            params![MIGRATIONS.len()],
        )?;
    } else {
        db.execute(
            "UPDATE migrations SET migration_index = ?",
            params![MIGRATIONS.len()],
        )?;
    }

    Ok(())
}

const MIGRATIONS: [&str; 1] = [
    include_str!("migrations/0001_init_schema.sql")
];
