use std::{collections::HashMap, path::Path};

use anyhow::{Result, anyhow};
use rusqlite::{params, Connection, OptionalExtension};
use shared::{db::{migrate, Database}, read_config::{Config, Resolved}};

use crate::PackedEntry;

pub fn build_sqlite_index(
    db_path: &Path,
    entries: &[PackedEntry],
    config: &Config,
    resolved: Resolved,
) -> Result<()> {
    let mut conn = Connection::open(db_path)?;

    {
        let db = DatabaseConnection(&mut conn);
        migrate(db)?;
    }

    let tx = conn.transaction()?;
    let mut tag_cache: HashMap<String, i64> = HashMap::new();
    {
        let mut tag_stmt = tx.prepare("INSERT INTO tags (name) VALUES (?1) RETURNING id")?;

        for tag in config.root_config.tags.keys() {
            let id = tag_stmt.query_row(params![tag], |row| row.get("id"))?;
            tag_cache.insert(tag.clone(), id);
        }

        let mut media_stmt = tx.prepare("INSERT INTO media (file_name, file_type, category, offset, length, width, height, duration, audio) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) RETURNING id")?;
        let mut media_tag_stmt =
            tx.prepare("INSERT INTO media_tags (media_id, tag_id) VALUES (?1, ?2)")?;

        for entry in entries {
            let parts = entry.file_info.to_parts();

            let media_id: i64 = media_stmt.query_row(
                params![
                    entry.file_name,
                    parts.file_type.as_str(),
                    entry.category.as_str(),
                    entry.offset as i64,
                    entry.length as i64,
                    parts.width,
                    parts.height,
                    parts.duration,
                    parts.audio
                ],
                |row| row.get("id"),
            )?;

            for tag in &entry.tags {
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

struct DatabaseConnection<'a>(&'a mut Connection);

impl<'a> Database for DatabaseConnection<'a> {
    fn exec(&self, sql: &str) -> Result<()> {
        self.0.execute_batch(sql)?;
        Ok(())
    }

    fn get_value(&self, sql: &str, field: &str) -> Result<Option<usize>> {
        self.0
            .query_row(sql, params![], |row| row.get(field))
            .optional()
            .map_err(|err| err.into())
    }
}
