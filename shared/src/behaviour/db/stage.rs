use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, named_params, params};

use super::tags::tag_id;
use crate::behaviour::{
    db::common::sql_array,
    schema::{
        ContentSelection, CountScope, EndStrategy, EventCountCondition, EventKind, EventSchedule,
        Events, Interval, Mitosis, Movement, Stage, StageEnd, StageEntry, StagePrompt,
    },
};

pub fn read_stages(conn: &Connection) -> Result<Vec<Stage>> {
    read_stages_where(conn, "", &[])
}

pub fn read_one_stage(conn: &Connection, id: &str) -> Result<Option<Stage>> {
    Ok(read_stages_where(conn, "WHERE id = ?", params![id])?.pop())
}

fn read_stages_where(
    conn: &Connection,
    filter: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<Stage>> {
    let mut statement = conn.prepare(&format!(
        "SELECT id, label, restricts_content, wallpaper, audio, audio_random, \
                entry_splash_media, entry_sound_media, entry_popup_burst, entry_notification_text, \
                prompt_timeouts_enabled, prompt_timeout_multiplier, prompt_popup_burst, prompt_sound_media \
         FROM behaviour_stage \
         {filter} ORDER BY position"
    ))?;

    statement
        .query_and_then(params, |row| -> anyhow::Result<_> {
            let id: String = row.get("id")?;

            Ok(Stage {
                content: ContentSelection {
                    tags: if row.get("restricts_content")? {
                        Some(read_stage_tags(conn, &id, false)?)
                    } else {
                        None
                    },
                    exclude: read_stage_tags(conn, &id, true)?,
                    owned_tag: read_owned_stage_tag(conn, &id, false)?,
                    owned_exclude_tag: read_owned_stage_tag(conn, &id, true)?,
                    wallpaper: row.get("wallpaper")?,
                    audio: row.get("audio")?,
                    audio_random: row.get("audio_random")?,
                },
                end: read_stage_end(conn, &id)?,
                events: read_stage_events(conn, &id)?,
                movement: read_stage_movement(conn, &id)?,
                mitosis: read_stage_mitosis(conn, &id)?,
                on_enter: StageEntry {
                    splash: row.get("entry_splash_media")?,
                    sound: row.get("entry_sound_media")?,
                    popup_burst: row.get("entry_popup_burst")?,
                    notification: row.get("entry_notification_text")?,
                },
                prompt: StagePrompt {
                    timeouts_enabled: row.get("prompt_timeouts_enabled")?,
                    timeout_multiplier: row.get("prompt_timeout_multiplier")?,
                    popup_burst: row.get("prompt_popup_burst")?,
                    sound: row.get("prompt_sound_media")?,
                },
                id,
                label: row.get("label")?,
            })
        })?
        .collect()
}

pub fn read_stage_entry(conn: &Connection, stage: &str) -> Result<Option<StageEntry>> {
    Ok(conn
        .query_row(
            "SELECT entry_splash_media, entry_sound_media, entry_popup_burst, entry_notification_text
         FROM behaviour_stage WHERE id = :stage",
            named_params![":stage": stage],
            |row| {
                Ok(StageEntry {
                    splash: row.get("entry_splash_media")?,
                    sound: row.get("entry_sound_media")?,
                    popup_burst: row.get("entry_popup_burst")?,
                    notification: row.get("entry_notification_text")?,
                })
            },
        )
        .optional()?)
}

pub fn read_stage_prompt(conn: &Connection, stage: &str) -> Result<StagePrompt> {
    Ok(conn
        .query_row(
            "SELECT prompt_timeouts_enabled, prompt_timeout_multiplier, prompt_popup_burst, prompt_sound_media
         FROM behaviour_stage WHERE id = :stage",
            named_params![":stage": stage],
            |row| {
                Ok(StagePrompt {
                    timeouts_enabled: row.get("prompt_timeouts_enabled")?,
                    timeout_multiplier: row.get("prompt_timeout_multiplier")?,
                    popup_burst: row.get("prompt_popup_burst")?,
                    sound: row.get("prompt_sound_media")?,
                })
            },
        )
        .optional()?
        .unwrap_or_default())
}

pub fn read_owned_stage_tag(
    conn: &Connection,
    stage: &str,
    excluded: bool,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT tags.name FROM behaviour_stage_tag
             JOIN tags ON tags.id = behaviour_stage_tag.tag_id
             WHERE behaviour_stage_tag.stage_id = :stage AND behaviour_stage_tag.owned = TRUE
               AND behaviour_stage_tag.excluded = :excluded
             ORDER BY behaviour_stage_tag.id LIMIT 1",
            named_params![":stage": stage, ":excluded": excluded],
            |row| row.get("name"),
        )
        .optional()?)
}

