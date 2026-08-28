use std::collections::HashSet;

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::behaviour::db::{
    read_one_stage, read_stages, read_timeline, read_transitions, write_one_stage,
};
use crate::behaviour::schema::{Stage, Timeline, Transition, TransitionCategory};

// ── The timeline ─────────────────────────────────────────────────────────────
//
// Two invariants hold over the stage list, and every structural edit below restores them rather
// than asking its caller to:
//
// - **The last stage has no end condition.** There is nothing after it to move on to, so an end
//   there would be a promise the mode cannot keep.
// - **There is exactly one transition per adjacent pair of stages**, and a transition whose two
//   endpoints are still adjacent keeps its identity — so reordering stages around it does not
//   silently reset a duration the author set.

/// The default an end condition takes when a stage stops being last and needs one.
pub const DEFAULT_STAGE_SECONDS: f64 = 300.0;

/// The timeline as the editor sees it, including one that is currently switched off.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineView {
    pub timeline: Timeline,
    pub label: Option<String>,
    pub enabled: bool,
}

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Restores the two timeline invariants directly in the database:
///
/// 1. The last stage has no end condition; all prior stages have an end condition.
/// 2. Exactly one transition exists per adjacent stage pair (from_stage -> to_stage),
///    preserving existing transition IDs and settings where endpoints remain adjacent.
pub fn normalize_timeline(tx: &Transaction<'_>, mut new_id: impl FnMut() -> String) -> Result<()> {
    // 1. Invariant: Last stage has no end condition
    let max_pos: Option<i64> = tx
        .query_row("SELECT MAX(position) FROM behaviour_stage", [], |row| {
            row.get(0)
        })
        .optional()?
        .flatten();

    let Some(max_pos) = max_pos else {
        return Ok(());
    };

    // Remove end condition from the last stage
    tx.execute(
        "DELETE FROM behaviour_stage_end WHERE stage_id = (
             SELECT id FROM behaviour_stage WHERE position = ?
         )",
        params![max_pos],
    )?;

    // Non-last stages missing an end condition get the default one
    tx.execute(
        "INSERT INTO behaviour_stage_end (stage_id, duration_seconds, strategy)
         SELECT id, ?1, 'any' FROM behaviour_stage
         WHERE position < ?2 AND id NOT IN (SELECT stage_id FROM behaviour_stage_end)",
        params![DEFAULT_STAGE_SECONDS, max_pos],
    )?;

    // 2. Invariant: Transitions for adjacent stage pairs
    // Find all adjacent pairs (s1.id, s2.id) ordered by position. The pair's own place in the
    // timeline does not need carrying along: a transition is ordered by `from_stage`, so putting
    // it between these two stages is already all the ordering it has.
    let mut stmt = tx.prepare(
        "SELECT s1.id, s2.id
         FROM behaviour_stage s1
         JOIN behaviour_stage s2 ON s2.position = s1.position + 1
         ORDER BY s1.position",
    )?;

    let adjacent_pairs: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;

    // Load existing transitions: (id, from_stage, to_stage)
    let mut trans_stmt = tx.prepare("SELECT id, from_stage, to_stage FROM behaviour_transition")?;
    let existing_transitions: Vec<(String, String, String)> = trans_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;

    let mut kept_trans_ids = HashSet::new();

    for (from_id, to_id) in &adjacent_pairs {
        if let Some((trans_id, _, _)) = existing_transitions
            .iter()
            .find(|(_, f, t)| f == from_id && t == to_id)
        {
            // Still spanning the same pair, so there is nothing to restate: its settings are the
            // author's and its order follows `from_stage` wherever that stage has moved to.
            kept_trans_ids.insert(trans_id.clone());
        } else {
            let new_trans_id = format!("transition-{}", new_id());
            tx.execute(
                "INSERT INTO behaviour_transition (id, from_stage, to_stage, duration_seconds, easing)
                 VALUES (?1, ?2, ?3, 0.0, 'linear')",
                params![new_trans_id, from_id, to_id],
            )?;
            kept_trans_ids.insert(new_trans_id);
        }
    }

    // Delete any transitions that are no longer adjacent
    for (trans_id, _, _) in existing_transitions {
        if !kept_trans_ids.contains(&trans_id) {
            tx.execute(
                "DELETE FROM behaviour_transition WHERE id = ?",
                params![trans_id],
            )?;
        }
    }

    Ok(())
}

