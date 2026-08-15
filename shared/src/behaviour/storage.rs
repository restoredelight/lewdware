//! Reading and writing a [`Behaviour`] against the pack database.
//!
//! The document is split across two places. Its structural half — the media slots and the whole
//! timeline — lives in the `behaviour_*` tables (see `migrations/0002_behaviour_timeline.sql`),
//! so that a slot is a foreign key the database enforces and so that editing one stage produces a
//! changeset the size of that stage. What is left — the content pools and their settings — is
//! still JSON in `pack_data`, and moves next.
//!
//! [`read`] assembles the two into the one `Behaviour` every other layer already understands, and
//! [`write`] takes it apart again. Nothing above this module knows about the split: the engine
//! still gets a `Behaviour`, the editor still patches a `Behaviour`, and the converter still
//! produces one.
//!
//! **Writes diff rather than replace.** Deleting every row and reinserting would be far simpler,
//! and would defeat the point: the editor's undo is a SQLite session changeset, so a wholesale
//! rewrite would record every stage in the pack as changed on every keystroke — exactly the cost
//! moving out of the blob was meant to remove. So each table is reconciled: rows that are gone are
//! deleted, rows that are new are inserted, and rows that already match are left alone.

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Serialize, de::DeserializeOwned};

use super::schema::{
    Behaviour, Content, ContentGroup, ContentSelection, EventCountCondition, EventSchedule, Events,
    Experience, Interval, Mitosis, Movement, PromptSettings, Stage, StageEnd, TextItem, Timeline,
    Transition, WebLink,
};

/// The `pack_data` key the document used to live under.
///
/// Nothing writes it any more. Its presence is how [`read`] recognises a pack from before the
/// content pools moved into tables, which it refuses rather than silently reading as empty.
const LEGACY_BLOB_KEY: &str = "behaviour";

/// The pack's behaviour document, or a default one if the pack has never had it written.
pub fn read(conn: &Connection) -> Result<Behaviour> {
    reject_legacy_blob(conn)?;
    let (wallpaper, splash) = conn
        .query_row(
            "SELECT wallpaper, splash FROM behaviour_content WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, Option<u64>>(0)?, row.get::<_, Option<u64>>(1)?)),
        )
        .optional()?
        .unwrap_or((None, None));
    let submit_label: Option<String> = conn
        .query_row(
            "SELECT prompt_submit_label FROM behaviour_settings WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .flatten();

    Ok(Behaviour {
        content: Content {
            content_groups: read_content_groups(conn)?,
            captions: read_text_pool(conn, "caption")?,
            prompts: read_text_pool(conn, "prompt")?,
            prompt_settings: PromptSettings { submit_label },
            notifications: read_text_pool(conn, "notification")?,
            subliminals: read_text_pool(conn, "subliminal")?,
            web_links: read_web_links(conn)?,
            wallpaper,
            splash,
        },
        experience: read_experience(conn)?,
    })
}

/// Replaces the stored document with `behaviour`, touching only the rows that actually differ.
pub fn write(tx: &Transaction<'_>, behaviour: &Behaviour) -> Result<()> {
    let content = &behaviour.content;
    tx.execute(
        "INSERT INTO behaviour_content (singleton, wallpaper, splash) VALUES (1, ?, ?)
         ON CONFLICT(singleton) DO UPDATE SET wallpaper = excluded.wallpaper,
                                              splash = excluded.splash",
        params![content.wallpaper, content.splash],
    )?;
    tx.execute(
        "INSERT INTO behaviour_settings (singleton, prompt_submit_label) VALUES (1, ?)
         ON CONFLICT(singleton) DO UPDATE SET prompt_submit_label = excluded.prompt_submit_label",
        params![content.prompt_settings.submit_label],
    )?;
    write_content_groups(tx, &content.content_groups)?;
    write_text_pool(tx, "caption", &content.captions)?;
    write_text_pool(tx, "prompt", &content.prompts)?;
    write_text_pool(tx, "notification", &content.notifications)?;
    write_text_pool(tx, "subliminal", &content.subliminals)?;
    write_web_links(tx, &content.web_links)?;
    write_experience(tx, behaviour.experience.as_ref())
}

fn reject_legacy_blob(conn: &Connection) -> Result<()> {
    let present: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pack_data WHERE name = ?)",
        params![LEGACY_BLOB_KEY],
        |row| row.get(0),
    )?;
    if present {
        bail!(
            "this pack's behaviour document predates the behaviour tables; re-import the pack in \
             the pack editor"
        );
    }
    Ok(())
}

// ── The content pools ────────────────────────────────────────────────────────

fn read_text_pool(conn: &Connection, kind: &str) -> Result<Vec<TextItem>> {
    let mut statement =
        conn.prepare("SELECT id, text FROM behaviour_text_item WHERE kind = ? ORDER BY position")?;
    let rows: Vec<(i64, String)> = statement
        .query_map(params![kind], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, text)| {
            Ok(TextItem {
                text,
                tags: read_tags(conn, "behaviour_text_item_tag", "item_id", &id)?,
            })
        })
        .collect()
}

