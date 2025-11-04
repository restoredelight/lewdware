use std::{collections::HashMap, path::Path};

use anyhow::{Result, anyhow};
use shared::read_config::{Config, Resolved};
use rusqlite::{Connection, params};

use crate::PackedEntry;

pub fn build_sqlite_index(
    db_path: &Path,
    entries: &[PackedEntry],
    config: &Config,
    resolved: Resolved,
) -> Result<()> {
    let mut conn = Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = OFF;
        PRAGMA synchronous = OFF;
        PRAGMA temp_store = MEMORY;
        PRAGMA page_size = 4096;
        CREATE TABLE media (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL,
            media_type TEXT CHECK(media_type IN ('image','video','audio','other')) NOT NULL,
            category TEXT CHECK(category IN ('default', 'wallpaper')) NOT NULL,
            offset INTEGER NOT NULL,
            length INTEGER NOT NULL,
            width INTEGER,
            height INTEGER,
            duration REAL
        );
        CREATE TABLE wallpapers (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL,
            offset INTEGER NOT NULL,
            length INTEGER NOT NULL
        );
        CREATE TABLE notifications (
            id INTEGER PRIMARY KEY,
            summary TEXT,
            body TEXT NOT NULL
        );
        CREATE TABLE links (
            id INTEGER PRIMARY KEY,
            link TEXT NOT NULL
        );
        CREATE TABLE prompts (
            id INTEGER PRIMARY KEY,
            prompt TEXT NOT NULL
        );
        CREATE TABLE tags (
            id INTEGER PRIMARY KEY,
            name TEXT UNIQUE NOT NULL
        );
        CREATE TABLE media_tags (
            media_id INTEGER NOT NULL,
            tag_id INTEGER NOT NULL,
            PRIMARY KEY(media_id, tag_id),
            FOREIGN KEY(media_id) REFERENCES media(id) ON DELETE CASCADE,
            FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
        );
        CREATE TABLE wallpaper_tags (
            wallpaper_id INTEGER NOT NULL,
            tag_id INTEGER NOT NULL,
            PRIMARY KEY(wallpaper_id, tag_id),
            FOREIGN KEY(wallpaper_id) REFERENCES wallpapers(id) ON DELETE CASCADE,
            FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
        );
        CREATE TABLE notification_tags (
            notification_id INTEGER NOT NULL,
            tag_id INTEGER NOT NULL,
            PRIMARY KEY(notification_id, tag_id),
            FOREIGN KEY(notification_id) REFERENCES notifications(id) ON DELETE CASCADE,
            FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
        );
        CREATE TABLE link_tags (
            link_id INTEGER NOT NULL,
            tag_id INTEGER NOT NULL,
            PRIMARY KEY(link_id, tag_id),
            FOREIGN KEY(link_id) REFERENCES links(id) ON DELETE CASCADE,
            FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
        );
        CREATE TABLE prompt_tags (
            prompt_id INTEGER NOT NULL,
            tag_id INTEGER NOT NULL,
            PRIMARY KEY(prompt_id, tag_id),
            FOREIGN KEY(prompt_id) REFERENCES prompts(id) ON DELETE CASCADE,
            FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
        );
        "#,
    )?;

    let tx = conn.transaction()?;
    let mut tag_cache: HashMap<String, i64> = HashMap::new();
    {
        let mut tag_stmt = tx.prepare("INSERT INTO tags (name) VALUES (?1) RETURNING id")?;

        for tag in config.root_config.tags.keys() {
            let id = tag_stmt.query_row(params![tag], |row| row.get("id"))?;
            tag_cache.insert(tag.clone(), id);
        }

        let mut media_stmt = tx.prepare("INSERT INTO media (path, media_type, category, offset, length, width, height, duration) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) RETURNING id")?;
        let mut media_tag_stmt =
            tx.prepare("INSERT INTO media_tags (media_id, tag_id) VALUES (?1, ?2)")?;

        for e in entries {
            let media_id: i64 = media_stmt.query_row(
                params![
                    e.rel_path,
                    e.media_type.as_str(),
                    e.category.as_str(),
                    e.offset as i64,
                    e.length as i64,
                    e.width,
                    e.height,
                    e.duration
                ],
                |row| row.get("id"),
            )?;

            for tag in &e.tags {
                let tag_id = tag_cache
                    .get(tag)
                    .ok_or_else(|| anyhow!("Tag {} not found", tag))?;

                media_tag_stmt.execute(params![media_id, tag_id])?;
            }
        }

        let mut notification_stmt =
            tx.prepare("INSERT INTO notifications (summary, body) VALUES (?1, ?2) RETURNING id")?;
        let mut notification_tag_stmt =
            tx.prepare("INSERT INTO notification_tags (notification_id, tag_id) VALUES (?1, ?2)")?;

        for notification in resolved.notifications {
            let notification_id: i64 = notification_stmt.query_row(
                params![notification.opts.summary, notification.primary],
                |row| row.get("id"),
            )?;

            for tag in notification.tags {
                let tag_id = tag_cache
                    .get(&tag)
                    .ok_or_else(|| anyhow!("Tag {} not found", tag))?;

                notification_tag_stmt.execute(params![notification_id, tag_id])?;
            }
        }

        let mut link_stmt = tx.prepare("INSERT INTO links (link) VALUES (?1) RETURNING id")?;
        let mut link_tag_stmt =
            tx.prepare("INSERT INTO link_tags (link_id, tag_id) VALUES (?1, ?2)")?;

        for link in resolved.links {
            let link_id: i64 = link_stmt.query_row(params![link.primary], |row| row.get("id"))?;

            for tag in link.tags {
                let tag_id = tag_cache
                    .get(&tag)
                    .ok_or_else(|| anyhow!("Tag {} not found", tag))?;

                link_tag_stmt.execute(params![link_id, tag_id])?;
            }
        }

        let mut prompt_stmt =
            tx.prepare("INSERT INTO prompts (prompt) VALUES (?1) RETURNING id")?;
        let mut prompt_tag_stmt =
            tx.prepare("INSERT INTO prompt_tags (prompt_id, tag_id) VALUES (?1, ?2)")?;

        for prompt in resolved.prompts {
            let prompt_id: i64 =
                prompt_stmt.query_row(params![prompt.primary], |row| row.get("id"))?;

            for tag in prompt.tags {
                let tag_id = tag_cache
                    .get(&tag)
                    .ok_or_else(|| anyhow!("Tag {} not found", tag))?;

                prompt_tag_stmt.execute(params![prompt_id, tag_id])?;
            }
        }
    }
    tx.commit()?;

    Ok(())
}