/// The timeline as the editor sees it, switched off or not. See [`TimelineView`].
pub fn timeline(conn: &Connection) -> Result<Option<TimelineView>> {
    Ok(
        read_timeline(conn)?.map(|(timeline, label, enabled)| TimelineView {
            timeline,
            label,
            enabled,
        }),
    )
}

/// Switches the pack's timeline on or off, keeping its stages either way.
///
/// Creates the section if the pack has never had one, so switching on from nothing is the same
/// call as switching back on. A toggle landing on the value already stored writes it again and
/// still costs no undo entry: the upsert leaves the row as it found it, and an empty changeset is
/// what "nothing changed" means here.
pub fn set_experience_enabled(tx: &Transaction<'_>, enabled: bool) -> Result<()> {
    tx.execute(
        "INSERT INTO behaviour_experience (singleton, label, enabled) VALUES (1, NULL, ?)
         ON CONFLICT(singleton) DO UPDATE SET enabled = excluded.enabled",
        params![enabled],
    )?;
    Ok(())
}

/// Sets the timeline's mode-name override, or clears it with `None`.
///
/// Does nothing if the pack has no timeline section: a name for a timeline that does not exist is
/// a stale editor writing into something that has since gone, not a reason to create one.
pub fn set_experience_label(tx: &Transaction<'_>, label: Option<&str>) -> Result<()> {
    tx.execute(
        "UPDATE behaviour_experience SET label = ? WHERE singleton = 1",
        params![label],
    )?;
    Ok(())
}

fn ensure_timeline_exists(tx: &Transaction<'_>) -> Result<()> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM behaviour_experience WHERE singleton = 1)",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        bail!("this pack has no timeline");
    }
    Ok(())
}

/// The stages, in order, whether or not the timeline is switched on.
pub fn stages(conn: &Connection) -> Result<Vec<Stage>> {
    read_stages(conn)
}

/// The transitions, in order.
pub fn transitions(conn: &Connection) -> Result<Vec<Transition>> {
    read_transitions(conn)
}

/// Inserts a stage after `after` (or at the end if `after` is `None`), returning its id.
///
/// `source` is the stage the new one copies its settings from — the author's expectation when
/// adding a stage next to one they have configured. The copy never inherits tag *ownership*: two
/// stages claiming one tag would mean renaming either rewrote a name the other reads.
pub fn add_stage(
    tx: &Transaction<'_>,
    after: Option<&str>,
    source: Option<&str>,
    label: &str,
) -> Result<String> {
    ensure_timeline_exists(tx)?;

    let at_position: i64 = match after {
        Some(after_id) => {
            let pos: Option<i64> = tx
                .query_row(
                    "SELECT position FROM behaviour_stage WHERE id = ?",
                    params![after_id],
                    |row| row.get(0),
                )
                .optional()?;
            match pos {
                Some(p) => p + 1,
                None => tx.query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_stage",
                    [],
                    |row| row.get(0),
                )?,
            }
        }
        None => tx.query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_stage",
            [],
            |row| row.get(0),
        )?,
    };

    tx.execute(
        "UPDATE behaviour_stage SET position = position + 1 WHERE position >= ?",
        params![at_position],
    )?;

    let new_stage_id = format!("stage-{}", uuid());

    if let Some(source_id) = source {
        let source_stage = read_one_stage(tx, source_id)?;
        if let Some(mut stage) = source_stage {
            stage.id = new_stage_id.clone();
            stage.label = label.to_string();
            stage.content.owned_tag = None;
            stage.content.owned_exclude_tag = None;
            write_one_stage(tx, at_position, &stage)?;
        } else {
            tx.execute(
                "INSERT INTO behaviour_stage (id, position, label, restricts_content)
                 VALUES (?1, ?2, ?3, 0)",
                params![new_stage_id, at_position, label],
            )?;
        }
    } else {
        tx.execute(
            "INSERT INTO behaviour_stage (id, position, label, restricts_content)
             VALUES (?1, ?2, ?3, 0)",
            params![new_stage_id, at_position, label],
        )?;
    }

    normalize_timeline(tx, uuid)?;
    Ok(new_stage_id)
}