fn write_text_pool(tx: &Transaction<'_>, kind: &str, items: &[TextItem]) -> Result<()> {
    // Pool entries have no author-facing identity -- nothing outside the document refers to a
    // caption -- so they are keyed by position and rewritten in place, rather than matched up.
    tx.execute(
        "DELETE FROM behaviour_text_item WHERE kind = ?1 AND position >= ?2",
        params![kind, items.len() as i64],
    )?;
    for (position, item) in items.iter().enumerate() {
        let id: i64 = tx.query_row(
            "INSERT INTO behaviour_text_item (kind, position, text) VALUES (?1, ?2, ?3)
             ON CONFLICT(kind, position) DO UPDATE SET text = excluded.text
             RETURNING id",
            params![kind, position as i64, item.text],
            |row| row.get(0),
        )?;
        write_tags(tx, "behaviour_text_item_tag", "item_id", &id, &item.tags)?;
    }
    Ok(())
}

fn read_web_links(conn: &Connection) -> Result<Vec<WebLink>> {
    let mut statement = conn.prepare("SELECT id, url FROM behaviour_web_link ORDER BY position")?;
    let rows: Vec<(i64, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;

    let mut args = conn
        .prepare("SELECT value FROM behaviour_web_link_arg WHERE link_id = ? ORDER BY position")?;
    rows.into_iter()
        .map(|(id, url)| {
            Ok(WebLink {
                url,
                args: args
                    .query_map(params![id], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?,
                tags: read_tags(conn, "behaviour_web_link_tag", "link_id", &id)?,
            })
        })
        .collect()
}

fn write_web_links(tx: &Transaction<'_>, links: &[WebLink]) -> Result<()> {
    tx.execute(
        "DELETE FROM behaviour_web_link WHERE position >= ?",
        params![links.len() as i64],
    )?;
    for (position, link) in links.iter().enumerate() {
        let id: i64 = tx.query_row(
            "INSERT INTO behaviour_web_link (position, url) VALUES (?1, ?2)
             ON CONFLICT(position) DO UPDATE SET url = excluded.url
             RETURNING id",
            params![position as i64, link.url],
            |row| row.get(0),
        )?;
        tx.execute(
            "DELETE FROM behaviour_web_link_arg WHERE link_id = ?1 AND position >= ?2",
            params![id, link.args.len() as i64],
        )?;
        for (position, value) in link.args.iter().enumerate() {
            tx.execute(
                "INSERT INTO behaviour_web_link_arg (link_id, position, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(link_id, position) DO UPDATE SET value = excluded.value",
                params![id, position as i64, value],
            )?;
        }
        write_tags(tx, "behaviour_web_link_tag", "link_id", &id, &link.tags)?;
    }
    Ok(())
}

fn read_content_groups(conn: &Connection) -> Result<Vec<ContentGroup>> {
    let mut statement = conn.prepare(
        "SELECT id, label, description, enabled_by_default
         FROM behaviour_content_group ORDER BY position",
    )?;
    let rows: Vec<(String, String, Option<String>, bool)> = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, label, description, enabled_by_default)| {
            Ok(ContentGroup {
                tags: read_tags(conn, "behaviour_content_group_tag", "group_id", &id)?,
                id,
                label,
                description,
                enabled_by_default,
            })
        })
        .collect()
}

fn write_content_groups(tx: &Transaction<'_>, groups: &[ContentGroup]) -> Result<()> {
    retain(
        tx,
        "behaviour_content_group",
        "id",
        groups.iter().map(|group| group.id.as_str()),
    )?;
    for (position, group) in groups.iter().enumerate() {
        tx.execute(
            "INSERT INTO behaviour_content_group
                 (id, position, label, description, enabled_by_default)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET position = excluded.position,
                                           label = excluded.label,
                                           description = excluded.description,
                                           enabled_by_default = excluded.enabled_by_default",
            params![
                group.id,
                position as i64,
                group.label,
                group.description,
                group.enabled_by_default,
            ],
        )?;
        write_tags(
            tx,
            "behaviour_content_group_tag",
            "group_id",
            &group.id,
            &group.tags,
        )?;
    }
    Ok(())
}

// ── Tags ─────────────────────────────────────────────────────────────────────
//
// Every tag list in the document is a join to `tags`. That is what makes renaming a tag
// `UPDATE tags SET name`, merging one a re-point, and deleting one a cascade, instead of a pass
// over the whole document rewriting strings (the late `Behaviour::rewrite_tag`).

fn read_tags(
    conn: &Connection,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
) -> Result<Vec<String>> {
    let mut statement = conn.prepare(&format!(
        "SELECT tags.name FROM {table}
         JOIN tags ON tags.id = {table}.tag_id
         WHERE {table}.{key} = ? ORDER BY {table}.position"
    ))?;
    let tags = statement
        .query_map([owner], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tags)
}

