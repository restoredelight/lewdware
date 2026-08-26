use anyhow::Result;
use rusqlite::{Connection, Transaction, params};

use super::common::{from_text, retain, text_array, to_text};
use crate::behaviour::schema::Transition;

pub fn read_transitions(conn: &Connection) -> Result<Vec<Transition>> {
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

pub fn write_transitions(
    tx: &Transaction<'_>,
    transitions: &[Transition],
) -> Result<()> {
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
             AND category NOT IN (SELECT value FROM rarray(?2))",
            params![
                transition.id,
                text_array(categories.iter().map(String::as_str))
            ],
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