pub fn read_stage_tags(conn: &Connection, stage: &str, excluded: bool) -> Result<Vec<String>> {
    let mut statement = conn.prepare(
        "SELECT tags.name FROM behaviour_stage_tag
         JOIN tags ON tags.id = behaviour_stage_tag.tag_id
         WHERE behaviour_stage_tag.stage_id = :stage AND behaviour_stage_tag.excluded = :excluded
         ORDER BY behaviour_stage_tag.id",
    )?;
    let tags = statement
        .query_map(
            named_params![":stage": stage, ":excluded": excluded],
            |row| row.get("name"),
        )?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tags)
}

pub fn write_stage_tags(
    tx: &Transaction<'_>,
    stage: &str,
    include: &[String],
    exclude: &[String],
) -> Result<()> {
    let ids = |tags: &[String]| {
        tags.iter()
            .map(|tag| tag_id(tx, tag))
            .collect::<Result<Vec<_>>>()
    };

    let (included, excluded) = (ids(include)?, ids(exclude)?);
    let keeping: Vec<i64> = included.iter().chain(excluded.iter()).copied().collect();

    tx.execute(
        "DELETE FROM behaviour_stage_tag WHERE stage_id = :stage
         AND tag_id NOT IN (SELECT value FROM rarray(:keeping))",
        named_params![":stage": stage, ":keeping": sql_array(keeping)?],
    )?;

    for (side, ids) in [(false, &included), (true, &excluded)] {
        for id in ids {
            tx.execute(
                "INSERT INTO behaviour_stage_tag (stage_id, tag_id, excluded)
                 VALUES (:stage, :id, :side)
                 ON CONFLICT(stage_id, tag_id) DO UPDATE SET excluded = excluded.excluded",
                named_params![":stage": stage, ":id": id, ":side": side],
            )?;
        }
    }
    Ok(())
}

/// Claim a tag as owned by a stage
pub fn write_owned_stage_tag(
    tx: &Transaction<'_>,
    stage: &str,
    excluded: bool,
    owned: Option<&str>,
) -> Result<()> {
    tx.execute(
        "UPDATE behaviour_stage_tag SET owned = FALSE
         WHERE stage_id = :stage AND owned = TRUE AND excluded = :excluded
         AND tag_id IS NOT (SELECT id FROM tags WHERE name = :owned)",
        named_params![":stage": stage, ":owned": owned, ":excluded": excluded],
    )?;

    if let Some(tag) = owned {
        tx.execute(
            "UPDATE behaviour_stage_tag SET owned = TRUE
             WHERE stage_id = :stage AND owned = FALSE AND excluded = :excluded
             AND tag_id = (SELECT id FROM tags WHERE name = :tag)",
            named_params![":stage": stage, ":tag": tag, ":excluded": excluded],
        )?;
    }

    Ok(())
}

pub fn read_stage_end(conn: &Connection, stage: &str) -> Result<Option<StageEnd>> {
    Ok(conn
        .query_row_and_then(
            "SELECT duration_seconds, strategy, event_kind, event_count, event_scope
             FROM behaviour_stage_end WHERE stage_id = :stage",
            named_params![":stage": stage],
            |row| {
                let duration_seconds = row.get::<_, Option<f64>>("duration_seconds")?;
                let strategy = row.get::<_, EndStrategy>("strategy")?;
                let kind = row.get::<_, Option<EventKind>>("event_kind")?;
                let count = row.get::<_, Option<u32>>("event_count")?;
                let scope = row.get::<_, Option<CountScope>>("event_scope")?;

                let event_count = match (kind, count, scope) {
                    (Some(event), Some(count), Some(scope)) => Some(EventCountCondition {
                        event,
                        count,
                        scope,
                    }),
                    _ => None,
                };

                Ok(StageEnd {
                    duration_seconds,
                    event_count,
                    strategy,
                })
            },
        )
        .optional()?)
}

pub fn read_stage_movement(conn: &Connection, stage: &str) -> Result<Option<Movement>> {
    Ok(conn
        .query_row(
            "SELECT minimum_speed, maximum_speed
             FROM behaviour_stage_movement WHERE stage_id = :stage",
            named_params![":stage": stage],
            |row| {
                Ok(Movement {
                    minimum_speed: row.get("minimum_speed")?,
                    maximum_speed: row.get("maximum_speed")?,
                })
            },
        )
        .optional()?)
}

pub fn read_stage_mitosis(conn: &Connection, stage: &str) -> Result<Option<Mitosis>> {
    Ok(conn
        .query_row(
            "SELECT chance, count FROM behaviour_stage_mitosis WHERE stage_id = :stage",
            named_params![":stage": stage],
            |row| {
                Ok(Mitosis {
                    chance: row.get("chance")?,
                    count: row.get("count")?,
                })
            },
        )
        .optional()?)
}