fn write_tags(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    owner: &dyn rusqlite::ToSql,
    tags: &[String],
) -> Result<()> {
    let ids = tags
        .iter()
        .map(|tag| tag_id(tx, tag))
        .collect::<Result<Vec<_>>>()?;
    tx.execute(
        &format!(
            "DELETE FROM {table} WHERE {key} = ?1
             AND tag_id NOT IN (SELECT value FROM json_each(?2))"
        ),
        params![owner, serde_json::to_string(&ids)?],
    )?;
    for (position, id) in ids.iter().enumerate() {
        tx.execute(
            &format!(
                "INSERT INTO {table} ({key}, tag_id, position) VALUES (?1, ?2, ?3)
                 ON CONFLICT({key}, tag_id) DO UPDATE SET position = excluded.position"
            ),
            params![owner, id, position as i64],
        )?;
    }
    Ok(())
}

/// The id of `name`, creating the tag if the pack does not have it yet.
///
/// A tag the document names *is* a tag the pack has -- that is what the foreign key asserts. So a
/// tag typed into a caption or a content group becomes a real row, and shows up in the Tags tab
/// with no media attached, rather than existing only as a string inside the document.
fn tag_id(tx: &Transaction<'_>, name: &str) -> Result<i64> {
    tx.execute(
        "INSERT OR IGNORE INTO tags (name) VALUES (?)",
        params![name],
    )?;
    Ok(
        tx.query_row("SELECT id FROM tags WHERE name = ?", params![name], |row| {
            row.get(0)
        })?,
    )
}

// ── The experience section ───────────────────────────────────────────────────

