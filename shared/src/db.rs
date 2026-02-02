use anyhow::Result;

pub fn migrate(db: impl Database) -> Result<()> {
    db.exec(
        "CREATE TABLE IF NOT EXISTS migrations (
            migration_index INTEGER NOT NULL
        )",
    )?;

    let value = db.get_value("SELECT migration_index FROM migrations", "migration_index")?;

    for i in value.unwrap_or(0)..MIGRATIONS.len() {
        db.exec(MIGRATIONS[i])?;
    }

    if value.is_none() {
        db.exec(&format!(
            "INSERT INTO migrations (migration_index) VALUES ({})",
            MIGRATIONS.len()
        ))?;
    } else {
        db.exec(&format!(
            "UPDATE migrations SET migration_index = {}",
            MIGRATIONS.len()
        ))?;
    }

    Ok(())
}

/// A basic database trait, because we work with single connections and connection pools (and I
/// don't want to work with features).
pub trait Database {
    fn exec(&self, sql: &str) -> Result<()>;

    fn get_value(&self, sql: &str, field: &str) -> Result<Option<usize>>;
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
    CREATE TABLE IF NOT EXISTS wallpapers (
        id INTEGER PRIMARY KEY,
        file_name TEXT NOT NULL,
        offset INTEGER NOT NULL,
        length INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS notifications (
        id INTEGER PRIMARY KEY,
        summary TEXT,
        body TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS links (
        id INTEGER PRIMARY KEY,
        link TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS prompts (
        id INTEGER PRIMARY KEY,
        prompt TEXT NOT NULL
    );
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
    CREATE TABLE IF NOT EXISTS wallpaper_tags (
        wallpaper_id INTEGER NOT NULL,
        tag_id INTEGER NOT NULL,
        PRIMARY KEY(wallpaper_id, tag_id),
        FOREIGN KEY(wallpaper_id) REFERENCES wallpapers(id) ON DELETE CASCADE,
        FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
    );
    CREATE TABLE IF NOT EXISTS notification_tags (
        notification_id INTEGER NOT NULL,
        tag_id INTEGER NOT NULL,
        PRIMARY KEY(notification_id, tag_id),
        FOREIGN KEY(notification_id) REFERENCES notifications(id) ON DELETE CASCADE,
        FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
    );
    CREATE TABLE IF NOT EXISTS link_tags (
        link_id INTEGER NOT NULL,
        tag_id INTEGER NOT NULL,
        PRIMARY KEY(link_id, tag_id),
        FOREIGN KEY(link_id) REFERENCES links(id) ON DELETE CASCADE,
        FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
    );
    CREATE TABLE IF NOT EXISTS prompt_tags (
        prompt_id INTEGER NOT NULL,
        tag_id INTEGER NOT NULL,
        PRIMARY KEY(prompt_id, tag_id),
        FOREIGN KEY(prompt_id) REFERENCES prompts(id) ON DELETE CASCADE,
        FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
    );
    "#];
