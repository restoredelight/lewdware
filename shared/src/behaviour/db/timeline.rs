use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::stage::{read_stages, write_stages};
use super::transition::{read_transitions, write_transitions};
use crate::behaviour::schema::Timeline;

pub fn timeline_enabled(conn: &Connection) -> Result<bool> {
    Ok(conn
        .query_row("SELECT enabled FROM behaviour_timeline", [], |row| {
            row.get(0)
        })
        .optional()?
        .unwrap_or(false))
}

pub fn read_timeline(conn: &Connection) -> Result<Option<Timeline>> {
    let row: Option<Option<String>> = conn
        .query_row("SELECT label FROM behaviour_timeline", [], |row| row.get(0))
        .optional()?;

    let Some(label) = row else {
        return Ok(None);
    };

    Ok(Some(Timeline {
        label,
        stages: read_stages(conn)?,
        transitions: read_transitions(conn)?,
    }))
}

pub fn read_timeline_if_enabled(conn: &Connection) -> Result<Option<Timeline>> {
    if timeline_enabled(conn)? {
        read_timeline(conn)
    } else {
        Ok(None)
    }
}

pub fn write_timeline(tx: &Transaction<'_>, timeline: &Timeline) -> Result<()> {
    tx.execute(
        "INSERT INTO behaviour_timeline (singleton, label, enabled) VALUES (1, ?, TRUE)",
        params![timeline.label],
    )?;

    write_stages(tx, &timeline.stages)?;
    write_transitions(tx, &timeline.transitions)?;

    Ok(())
}
