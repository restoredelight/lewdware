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
//!
//! [`LastStop`] is what keeps that inference honest: "the supervisor was not running" has two
//! quite different causes, and only one of them says anything at all about the user.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use shared::schedule::PresenceProfile;

use crate::residuals::Accumulator;
use uuid::Uuid;

/// Why the previous run ended, which is the only thing that says what the gap since its
/// `last_tick` means.
///
/// The distinction the profile lives or dies by: a supervisor that bowed out on its own leaves a
/// gap the user was very probably sitting through, while a machine that took the supervisor down
/// with it leaves a gap nobody could have been at the desk for. Learning the first as absence
/// teaches the profile that nobody is ever there -- and a schedule that thinks nobody is ever
/// there stops firing.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LastStop {
    /// The process ended without the chance to say so: a crash, a `SIGKILL`, a power cut, or a
    /// platform whose logout we cannot hook. Read as absence -- the same reading the gap got
    /// before this field existed, and the right one for the common causes, which all involve the
    /// machine going away.
    #[default]
    Unrecorded,
    /// The supervisor stopped itself while the machine kept running: the idle self-terminate
    /// after a session ends, or scheduling being switched off. The user was very likely still
    /// there, so the gap is not evidence of anything and is learned as nothing at all.
    Supervisor,
    /// The session or the machine went away under us -- a logout, a shutdown, a reboot. Learned
    /// as absence, and unlike [`Unrecorded`](LastStop::Unrecorded) the interval leading up to it
    /// is recorded too, so the last hour before a shutdown is not swallowed by the gap after it.
    System,
}

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
    /// When the engine last ticked, or when the previous run recorded its stop. The gap between
    /// this and startup is time the supervisor was not running; `last_stop` says what that means.
    #[serde(default)]
    pub last_tick: Option<DateTime<Local>>,
    /// How the previous run ended. Written at a stop we saw coming and cleared on the first tick
    /// of the next run, so a run that dies without warning leaves the default behind.
    #[serde(default)]
    pub last_stop: LastStop,
    #[serde(default)]
    pub profile: PresenceProfile,
    /// Part-accumulated calibration intervals, one per rate rule. Only ever written when
    /// diagnostics are on, and `#[serde(default)]` so a state file from a run without them loads
    /// unchanged. Kept across restarts because the supervisor restarts often enough that dropping
    /// a part-accumulated interval each time would bias `N - Q` upward on its own.
    #[serde(default)]
    pub accumulators: HashMap<Uuid, Accumulator>,
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