/// Copies `id` in beside itself, returning the copy's id.
pub fn duplicate_stage(tx: &Transaction<'_>, id: &str) -> Result<String> {
    let source_label: String = tx
        .query_row(
            "SELECT label FROM behaviour_stage WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("that stage is no longer in the timeline"))?;
    let label = format!("{source_label} copy");
    add_stage(tx, Some(id), Some(id), &label)
}

/// Moves `id` to index `to`, clamped to the timeline's bounds.
pub fn move_stage(tx: &Transaction<'_>, id: &str, to: usize) -> Result<()> {
    ensure_timeline_exists(tx)?;
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM behaviour_stage", [], |row| row.get(0))?;
    if count == 0 {
        bail!("that stage is no longer in the timeline");
    }
    let from_pos: Option<i64> = tx
        .query_row(
            "SELECT position FROM behaviour_stage WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(from_pos) = from_pos else {
        bail!("that stage is no longer in the timeline");
    };

    let max_pos = count - 1;
    let target_pos = (to as i64).min(max_pos);
    if from_pos == target_pos {
        return Ok(());
    }

    if from_pos < target_pos {
        tx.execute(
            "UPDATE behaviour_stage SET position = -1 WHERE id = ?",
            params![id],
        )?;
        tx.execute(
            "UPDATE behaviour_stage SET position = position - 1 
             WHERE position > ?1 AND position <= ?2",
            params![from_pos, target_pos],
        )?;
        tx.execute(
            "UPDATE behaviour_stage SET position = ?1 WHERE id = ?2",
            params![target_pos, id],
        )?;
    } else {
        tx.execute(
            "UPDATE behaviour_stage SET position = -1 WHERE id = ?",
            params![id],
        )?;
        tx.execute(
            "UPDATE behaviour_stage SET position = position + 1 
             WHERE position >= ?1 AND position < ?2",
            params![target_pos, from_pos],
        )?;
        tx.execute(
            "UPDATE behaviour_stage SET position = ?1 WHERE id = ?2",
            params![target_pos, id],
        )?;
    }

    normalize_timeline(tx, uuid)?;
    Ok(())
}

/// Removes `id` from the timeline.
///
/// Refuses to remove the last remaining stage: a timeline with no stages is not a state the editor
/// can show or the mode can play. Retiring the stage's media and its owned tag is the caller's
/// business — both need the pack's media tables, which this module deliberately does not touch.
pub fn remove_stage(tx: &Transaction<'_>, id: &str) -> Result<()> {
    ensure_timeline_exists(tx)?;
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM behaviour_stage", [], |row| row.get(0))?;
    if count <= 1 {
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM behaviour_stage WHERE id = ?)",
            params![id],
            |row| row.get(0),
        )?;
        if exists {
            bail!("a timeline needs at least one stage");
        } else {
            return Ok(());
        }
    }

    let pos: Option<i64> = tx
        .query_row(
            "SELECT position FROM behaviour_stage WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(pos) = pos else {
        return Ok(());
    };

    tx.execute("DELETE FROM behaviour_stage WHERE id = ?", params![id])?;
    tx.execute(
        "UPDATE behaviour_stage SET position = position - 1 WHERE position > ?",
        params![pos],
    )?;

    normalize_timeline(tx, uuid)?;
    Ok(())
}

pub fn transition_categories(conn: &Connection, id: &str) -> Result<Vec<TransitionCategory>> {
    let mut statement = conn.prepare(
        "SELECT category FROM behaviour_transition_category WHERE transition_id = ?
         ORDER BY category",
    )?;

    Ok(statement
        .query_map(params![id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}
