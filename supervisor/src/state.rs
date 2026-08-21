//! The schedule engine's state, kept across restarts.
//!
//! Everything here used to live only in memory, which meant a logout quietly undid it: budgets
//! reset to a full allowance, so "three times a day" became three *per login*; and a panic's
//! two-hour hold ended the moment the user logged back in, which is the opposite of what panic
//! promises.
//!
//! `last_tick` is here for a subtler reason. It is the only way to notice that time passed while
//! the supervisor was *not running* -- and that gap is the presence signal: a machine that was off
//! is a machine nobody was sitting at. Without persisting it there is nothing to learn from.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use shared::schedule::PresenceProfile;
use uuid::Uuid;

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub struct PersistedBudget {
    pub period_key: NaiveDate,
    pub remaining: u32,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct PersistedState {
    /// Keyed by rule id, so budgets survive an unrelated edit to the rule list. A stale entry
    /// expires on its own: a period key that no longer matches is reset rather than trusted.
    #[serde(default)]
    pub budgets: HashMap<Uuid, PersistedBudget>,
    /// A pause still running. Restored so a panic outlives a logout.
    #[serde(default)]
    pub cooldown_until: Option<DateTime<Local>>,
    /// When the engine last ticked. The gap between this and startup is time the supervisor was
    /// not running.
    #[serde(default)]
    pub last_tick: Option<DateTime<Local>>,
    #[serde(default)]
    pub profile: PresenceProfile,
}

/// Beside the wallpaper snapshot, not in the config directory: this is state the supervisor owns,
/// not settings the user edits. Plain JSON and safe to delete -- doing so costs a budget period
/// and the learned profile, nothing more.
pub fn state_path() -> Option<PathBuf> {
    let dir = dirs::data_local_dir()?.join("lewdware");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("schedule-state.json"))
}

/// Missing, unreadable or corrupt all mean the same thing: start fresh. Losing this file is a
/// setback, never a failure -- the defaults are exactly what a first run uses.
pub fn load(path: Option<&PathBuf>) -> PersistedState {
    let Some(path) = path else {
        return PersistedState::default();
    };
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|err| {
            tracing::warn!("ignoring unreadable schedule state at {path:?}: {err}");
            PersistedState::default()
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => PersistedState::default(),
        Err(err) => {
            tracing::warn!("could not read schedule state at {path:?}: {err}");
            PersistedState::default()
        }
    }
}

/// Written via a temporary file and renamed, so a crash mid-write cannot leave a half-written file
/// that the next start would discard.
pub fn save(path: Option<&PathBuf>, state: &PersistedState) {
    let Some(path) = path else { return };
    let Ok(json) = serde_json::to_string_pretty(state) else {
        return;
    };
    let temp = path.with_extension("json.tmp");
    if let Err(err) = std::fs::write(&temp, json) {
        tracing::warn!("could not write schedule state: {err}");
        return;
    }
    if let Err(err) = std::fs::rename(&temp, path) {
        tracing::warn!("could not replace schedule state: {err}");
        let _ = std::fs::remove_file(&temp);
    }
}
