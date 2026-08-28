use anyhow::Result;
use rusqlite::{Connection, Transaction, params};

use super::common::{retain, sql_array};
use crate::behaviour::schema::{Easing, Transition, TransitionCategory};

pub fn read_transitions(conn: &Connection) -> Result<Vec<Transition>> {
    // Ordered by where `from_stage` sits in the timeline, which is the only thing a transition's
    // place in it ever meant. The join is an inner one deliberately: a transition whose endpoint
    // is gone is not a transition, and the foreign key already cascades it away.
    let mut statement = conn.prepare(
        "SELECT t.id, t.from_stage, t.to_stage, t.duration_seconds, t.easing
         FROM behaviour_transition t
         JOIN behaviour_stage s ON s.id = t.from_stage
         ORDER BY s.position",
    )?;
    let rows: Vec<(String, String, String, f64, Easing)> = statement
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
         WHERE transition_id = ? ORDER BY category",
    )?;
    rows.into_iter()
        .map(|(id, from_stage, to_stage, duration_seconds, easing)| {
            let affected: Vec<TransitionCategory> = categories
                .query_map(params![&id], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(Transition {
                id,
                from_stage,
                to_stage,
                duration_seconds,
                easing,
                affected,
            })
        })
        .collect()
}

pub fn write_transitions(tx: &Transaction<'_>, transitions: &[Transition]) -> Result<()> {
    retain(
        tx,
        "behaviour_transition",
        "id",
        transitions.iter().map(|t| t.id.as_str()),
    )?;
    for transition in transitions {
        tx.execute(
            "INSERT INTO behaviour_transition
                 (id, from_stage, to_stage, duration_seconds, easing)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET from_stage = excluded.from_stage,
                                           to_stage = excluded.to_stage,
                                           duration_seconds = excluded.duration_seconds,
                                           easing = excluded.easing",
            params![
                transition.id,
                transition.from_stage,
                transition.to_stage,
                transition.duration_seconds,
                transition.easing,
            ],
        )?;
        tx.execute(
            "DELETE FROM behaviour_transition_category WHERE transition_id = ?1
             AND category NOT IN (SELECT value FROM rarray(?2))",
            params![transition.id, sql_array(transition.affected.iter())?,],
        )?;
        for category in &transition.affected {
            tx.execute(
                "INSERT INTO behaviour_transition_category (transition_id, category)
                 VALUES (?, ?)
                 ON CONFLICT(transition_id, category) DO NOTHING",
                params![transition.id, category],
            )?;
        }
    }
    Ok(())
}
