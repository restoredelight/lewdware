use anyhow::{Result, bail};
use rusqlite::{Connection, Transaction, params};

use crate::behaviour::schema::{
    ContentSelection, EndStrategy, Stage, StageEnd, Timeline, Transition, TransitionCategory,
};
use crate::behaviour::storage::{
    TimelineView, from_text, read_stages, read_timeline, read_transitions, retain, write_one_stage,
    write_transitions,
};

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
//
// This used to live in the editor's front end (`timelineModel.ts`) because that is where the edit
// was constructed. It belongs here: the front end never needs it to *render* a timeline, only to
// build one, and building one is now the backend's job.

/// The default an end condition takes when a stage stops being last and needs one.
pub const DEFAULT_STAGE_SECONDS: f64 = 300.0;

/// Restores the two timeline invariants, in place.
///
/// `new_id` mints ids for transitions that have to be created, so tests can make them predictable.
fn normalize(timeline: &mut Timeline, mut new_id: impl FnMut() -> String) {
    let last = timeline.stages.len().saturating_sub(1);
    for (index, stage) in timeline.stages.iter_mut().enumerate() {
        if index == last {
            stage.end = None;
        } else if stage.end.is_none() {
            stage.end = Some(StageEnd {
                duration_seconds: Some(DEFAULT_STAGE_SECONDS),
                event_count: None,
                strategy: EndStrategy::Any,
            });
        }
    }

    let existing = std::mem::take(&mut timeline.transitions);
    timeline.transitions = timeline
        .stages
        .windows(2)
        .map(|pair| {
            let (from, to) = (&pair[0], &pair[1]);
            existing
                .iter()
                .find(|candidate| candidate.from_stage == from.id && candidate.to_stage == to.id)
                .cloned()
                .unwrap_or_else(|| Transition {
                    id: format!("transition-{}", new_id()),
                    from_stage: from.id.clone(),
                    to_stage: to.id.clone(),
                    duration_seconds: 0.0,
                    easing: Default::default(),
                    affected: Vec::new(),
                })
        })
        .collect();
}

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The timeline as the editor sees it, switched off or not. See [`TimelineView`].
pub fn timeline(conn: &Connection) -> Result<Option<TimelineView>> {
    read_timeline(conn)
}

/// Writes `timeline` back over the stored one, touching only the rows that differ.
///
/// The structural edits below all share this tail: they read the timeline, rearrange the stage
/// list, normalize, and write. Only the rows that actually changed are touched, so reordering two
/// stages records a changeset naming two stages rather than the whole timeline.
fn write_timeline(tx: &Transaction<'_>, timeline: &Timeline) -> Result<()> {
    retain(
        tx,
        "behaviour_stage",
        "id",
        timeline.stages.iter().map(|stage| stage.id.as_str()),
    )?;
    for (position, stage) in timeline.stages.iter().enumerate() {
        write_one_stage(tx, position as i64, stage)?;
    }
    write_transitions(tx, &timeline.transitions)?;
    Ok(())
}

/// Reads the timeline for a structural edit, refusing if the pack has none.
fn timeline_for_edit(tx: &Transaction<'_>) -> Result<Timeline> {
    let Some(view) = read_timeline(tx)? else {
        bail!("this pack has no timeline");
    };
    Ok(view.timeline)
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
    let mut timeline = timeline_for_edit(tx)?;
    let template = source
        .and_then(|id| timeline.stages.iter().find(|stage| stage.id == id))
        .cloned();
    let mut stage = template.unwrap_or_else(|| Stage {
        id: String::new(),
        label: String::new(),
        end: None,
        content: ContentSelection::default(),
        events: Default::default(),
        movement: None,
        mitosis: None,
        on_enter: Default::default(),
        prompt: Default::default(),
    });
    stage.id = format!("stage-{}", uuid());
    stage.label = label.to_string();
    stage.content.owned_tag = None;

    let id = stage.id.clone();
    let at = match after {
        Some(after) => timeline
            .stages
            .iter()
            .position(|stage| stage.id == after)
            .map(|index| index + 1)
            .unwrap_or(timeline.stages.len()),
        None => timeline.stages.len(),
    };
    timeline.stages.insert(at, stage);
    normalize(&mut timeline, uuid);
    write_timeline(tx, &timeline)?;
    Ok(id)
}

/// Copies `id` in beside itself, returning the copy's id.
pub fn duplicate_stage(tx: &Transaction<'_>, id: &str) -> Result<String> {
    let label = {
        let timeline = timeline_for_edit(tx)?;
        let Some(source) = timeline.stages.iter().find(|stage| stage.id == id) else {
            bail!("that stage is no longer in the timeline");
        };
        format!("{} copy", source.label)
    };
    add_stage(tx, Some(id), Some(id), &label)
}

/// Moves `id` to index `to`, clamped to the timeline's bounds.
pub fn move_stage(tx: &Transaction<'_>, id: &str, to: usize) -> Result<bool> {
    let mut timeline = timeline_for_edit(tx)?;
    let Some(from) = timeline.stages.iter().position(|stage| stage.id == id) else {
        bail!("that stage is no longer in the timeline");
    };
    let to = to.min(timeline.stages.len().saturating_sub(1));
    if from == to {
        return Ok(false);
    }
    let stage = timeline.stages.remove(from);
    timeline.stages.insert(to, stage);
    normalize(&mut timeline, uuid);
    write_timeline(tx, &timeline)?;
    Ok(true)
}

/// Removes `id` from the timeline.
///
/// Refuses to remove the last remaining stage: a timeline with no stages is not a state the editor
/// can show or the mode can play. Retiring the stage's media and its owned tag is the caller's
/// business — both need the pack's media tables, which this module deliberately does not touch.
pub fn remove_stage(tx: &Transaction<'_>, id: &str) -> Result<bool> {
    let mut timeline = timeline_for_edit(tx)?;
    if timeline.stages.len() <= 1 {
        bail!("a timeline needs at least one stage");
    }
    let Some(index) = timeline.stages.iter().position(|stage| stage.id == id) else {
        return Ok(false);
    };
    timeline.stages.remove(index);
    normalize(&mut timeline, uuid);
    write_timeline(tx, &timeline)?;
    Ok(true)
}

pub fn transition_categories(
    conn: &Connection,
    id: &str,
) -> Result<Vec<TransitionCategory>> {
    let mut statement = conn.prepare(
        "SELECT category FROM behaviour_transition_category WHERE transition_id = ?
         ORDER BY position",
    )?;
    let rows: Vec<String> = statement
        .query_map(params![id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    rows.iter()
        .map(|value| from_text(value))
        .collect()
}
