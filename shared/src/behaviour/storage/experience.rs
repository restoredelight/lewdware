use super::*;

// ── The experience section ───────────────────────────────────────────────────

/// The timeline as the *editor* sees it: present even while switched off.
///
/// [`read`] deliberately cannot express this -- a suspended timeline reads as no timeline, which is
/// what keeps suspension invisible to the engine. The editor needs the other view, because the
/// stages it must still render are exactly the ones `Experience: None` is hiding.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineView {
    /// The stages and transitions, whether or not the timeline is switched on.
    pub timeline: Timeline,
    /// The mode-name override, if the author set one.
    pub label: Option<String>,
    /// Whether the pack plays this timeline. `false` is a timeline the author switched off.
    pub enabled: bool,
}

/// The pack's timeline, or `None` if it has never had one.
pub fn read_timeline(conn: &Connection) -> Result<Option<TimelineView>> {
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
    Ok(Some(TimelineView {
        timeline: Timeline {
            stages: read_stages(conn)?,
            transitions: read_transitions(conn)?,
        },
        label,
        enabled,
    }))
}

/// Switches the pack's timeline on or off, keeping its stages either way.
///
/// Creates the section if the pack has never had one, so switching on from nothing is the same
/// call as switching back on. Returns whether anything changed, which is what keeps a toggle
/// landing on its current value from costing an undo entry.
pub fn set_experience_enabled(tx: &Transaction<'_>, enabled: bool) -> Result<bool> {
    let current: Option<bool> = tx
        .query_row(
            "SELECT enabled FROM behaviour_experience WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if current == Some(enabled) {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO behaviour_experience (singleton, label, enabled) VALUES (1, NULL, ?)
         ON CONFLICT(singleton) DO UPDATE SET enabled = excluded.enabled",
        params![enabled],
    )?;
    Ok(true)
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

pub(in crate::behaviour) fn read_experience(conn: &Connection) -> Result<Option<Experience>> {
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
    // `read_timeline`, which is how the editor gets at them.
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

pub(in crate::behaviour) fn write_experience(
    tx: &Transaction<'_>,
    experience: Option<&Experience>,
) -> Result<()> {
    let Some(experience) = experience else {
        // The pack has no timeline at all -- what a converted pack without one produces. This is
        // not suspension: switching a timeline off keeps its rows and clears `enabled` instead
        // (`set_experience_enabled`), because absence cannot hold anything to switch back on.
        // The media the stages pointed at is deliberately *not* retired either way.
        tx.execute("DELETE FROM behaviour_experience", [])?;
        tx.execute("DELETE FROM behaviour_stage", [])?;
        tx.execute("DELETE FROM behaviour_transition", [])?;
        return Ok(());
    };
    // `enabled` is set rather than left alone: writing a document that *has* a timeline is the
    // whole-document path's way of saying the pack has one, and a stale suspension flag would
    // hide it.
    tx.execute(
        "INSERT INTO behaviour_experience (singleton, label, enabled) VALUES (1, ?, 1)
         ON CONFLICT(singleton) DO UPDATE SET label = excluded.label, enabled = 1",
        params![experience.label],
    )?;
    // Stages first: a transition's endpoints are foreign keys, so a transition into a stage this
    // write is adding can only be inserted once that stage exists. Removing a stage cascades to
    // the transitions touching it, which is why they are reconciled afterwards rather than before.
    write_stages(tx, &experience.timeline.stages)?;
    write_transitions(tx, &experience.timeline.transitions)?;
    Ok(())
}
