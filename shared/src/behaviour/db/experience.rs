use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::stage::{read_stages, write_stages};
use super::transition::{read_transitions, write_transitions};
use crate::behaviour::schema::{Experience, Timeline};

// ── The experience section ───────────────────────────────────────────────────

/// The stored timeline, its optional label, and whether it is enabled.
///
/// Unlike the whole-document read, this preserves a disabled timeline for the editor.
pub fn read_timeline(conn: &Connection) -> Result<Option<(Timeline, Option<String>, bool)>> {
    let row: Option<(Option<String>, bool)> = conn
        .query_row(
            "SELECT label, enabled FROM behaviour_experience WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((label, enabled)) = row else {
        return Ok(None);
    };
    Ok(Some((
        Timeline {
            stages: read_stages(conn)?,
            transitions: read_transitions(conn)?,
        },
        label,
        enabled,
    )))
}

pub fn read_experience(conn: &Connection) -> Result<Option<Experience>> {
    let row: Option<(Option<String>, bool)> = conn
        .query_row(
            "SELECT label, enabled FROM behaviour_experience WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    // No row at all is a pack with no timeline; a row with a NULL label is a timeline that simply
    // doesn't override the mode's name.
    let Some((label, enabled)) = row else {
        return Ok(None);
    };
    // A suspended timeline reads exactly as no timeline, which is what keeps this change invisible
    // to the engine, the converter and `lw`. Its stage rows are still there -- see
    // The db-level `read_timeline` keeps them available to the editor.
    if !enabled {
        return Ok(None);
    }
    Ok(Some(Experience {
        timeline: Timeline {
            stages: read_stages(conn)?,
            transitions: read_transitions(conn)?,
        },
        label,
    }))
}

pub fn write_experience(tx: &Transaction<'_>, experience: Option<&Experience>) -> Result<()> {
    let Some(experience) = experience else {
        // The whole-document writer cleared the previous roots before reaching this point.
        return Ok(());
    };
    tx.execute(
        "INSERT INTO behaviour_experience (singleton, label, enabled) VALUES (1, ?, 1)",
        params![experience.label],
    )?;
    // Stages first: a transition's endpoints are foreign keys, so a transition into a stage this
    // write is adding can only be inserted once that stage exists. Removing a stage cascades to
    // the transitions touching it, which is why they are reconciled afterwards rather than before.
    write_stages(tx, &experience.timeline.stages)?;
    write_transitions(tx, &experience.timeline.transitions)?;
    Ok(())
}