pub fn read_stage_events(conn: &Connection, stage: &str) -> Result<Events> {
    let mut statement = conn.prepare(
        "SELECT kind, interval_kind, seconds, minimum_seconds, maximum_seconds,
                initial_delay_seconds, max_concurrent
         FROM behaviour_stage_event WHERE stage_id = :stage",
    )?;
    let mut events = Events::default();

    let rows = statement.query_and_then(named_params![":stage": stage], |row| -> Result<_> {
        let kind: EventKind = row.get("kind")?;
        let interval_kind: String = row.get("interval_kind")?;
        let seconds: Option<f64> = row.get("seconds")?;
        let minimum_seconds: Option<f64> = row.get("minimum_seconds")?;
        let maximum_seconds: Option<f64> = row.get("maximum_seconds")?;
        let initial_delay_seconds: Option<f64> = row.get("initial_delay_seconds")?;
        let max_concurrent: Option<u32> = row.get("max_concurrent")?;

        let interval = match interval_kind.as_str() {
            "fixed" => Interval::Fixed {
                seconds: seconds.unwrap_or_default(),
            },
            "random" => Interval::Random {
                minimum_seconds: minimum_seconds.unwrap_or_default(),
                maximum_seconds: maximum_seconds.unwrap_or_default(),
            },
            other => bail!("unknown event interval kind {other:?}"),
        };

        let schedule = EventSchedule {
            interval,
            initial_delay_seconds,
            max_concurrent,
        };

        Ok((kind, schedule))
    })?;

    for item in rows {
        let (kind, schedule) = item?;
        match kind {
            EventKind::Popup => events.popup = Some(schedule),
            EventKind::Web => events.web = Some(schedule),
            EventKind::Notification => events.notification = Some(schedule),
            EventKind::Prompt => events.prompt = Some(schedule),
            EventKind::Sound => events.sound = Some(schedule),
        }
    }
    Ok(events)
}

pub fn write_stages(tx: &Transaction<'_>, stages: &[Stage]) -> Result<()> {
    for (position, stage) in stages.iter().enumerate() {
        write_one_stage(tx, position as i64, stage)?;
    }
    Ok(())
}

pub fn write_one_stage(tx: &Transaction<'_>, position: i64, stage: &Stage) -> Result<()> {
    tx.execute(
        "INSERT INTO behaviour_stage (id, position, label, restricts_content, wallpaper, audio, audio_random)
         VALUES (:id, :position, :label, :restricts_content, :wallpaper, :audio, :audio_random)
         ON CONFLICT(id) DO UPDATE SET position = excluded.position,
                                       label = excluded.label,
                                       restricts_content = excluded.restricts_content,
                                       wallpaper = excluded.wallpaper,
                                       audio = excluded.audio,
                                       audio_random = excluded.audio_random",
        named_params![
            ":id": stage.id,
            ":position": position,
            ":label": stage.label,
            ":restricts_content": stage.content.tags.is_some(),
            ":wallpaper": stage.content.wallpaper,
            ":audio": stage.content.audio,
            ":audio_random": stage.content.audio_random,
        ],
    )?;

    write_stage_tags(
        tx,
        &stage.id,
        stage.content.tags.as_deref().unwrap_or(&[]),
        &stage.content.exclude,
    )?;
    write_owned_stage_tag(tx, &stage.id, false, stage.content.owned_tag.as_deref())?;
    write_owned_stage_tag(
        tx,
        &stage.id,
        true,
        stage.content.owned_exclude_tag.as_deref(),
    )?;
    write_stage_end(tx, &stage.id, stage.end.as_ref())?;
    write_stage_movement(tx, &stage.id, stage.movement.as_ref())?;
    write_stage_mitosis(tx, &stage.id, stage.mitosis.as_ref())?;
    write_stage_entry(
        tx,
        &stage.id,
        (!stage.on_enter.is_default()).then_some(&stage.on_enter),
    )?;
    write_stage_prompt(tx, &stage.id, &stage.prompt)?;
    write_stage_events(tx, &stage.id, &stage.events)?;

    Ok(())
}

pub fn write_stage_entry(
    tx: &Transaction<'_>,
    stage: &str,
    entry: Option<&StageEntry>,
) -> Result<()> {
    let entry = entry.cloned().unwrap_or_default();

    tx.execute(
        "UPDATE behaviour_stage SET entry_splash_media = ?1, entry_sound_media = ?2,
             entry_popup_burst = ?3, entry_notification_text = ?4 WHERE id = ?5",
        params![
            entry.splash,
            entry.sound,
            entry.popup_burst,
            entry.notification,
            stage,
        ],
    )?;

    Ok(())
}

