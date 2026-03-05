use anyhow::Result;
use rusqlite::{OptionalExtension, params};

pub fn migrate(db: &rusqlite::Connection) -> Result<()> {
    db.execute(
        "CREATE TABLE IF NOT EXISTS migrations (
            migration_index INTEGER NOT NULL
        )",
        [],
    )?;

    let value = db.query_row("SELECT migration_index FROM migrations", [], |row| {
        row.get("migration_index")
    }).optional()?;

    println!("{:?}", value);

    for i in value.unwrap_or(0)..MIGRATIONS.len() {
        db.execute_batch(MIGRATIONS[i])?;
    }

    println!("Executed migrations");

    if value.is_none() {
        db.execute(
            "INSERT INTO migrations (migration_index) VALUES (?)",
            params![MIGRATIONS.len()]
        )?;
    } else {
        db.execute(
            "UPDATE migrations SET migration_index = ?",
            params![MIGRATIONS.len()]
        )?;
    }

    Ok(())
}

const MIGRATIONS: [&str; 1] = [r#"
    CREATE TABLE IF NOT EXISTS media (
        id INTEGER PRIMARY KEY,
        file_name TEXT NOT NULL,
        file_type TEXT CHECK(file_type IN ('image','video','audio')) NOT NULL,
        offset INTEGER,
        length INTEGER,
        path TEXT,
        width INTEGER,
        height INTEGER,
        transparent INTEGER,
        duration REAL,
        audio INTEGER,
        hash BLOB NOT NULL,
        thumbnail BLOB
    );
    CREATE INDEX media_hash_index ON media (hash);
    CREATE TABLE IF NOT EXISTS tags (
        id INTEGER PRIMARY KEY,
        name TEXT UNIQUE NOT NULL
    );
    CREATE TABLE IF NOT EXISTS media_tags (
        media_id INTEGER NOT NULL,
        tag_id INTEGER NOT NULL,
        PRIMARY KEY(media_id, tag_id),
        FOREIGN KEY(media_id) REFERENCES media(id) ON DELETE CASCADE,
        FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
    );
    CREATE TABLE IF NOT EXISTS modes (
        id INTEGER PRIMARY KEY,
        file BLOB NOT NULL
    );
    "#];