fn read_experience(conn: &Connection) -> Result<Option<Experience>> {
    let label: Option<Option<String>> = conn
        .query_row(
            "SELECT label FROM behaviour_experience WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    // No row at all is a pack with no timeline; a row with a NULL label is a timeline that simply
    // doesn't override the mode's name.
    let Some(label) = label else {
        return Ok(None);
    };
    Ok(Some(Experience {
        timeline: Timeline {
            stages: read_stages(conn)?,
            transitions: read_transitions(conn)?,
        },
        label,
    }))
}

fn read_stages(conn: &Connection) -> Result<Vec<Stage>> {
    let mut statement = conn.prepare(
        "SELECT id, label, restricts_content, wallpaper FROM behaviour_stage ORDER BY position",
    )?;
    let rows: Vec<(String, String, bool, Option<u64>)> = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;

    rows.into_iter()
        .map(|(id, label, restricts_content, wallpaper)| {
            Ok(Stage {
                content: ContentSelection {
                    tags: restricts_content
                        .then(|| read_tags(conn, "behaviour_stage_tag", "stage_id", &id))
                        .transpose()?,
                    wallpaper,
                },
                end: read_stage_end(conn, &id)?,
                events: read_stage_events(conn, &id)?,
                movement: read_stage_movement(conn, &id)?,
                mitosis: read_stage_mitosis(conn, &id)?,
                id,
                label,
            })
        })
        .collect()
}

fn read_stage_end(conn: &Connection, stage: &str) -> Result<Option<StageEnd>> {
    conn.query_row(
        "SELECT duration_seconds, strategy, event_kind, event_count, event_scope
         FROM behaviour_stage_end WHERE stage_id = ?",
        params![stage],
        |row| {
            Ok((
                row.get::<_, Option<f64>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<u32>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        },
    )
    .optional()?
    .map(|(duration_seconds, strategy, kind, count, scope)| {
        let event_count = match (kind, count, scope) {
            (Some(kind), Some(count), Some(scope)) => Some(EventCountCondition {
                event: from_text(&kind)?,
                count,
                scope: from_text(&scope)?,
            }),
            _ => None,
        };
        Ok(StageEnd {
            duration_seconds,
            event_count,
            strategy: from_text(&strategy)?,
        })
    })
    .transpose()
}

fn read_stage_movement(conn: &Connection, stage: &str) -> Result<Option<Movement>> {
    Ok(conn
        .query_row(
            "SELECT minimum_speed, maximum_speed FROM behaviour_stage_movement WHERE stage_id = ?",
            params![stage],
            |row| {
                Ok(Movement {
                    minimum_speed: row.get(0)?,
                    maximum_speed: row.get(1)?,
                })
            },
        )
        .optional()?)
}

fn read_stage_mitosis(conn: &Connection, stage: &str) -> Result<Option<Mitosis>> {
    Ok(conn
        .query_row(
            "SELECT chance, count FROM behaviour_stage_mitosis WHERE stage_id = ?",
            params![stage],
            |row| {
                Ok(Mitosis {
                    chance: row.get(0)?,
                    count: row.get(1)?,
                })
            },
        )
        .optional()?)
}

/// One `behaviour_stage_event` row. Which interval columns carry a value depends on
/// `interval_kind`, mirroring the `Interval` enum's own tag.
struct EventRow {
    kind: String,
    interval_kind: String,
    seconds: Option<f64>,
    minimum_seconds: Option<f64>,
    maximum_seconds: Option<f64>,
    initial_delay_seconds: Option<f64>,
    max_concurrent: Option<u32>,
}

fn read_stage_events(conn: &Connection, stage: &str) -> Result<Events> {
    let mut statement = conn.prepare(
        "SELECT kind, interval_kind, seconds, minimum_seconds, maximum_seconds,
                initial_delay_seconds, max_concurrent
         FROM behaviour_stage_event WHERE stage_id = ?",
    )?;
    let rows: Vec<EventRow> = statement
        .query_map(params![stage], |row| {
            Ok(EventRow {
                kind: row.get(0)?,
                interval_kind: row.get(1)?,
                seconds: row.get(2)?,
                minimum_seconds: row.get(3)?,
                maximum_seconds: row.get(4)?,
                initial_delay_seconds: row.get(5)?,
                max_concurrent: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let mut events = Events::default();
    for row in rows {
        let interval = match row.interval_kind.as_str() {
            "fixed" => Interval::Fixed {
                seconds: row.seconds.unwrap_or_default(),
            },
            "random" => Interval::Random {
                minimum_seconds: row.minimum_seconds.unwrap_or_default(),
                maximum_seconds: row.maximum_seconds.unwrap_or_default(),
            },
            other => bail!("unknown event interval kind {other:?}"),
        };
        let schedule = Some(EventSchedule {
            interval,
            initial_delay_seconds: row.initial_delay_seconds,
            max_concurrent: row.max_concurrent,
        });
        match row.kind.as_str() {
            "popup" => events.popup = schedule,
            "web" => events.web = schedule,
            "notification" => events.notification = schedule,
            "prompt" => events.prompt = schedule,
            "subliminal" => events.subliminal = schedule,
            other => bail!("unknown event kind {other:?}"),
        }
    }
    Ok(events)
}

fn read_transitions(conn: &Connection) -> Result<Vec<Transition>> {
    let mut statement = conn.prepare(
        "SELECT id, from_stage, to_stage, duration_seconds, easing
         FROM behaviour_transition ORDER BY position",
    )?;
    let rows: Vec<(String, String, String, f64, String)> = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;

    let mut categories = conn.prepare(
        "SELECT category FROM behaviour_transition_category
         WHERE transition_id = ? ORDER BY position",
    )?;
    rows.into_iter()
        .map(|(id, from_stage, to_stage, duration_seconds, easing)| {
            let affected: Vec<String> = categories
                .query_map(params![&id], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(Transition {
                id,
                from_stage,
                to_stage,
                duration_seconds,
                easing: from_text(&easing)?,
                affected: affected
                    .iter()
                    .map(|value| from_text(value))
                    .collect::<Result<_>>()?,
            })
        })
        .collect()
}

fn write_experience(tx: &Transaction<'_>, experience: Option<&Experience>) -> Result<()> {
    let Some(experience) = experience else {
        // Suspending the timeline empties the section; the stage rows go with it, and the media
        // they pointed at is deliberately *not* retired (see `MediaPack::edit_behaviour`).
        tx.execute("DELETE FROM behaviour_experience", [])?;
        tx.execute("DELETE FROM behaviour_stage", [])?;
        tx.execute("DELETE FROM behaviour_transition", [])?;
        return Ok(());
    };
    tx.execute(
        "INSERT INTO behaviour_experience (singleton, label) VALUES (1, ?)
         ON CONFLICT(singleton) DO UPDATE SET label = excluded.label",
        params![experience.label],
    )?;
    // Stages first: a transition's endpoints are foreign keys, so a transition into a stage this
    // write is adding can only be inserted once that stage exists. Removing a stage cascades to
    // the transitions touching it, which is why they are reconciled afterwards rather than before.
    write_stages(tx, &experience.timeline.stages)?;
    write_transitions(tx, &experience.timeline.transitions)?;
    Ok(())
}

fn write_stages(tx: &Transaction<'_>, stages: &[Stage]) -> Result<()> {
    retain(
        tx,
        "behaviour_stage",
        "id",
        stages.iter().map(|s| s.id.as_str()),
    )?;
    for (position, stage) in stages.iter().enumerate() {
        tx.execute(
            "INSERT INTO behaviour_stage (id, position, label, restricts_content, wallpaper)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET position = excluded.position,
                                           label = excluded.label,
                                           restricts_content = excluded.restricts_content,
                                           wallpaper = excluded.wallpaper",
            params![
                stage.id,
                position as i64,
                stage.label,
                stage.content.tags.is_some(),
                stage.content.wallpaper,
            ],
        )?;
        write_tags(
            tx,
            "behaviour_stage_tag",
            "stage_id",
            &stage.id,
            stage.content.tags.as_deref().unwrap_or(&[]),
        )?;
        write_stage_end(tx, &stage.id, stage.end.as_ref())?;
        write_stage_movement(tx, &stage.id, stage.movement.as_ref())?;
        write_stage_mitosis(tx, &stage.id, stage.mitosis.as_ref())?;
        write_stage_events(tx, &stage.id, &stage.events)?;
    }
    Ok(())
}

fn write_stage_end(tx: &Transaction<'_>, stage: &str, end: Option<&StageEnd>) -> Result<()> {
    let Some(end) = end else {
        tx.execute(
            "DELETE FROM behaviour_stage_end WHERE stage_id = ?",
            params![stage],
        )?;
        return Ok(());
    };
    let (kind, count, scope) = match &end.event_count {
        Some(condition) => (
            Some(to_text(&condition.event)?),
            Some(condition.count),
            Some(to_text(&condition.scope)?),
        ),
        None => (None, None, None),
    };
    tx.execute(
        "INSERT INTO behaviour_stage_end
             (stage_id, duration_seconds, strategy, event_kind, event_count, event_scope)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(stage_id) DO UPDATE SET duration_seconds = excluded.duration_seconds,
                                             strategy = excluded.strategy,
                                             event_kind = excluded.event_kind,
                                             event_count = excluded.event_count,
                                             event_scope = excluded.event_scope",
        params![
            stage,
            end.duration_seconds,
            to_text(&end.strategy)?,
            kind,
            count,
            scope
        ],
    )?;
    Ok(())
}

fn write_stage_movement(
    tx: &Transaction<'_>,
    stage: &str,
    movement: Option<&Movement>,
) -> Result<()> {
    let Some(movement) = movement else {
        tx.execute(
            "DELETE FROM behaviour_stage_movement WHERE stage_id = ?",
            params![stage],
        )?;
        return Ok(());
    };
    tx.execute(
        "INSERT INTO behaviour_stage_movement (stage_id, minimum_speed, maximum_speed)
         VALUES (?, ?, ?)
         ON CONFLICT(stage_id) DO UPDATE SET minimum_speed = excluded.minimum_speed,
                                             maximum_speed = excluded.maximum_speed",
        params![stage, movement.minimum_speed, movement.maximum_speed],
    )?;
    Ok(())
}

fn write_stage_mitosis(tx: &Transaction<'_>, stage: &str, mitosis: Option<&Mitosis>) -> Result<()> {
    let Some(mitosis) = mitosis else {
        tx.execute(
            "DELETE FROM behaviour_stage_mitosis WHERE stage_id = ?",
            params![stage],
        )?;
        return Ok(());
    };
    tx.execute(
        "INSERT INTO behaviour_stage_mitosis (stage_id, chance, count) VALUES (?, ?, ?)
         ON CONFLICT(stage_id) DO UPDATE SET chance = excluded.chance, count = excluded.count",
        params![stage, mitosis.chance, mitosis.count],
    )?;
    Ok(())
}

fn write_stage_events(tx: &Transaction<'_>, stage: &str, events: &Events) -> Result<()> {
    let scheduled = [
        ("popup", events.popup.as_ref()),
        ("web", events.web.as_ref()),
        ("notification", events.notification.as_ref()),
        ("prompt", events.prompt.as_ref()),
        ("subliminal", events.subliminal.as_ref()),
    ];
    for (kind, schedule) in scheduled {
        let Some(schedule) = schedule else {
            tx.execute(
                "DELETE FROM behaviour_stage_event WHERE stage_id = ? AND kind = ?",
                params![stage, kind],
            )?;
            continue;
        };
        let (interval_kind, seconds, minimum, maximum) = match schedule.interval {
            Interval::Fixed { seconds } => ("fixed", Some(seconds), None, None),
            Interval::Random {
                minimum_seconds,
                maximum_seconds,
            } => ("random", None, Some(minimum_seconds), Some(maximum_seconds)),
        };
        tx.execute(
            "INSERT INTO behaviour_stage_event
                 (stage_id, kind, interval_kind, seconds, minimum_seconds, maximum_seconds,
                  initial_delay_seconds, max_concurrent)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(stage_id, kind) DO UPDATE SET
                 interval_kind = excluded.interval_kind,
                 seconds = excluded.seconds,
                 minimum_seconds = excluded.minimum_seconds,
                 maximum_seconds = excluded.maximum_seconds,
                 initial_delay_seconds = excluded.initial_delay_seconds,
                 max_concurrent = excluded.max_concurrent",
            params![
                stage,
                kind,
                interval_kind,
                seconds,
                minimum,
                maximum,
                schedule.initial_delay_seconds,
                schedule.max_concurrent,
            ],
        )?;
    }
    Ok(())
}

fn write_transitions(tx: &Transaction<'_>, transitions: &[Transition]) -> Result<()> {
    retain(
        tx,
        "behaviour_transition",
        "id",
        transitions.iter().map(|t| t.id.as_str()),
    )?;
    for (position, transition) in transitions.iter().enumerate() {
        tx.execute(
            "INSERT INTO behaviour_transition
                 (id, position, from_stage, to_stage, duration_seconds, easing)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET position = excluded.position,
                                           from_stage = excluded.from_stage,
                                           to_stage = excluded.to_stage,
                                           duration_seconds = excluded.duration_seconds,
                                           easing = excluded.easing",
            params![
                transition.id,
                position as i64,
                transition.from_stage,
                transition.to_stage,
                transition.duration_seconds,
                to_text(&transition.easing)?,
            ],
        )?;
        let categories = transition
            .affected
            .iter()
            .map(to_text)
            .collect::<Result<Vec<_>>>()?;
        tx.execute(
            "DELETE FROM behaviour_transition_category WHERE transition_id = ?1
             AND category NOT IN (SELECT value FROM json_each(?2))",
            params![transition.id, serde_json::to_string(&categories)?],
        )?;
        for (position, category) in categories.iter().enumerate() {
            tx.execute(
                "INSERT INTO behaviour_transition_category (transition_id, category, position)
                 VALUES (?, ?, ?)
                 ON CONFLICT(transition_id, category) DO UPDATE SET position = excluded.position",
                params![transition.id, category, position as i64],
            )?;
        }
    }
    Ok(())
}

/// Deletes every row of `table` whose `key` column is not in `keep`.
///
/// The delete half of the diff. Passing the surviving keys as JSON rather than building an `IN
/// (?, ?, …)` keeps one prepared statement whatever the timeline's length.
fn retain<'a>(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    keep: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let keep: Vec<&str> = keep.collect();
    tx.execute(
        &format!("DELETE FROM {table} WHERE {key} NOT IN (SELECT value FROM json_each(?))"),
        params![serde_json::to_string(&keep)?],
    )?;
    Ok(())
}

/// An enum's database text, which is deliberately the same string its JSON uses.
///
/// Routed through serde rather than a hand-written match so the two representations cannot drift:
/// renaming a variant's `rename_all` spelling changes both at once.
fn to_text<T: Serialize>(value: &T) -> Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(text) => Ok(text),
        other => bail!("expected a string-valued enum, got {other}"),
    }
}

fn from_text<T: DeserializeOwned>(text: &str) -> Result<T> {
    Ok(serde_json::from_value(serde_json::Value::String(
        text.to_string(),
    ))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behaviour::{
        ContentGroup, CountScope, Easing, EndStrategy, EventKind, PromptSettings, TextItem,
        TransitionCategory, WebLink,
    };

    /// A migrated pack database with one media file, so the slot foreign keys have something real
    /// to point at.
    fn pack() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        crate::db::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO media (id, file_name, file_type, \"offset\", length, hash)
             VALUES (1, 'bg.png', 'image', 0, 0, x'00'), (2, 'intro.gif', 'video', 0, 0, x'01')",
            [],
        )
        .unwrap();
        conn
    }

    fn store(conn: &mut Connection, behaviour: &Behaviour) {
        let tx = conn.transaction().unwrap();
        write(&tx, behaviour).unwrap();
        tx.commit().unwrap();
    }

    fn stage(id: &str) -> Stage {
        Stage {
            id: id.to_string(),
            label: format!("Stage {id}"),
            end: None,
            content: ContentSelection::default(),
            events: Events::default(),
            movement: None,
            mitosis: None,
        }
    }

    /// Everything the structural half can express, in one document, through a full round trip.
    #[test]
    fn a_full_document_survives_the_round_trip() {
        let mut conn = pack();
        let mut first = stage("a");
        first.content.tags = Some(vec!["kinky".to_string(), "soft".to_string()]);
        first.content.wallpaper = Some(1);
        first.end = Some(StageEnd {
            duration_seconds: Some(300.0),
            event_count: Some(EventCountCondition {
                event: EventKind::Popup,
                count: 10,
                scope: CountScope::Session,
            }),
            strategy: EndStrategy::All,
        });
        first.movement = Some(Movement {
            minimum_speed: Some(50.0),
            maximum_speed: None,
        });
        first.mitosis = Some(Mitosis {
            chance: Some(0.5),
            count: Some(2),
        });
        first.events.popup = Some(EventSchedule {
            interval: Interval::Fixed { seconds: 30.0 },
            initial_delay_seconds: Some(5.0),
            max_concurrent: Some(3),
        });
        first.events.web = Some(EventSchedule {
            interval: Interval::Random {
                minimum_seconds: 10.0,
                maximum_seconds: 20.0,
            },
            initial_delay_seconds: None,
            max_concurrent: None,
        });

        let behaviour = Behaviour {
            content: crate::behaviour::Content {
                content_groups: vec![ContentGroup {
                    id: "kinky".to_string(),
                    label: "Kinky".to_string(),
                    description: Some("Kinky-tagged content".to_string()),
                    tags: vec!["kinky".to_string(), "soft".to_string()],
                    enabled_by_default: false,
                }],
                captions: vec![
                    TextItem {
                        text: "Obey.".to_string(),
                        tags: vec!["kinky".to_string()],
                    },
                    TextItem {
                        text: "Good.".to_string(),
                        tags: vec![],
                    },
                ],
                prompts: vec![TextItem {
                    text: "Type this.".to_string(),
                    tags: vec![],
                }],
                prompt_settings: PromptSettings {
                    submit_label: Some("Obey".to_string()),
                },
                notifications: vec![TextItem {
                    text: "Ping.".to_string(),
                    tags: vec!["soft".to_string()],
                }],
                subliminals: vec![TextItem {
                    text: "Deeper.".to_string(),
                    tags: vec![],
                }],
                web_links: vec![WebLink {
                    url: "https://duckduckgo.com/?q=".to_string(),
                    args: vec!["one".to_string(), "two".to_string()],
                    tags: vec!["kinky".to_string()],
                }],
                wallpaper: Some(1),
                splash: Some(2),
            },
            experience: Some(Experience {
                timeline: Timeline {
                    stages: vec![first, stage("b")],
                    transitions: vec![Transition {
                        id: "t1".to_string(),
                        from_stage: "a".to_string(),
                        to_stage: "b".to_string(),
                        duration_seconds: 12.5,
                        easing: Easing::EaseInOut,
                        affected: vec![
                            TransitionCategory::MitosisChance,
                            TransitionCategory::PopupInterval,
                        ],
                    }],
                },
                label: Some("Corruption".to_string()),
            }),
        };

        store(&mut conn, &behaviour);
        assert_eq!(read(&conn).unwrap(), behaviour);
    }

    /// The three-state tag field. "No restriction" and "restricted to nothing" are different
    /// answers and an empty table cannot tell them apart on its own.
    #[test]
    fn an_empty_tag_selection_stays_distinct_from_no_selection() {
        let mut conn = pack();
        let mut unrestricted = stage("a");
        unrestricted.content.tags = None;
        let mut restricted = stage("b");
        restricted.content.tags = Some(vec![]);

        let behaviour = Behaviour {
            experience: Some(Experience {
                timeline: Timeline {
                    stages: vec![unrestricted, restricted],
                    transitions: vec![],
                },
                label: None,
            }),
            ..Behaviour::new()
        };
        store(&mut conn, &behaviour);

        let stages = read(&conn).unwrap().experience.unwrap().timeline.stages;
        assert_eq!(stages[0].content.tags, None);
        assert_eq!(stages[1].content.tags, Some(vec![]));
    }

    /// Same shape for the optional sub-structures: `Some(Movement { .. all None })` is a stage
    /// that has movement with no speeds set, not a stage with no movement.
    #[test]
    fn an_empty_sub_structure_stays_distinct_from_an_absent_one() {
        let mut conn = pack();
        let mut present = stage("a");
        present.movement = Some(Movement {
            minimum_speed: None,
            maximum_speed: None,
        });
        present.mitosis = Some(Mitosis {
            chance: None,
            count: None,
        });
        let absent = stage("b");

        let behaviour = Behaviour {
            experience: Some(Experience {
                timeline: Timeline {
                    stages: vec![present, absent],
                    transitions: vec![],
                },
                label: None,
            }),
            ..Behaviour::new()
        };
        store(&mut conn, &behaviour);

        let stages = read(&conn).unwrap().experience.unwrap().timeline.stages;
        assert_eq!(
            stages[0].movement,
            Some(Movement {
                minimum_speed: None,
                maximum_speed: None
            })
        );
        assert_eq!(
            stages[0].mitosis,
            Some(Mitosis {
                chance: None,
                count: None
            })
        );
        assert_eq!(stages[1].movement, None);
        assert_eq!(stages[1].mitosis, None);
    }

    /// A pack with no timeline, versus one whose timeline simply doesn't rename the mode. Both
    /// read back as themselves rather than collapsing into each other.
    #[test]
    fn a_suspended_timeline_is_distinct_from_an_unlabelled_one() {
        let mut conn = pack();
        let with_timeline = Behaviour {
            experience: Some(Experience::default()),
            ..Behaviour::new()
        };
        store(&mut conn, &with_timeline);
        assert_eq!(read(&conn).unwrap().experience, Some(Experience::default()));

        store(&mut conn, &Behaviour::new());
        assert_eq!(read(&conn).unwrap().experience, None);
    }

    /// The reason writes diff instead of replacing: undo is a changeset, so a wholesale rewrite
    /// would record every stage as changed on every keystroke. Editing one stage's label must
    /// leave every other row exactly as it was.
    #[test]
    fn editing_one_stage_leaves_the_other_rows_untouched() {
        let mut conn = pack();
        let mut behaviour = Behaviour {
            experience: Some(Experience {
                timeline: Timeline {
                    stages: vec![stage("a"), stage("b"), stage("c")],
                    transitions: vec![],
                },
                label: None,
            }),
            ..Behaviour::new()
        };
        store(&mut conn, &behaviour);

        // `data_version` ticks once per committed write to the database file, so it cannot show
        // which rows moved. Row-level `rowid`s can: an untouched row keeps its identity, while a
        // delete-and-reinsert gives it a new one.
        let rowids = |conn: &Connection| -> Vec<(String, i64)> {
            let mut statement = conn
                .prepare("SELECT id, rowid FROM behaviour_stage ORDER BY position")
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        let before = rowids(&conn);

        behaviour.experience.as_mut().unwrap().timeline.stages[1].label = "Renamed".to_string();
        store(&mut conn, &behaviour);

        assert_eq!(before, rowids(&conn), "no stage row was replaced");
        let stages = read(&conn).unwrap().experience.unwrap().timeline.stages;
        assert_eq!(stages[1].label, "Renamed");
    }

    /// Removing a stage takes its child rows with it, and the transition that ran through it --
    /// the cascades the schema declares, rather than a cleanup written out longhand.
    #[test]
    fn removing_a_stage_takes_its_children_and_transitions() {
        let mut conn = pack();
        let mut first = stage("a");
        first.content.tags = Some(vec!["kinky".to_string()]);
        first.movement = Some(Movement {
            minimum_speed: None,
            maximum_speed: None,
        });
        first.events.popup = Some(EventSchedule {
            interval: Interval::Fixed { seconds: 30.0 },
            initial_delay_seconds: None,
            max_concurrent: None,
        });
        let mut behaviour = Behaviour {
            experience: Some(Experience {
                timeline: Timeline {
                    stages: vec![first, stage("b")],
                    transitions: vec![Transition {
                        id: "t1".to_string(),
                        from_stage: "a".to_string(),
                        to_stage: "b".to_string(),
                        duration_seconds: 0.0,
                        easing: Easing::Linear,
                        affected: vec![TransitionCategory::Events],
                    }],
                },
                label: None,
            }),
            ..Behaviour::new()
        };
        store(&mut conn, &behaviour);

        let timeline = &mut behaviour.experience.as_mut().unwrap().timeline;
        timeline.stages.remove(0);
        timeline.transitions.clear();
        store(&mut conn, &behaviour);

        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM behaviour_stage"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM behaviour_stage_tag"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM behaviour_stage_movement"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM behaviour_stage_event"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM behaviour_transition"), 0);
        assert_eq!(
            count("SELECT COUNT(*) FROM behaviour_transition_category"),
            0
        );
    }

    /// What the tag joins are for. A tag list in the document is a foreign key into `tags`, so a
    /// tag the author typed into a caption is a real tag the pack has -- which is what lets
    /// renaming one be `UPDATE tags SET name` instead of a pass over the whole document.
    #[test]
    fn a_tag_named_by_the_document_becomes_a_real_tag() {
        let mut conn = pack();
        let behaviour = Behaviour {
            content: crate::behaviour::Content {
                captions: vec![TextItem {
                    text: "Obey.".to_string(),
                    tags: vec!["fresh".to_string()],
                }],
                ..Default::default()
            },
            ..Behaviour::new()
        };
        store(&mut conn, &behaviour);

        let id: i64 = conn
            .query_row("SELECT id FROM tags WHERE name = 'fresh'", [], |row| {
                row.get(0)
            })
            .unwrap();

        // Renaming the tag row is the whole operation: the caption follows because it holds the id.
        conn.execute("UPDATE tags SET name = 'renamed' WHERE id = ?", [id])
            .unwrap();
        assert_eq!(
            read(&conn).unwrap().content.captions[0].tags,
            vec!["renamed".to_string()]
        );

        // And deleting it cascades, which is what "delete this tag" used to mean longhand.
        conn.execute("DELETE FROM tags WHERE id = ?", [id]).unwrap();
        assert!(read(&conn).unwrap().content.captions[0].tags.is_empty());
    }

    /// The point of the whole step: a slot is a foreign key now, so pointing one at media the pack
    /// does not have is refused by the database rather than discovered later.
    #[test]
    fn a_slot_cannot_point_at_media_the_pack_does_not_have() {
        let mut conn = pack();
        let behaviour = Behaviour {
            content: crate::behaviour::Content {
                wallpaper: Some(404),
                ..Default::default()
            },
            ..Behaviour::new()
        };
        let tx = conn.transaction().unwrap();
        let error = write(&tx, &behaviour).unwrap_err();
        assert!(
            error.to_string().to_lowercase().contains("foreign key"),
            "unexpected error: {error}"
        );
    }

    /// Nothing is left in `pack_data`: the document is rows, end to end.
    #[test]
    fn the_document_no_longer_touches_pack_data() {
        let mut conn = pack();
        let behaviour = Behaviour {
            content: crate::behaviour::Content {
                captions: vec![TextItem {
                    text: "Obey.".to_string(),
                    tags: vec!["kinky".to_string()],
                }],
                wallpaper: Some(1),
                ..Default::default()
            },
            experience: Some(Experience::default()),
        };
        store(&mut conn, &behaviour);

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM pack_data", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0, "the behaviour blob is gone, not merely smaller");
        assert_eq!(read(&conn).unwrap(), behaviour);
    }

    /// A pack still carrying the old blob is one the tables cannot describe, so it is refused
    /// rather than read as an empty document.
    #[test]
    fn a_pack_from_before_the_content_tables_is_refused() {
        let conn = pack();
        conn.execute(
            "INSERT INTO pack_data (name, blob) VALUES ('behaviour', ?)",
            params![br#"{"version":5,"content":{}}"#],
        )
        .unwrap();

        let error = read(&conn).unwrap_err();
        assert!(error.to_string().contains("re-import"), "{error}");
    }
}