pub fn write_stage_prompt(tx: &Transaction<'_>, stage: &str, prompt: &StagePrompt) -> Result<()> {
    tx.execute(
        "UPDATE behaviour_stage SET prompt_timeouts_enabled = ?1,
             prompt_timeout_multiplier = ?2, prompt_popup_burst = ?3,
             prompt_sound_media = ?4 WHERE id = ?5",
        params![
            prompt.timeouts_enabled,
            prompt.timeout_multiplier,
            prompt.popup_burst,
            prompt.sound,
            stage,
        ],
    )?;

    Ok(())
}

pub fn write_stage_end(tx: &Transaction<'_>, stage: &str, end: Option<&StageEnd>) -> Result<()> {
    let Some(end) = end else {
        tx.execute(
            "DELETE FROM behaviour_stage_end WHERE stage_id = ?",
            params![stage],
        )?;
        return Ok(());
    };

    let (kind, count, scope) = match &end.event_count {
        Some(condition) => (
            Some(condition.event),
            Some(condition.count),
            Some(condition.scope),
        ),
        None => (None, None, None),
    };

    tx.execute(
        "INSERT INTO behaviour_stage_end
             (stage_id, duration_seconds, strategy, event_kind, event_count, event_scope)
         VALUES (:stage, :duration_seconds, :strategy, :kind, :count, :scope)
         ON CONFLICT(stage_id) DO UPDATE SET duration_seconds = excluded.duration_seconds,
                                             strategy = excluded.strategy,
                                             event_kind = excluded.event_kind,
                                             event_count = excluded.event_count,
                                             event_scope = excluded.event_scope",
        named_params![
            ":stage": stage,
            ":duration_seconds": end.duration_seconds,
            ":strategy": end.strategy,
            ":kind": kind,
            ":count": count,
            ":scope": scope
        ],
    )?;

    Ok(())
}

pub fn write_stage_movement(
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
         VALUES (:stage, :minimum_speed, :maximum_speed)
         ON CONFLICT(stage_id) DO UPDATE SET minimum_speed = excluded.minimum_speed,
                                             maximum_speed = excluded.maximum_speed",
        named_params![
            ":stage": stage,
            ":minimum_speed": movement.minimum_speed,
            ":maximum_speed": movement.maximum_speed
        ],
    )?;

    Ok(())
}

pub fn write_stage_mitosis(
    tx: &Transaction<'_>,
    stage: &str,
    mitosis: Option<&Mitosis>,
) -> Result<()> {
    let Some(mitosis) = mitosis else {
        tx.execute(
            "DELETE FROM behaviour_stage_mitosis WHERE stage_id = ?",
            params![stage],
        )?;
        return Ok(());
    };

    tx.execute(
        "INSERT INTO behaviour_stage_mitosis (stage_id, chance, count) VALUES (:stage, :chance, :count)
         ON CONFLICT(stage_id) DO UPDATE SET chance = excluded.chance, count = excluded.count",
        named_params![":stage": stage, ":chance": mitosis.chance, ":count": mitosis.count],
    )?;

    Ok(())
}

pub fn write_one_stage_event(
    tx: &Transaction<'_>,
    stage: &str,
    kind: EventKind,
    schedule: Option<&EventSchedule>,
) -> Result<()> {
    let Some(schedule) = schedule else {
        tx.execute(
            "DELETE FROM behaviour_stage_event WHERE stage_id = ? AND kind = ?",
            params![stage, kind],
        )?;
        return Ok(());
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
         VALUES (:stage, :kind, :interval_kind, :seconds, :minimum, :maximum, :initial_delay_seconds, :max_concurrent)
         ON CONFLICT(stage_id, kind) DO UPDATE SET
             interval_kind = excluded.interval_kind,
             seconds = excluded.seconds,
             minimum_seconds = excluded.minimum_seconds,
             maximum_seconds = excluded.maximum_seconds,
             initial_delay_seconds = excluded.initial_delay_seconds,
             max_concurrent = excluded.max_concurrent",
        named_params![
            ":stage": stage,
            ":kind": kind,
            ":interval_kind": interval_kind,
            ":seconds": seconds,
            ":minimum": minimum,
            ":maximum": maximum,
            ":initial_delay_seconds": schedule.initial_delay_seconds,
            ":max_concurrent": schedule.max_concurrent,
        ],
    )?;

    Ok(())
}

pub fn write_stage_events(tx: &Transaction<'_>, stage: &str, events: &Events) -> Result<()> {
    let scheduled = [
        (EventKind::Popup, events.popup.as_ref()),
        (EventKind::Web, events.web.as_ref()),
        (EventKind::Notification, events.notification.as_ref()),
        (EventKind::Prompt, events.prompt.as_ref()),
        (EventKind::Sound, events.sound.as_ref()),
    ];

    for (kind, schedule) in scheduled {
        write_one_stage_event(tx, stage, kind, schedule)?;
    }

    Ok(())
}
