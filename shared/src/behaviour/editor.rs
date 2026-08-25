//! Fine-grained editing of a stored behaviour document.
//!
//! The storage module reads and writes complete documents for consumers such as the engine and
//! converter. The editor instead addresses rows by stable primary key and changes only the field or
//! collection item named by an author action. Setters report whether they changed anything so the
//! caller can avoid recording empty undo entries.
//!
//! The implementation is divided by editor domain. Public items are re-exported here to preserve
//! the original `behaviour::editor::*` API.

use std::collections::HashSet;

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::schema::{
    AudioMedia, ContentGroup, ContentSelection, Easing, EndStrategy, EventCountCondition,
    EventKind, EventSchedule, MediaSlot, Mitosis, MonitorPreference, Movement, PopupMedia,
    SpawnRegion, Stage, StageEnd, TextItem, Timeline, Transition, TransitionCategory, WebLink,
};
use super::storage::{
    TimelineView, read_one_stage, read_stages, read_tags, read_transitions, to_text,
    write_one_stage, write_owned_stage_tag, write_stage_end, write_stage_entry, write_stage_events,
    write_stage_mitosis, write_stage_movement, write_stage_prompt, write_stage_tags, write_tags,
};

mod common;
mod group;
mod link;
mod media;
mod membership;
mod pool;
mod stage;
mod stage_tags;
mod timeline;
mod transition;

use common::*;
pub use group::*;
pub use link::*;
#[cfg(test)]
use media::edit_popup_entries;
pub use media::*;
#[cfg(test)]
use membership::selects;
pub use membership::*;
pub use pool::*;
pub use stage::*;
#[cfg(test)]
use stage_tags::MAX_SLUG;
pub use stage_tags::*;
#[cfg(test)]
use timeline::DEFAULT_STAGE_SECONDS;
use timeline::transition_categories;
pub use timeline::*;
pub use transition::*;

// Switching the timeline on and off is represented by storage's `enabled` column, but remains an
// editor action and therefore stays available through this facade.
pub use super::storage::{set_experience_enabled, set_experience_label};

#[cfg(test)]
mod tests;
