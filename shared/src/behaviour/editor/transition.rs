use anyhow::{Result, bail};
use rusqlite::{Transaction, params};

use super::common::{exists, set_column};
use super::timeline::transition_categories;
use crate::behaviour::schema::{Easing, TransitionCategory};

// ── Transitions ──────────────────────────────────────────────────────────────

const NO_TRANSITION: &str = "that transition is no longer in the timeline";

pub fn set_transition_duration(tx: &Transaction<'_>, id: &str, seconds: f64) -> Result<()> {
    set_column(
        tx,
        "behaviour_transition",
        "id",
        &id,
        "duration_seconds",
        seconds,
        NO_TRANSITION,
    )
}

pub fn set_transition_easing(tx: &Transaction<'_>, id: &str, easing: Easing) -> Result<()> {
    set_column(
        tx,
        "behaviour_transition",
        "id",
        &id,
        "easing",
        easing,
        NO_TRANSITION,
    )
}

/// Replaces the whole set of values that interpolate gradually.
///
/// Not reached from the editor, which toggles one category at a time — this is what
/// [`set_transition_category`] writes the result with once it has expanded any legacy group.
pub fn set_transition_affected(
    tx: &Transaction<'_>,
    id: &str,
    affected: &[TransitionCategory],
) -> Result<()> {
    if !exists(tx, "behaviour_transition", "id", &id)? {
        bail!("{NO_TRANSITION}");
    }
    tx.execute(
        "DELETE FROM behaviour_transition_category WHERE transition_id = ?",
        params![id],
    )?;
    for category in affected {
        tx.execute(
            "INSERT INTO behaviour_transition_category (transition_id, category)
             VALUES (?1, ?2)",
            params![id, category],
        )?;
    }
    Ok(())
}

/// Turns one transition category on or off, expanding a legacy broad category first.
///
/// Early v3 editor builds stored one entry for a whole group ("events"), where the editor now names
/// each value. Ticking any member of such a group has to replace the broad entry with its siblings,
/// or switching one off would silently switch the rest off too. That expansion lives here rather
/// than in the editor so the whole toggle is one command — two of them built from the last fetched
/// list would each drop the other's work.
pub fn set_transition_category(
    tx: &Transaction<'_>,
    id: &str,
    category: TransitionCategory,
    enabled: bool,
) -> Result<()> {
    let mut affected = transition_categories(tx, id)?;
    if let Some(group) = legacy_group_of(category)
        && let Some(at) = affected.iter().position(|item| *item == group.0)
    {
        affected.remove(at);
        for member in group.1 {
            if !affected.contains(member) {
                affected.push(*member);
            }
        }
    }
    let present = affected.contains(&category);
    if enabled && !present {
        affected.push(category);
    } else if !enabled && present {
        affected.retain(|item| *item != category);
    }
    set_transition_affected(tx, id, &affected)
}

/// The broad category a value used to be covered by, and everything that covers now.
fn legacy_group_of(
    category: TransitionCategory,
) -> Option<(TransitionCategory, &'static [TransitionCategory])> {
    const INTERVALS: &[TransitionCategory] = &[
        TransitionCategory::PopupInterval,
        TransitionCategory::WebInterval,
        TransitionCategory::NotificationInterval,
        TransitionCategory::PromptInterval,
        TransitionCategory::SoundInterval,
    ];
    const MOVEMENT: &[TransitionCategory] = &[
        TransitionCategory::MovementMinimumSpeed,
        TransitionCategory::MovementMaximumSpeed,
    ];
    const MITOSIS: &[TransitionCategory] = &[
        TransitionCategory::MitosisChance,
        TransitionCategory::MitosisCount,
    ];
    match category {
        TransitionCategory::PopupInterval
        | TransitionCategory::WebInterval
        | TransitionCategory::NotificationInterval
        | TransitionCategory::PromptInterval
        | TransitionCategory::SoundInterval => Some((TransitionCategory::Events, INTERVALS)),
        TransitionCategory::MovementMinimumSpeed | TransitionCategory::MovementMaximumSpeed => {
            Some((TransitionCategory::Movement, MOVEMENT))
        }
        TransitionCategory::MitosisChance | TransitionCategory::MitosisCount => {
            Some((TransitionCategory::Mitosis, MITOSIS))
        }
        _ => None,
    }
}
